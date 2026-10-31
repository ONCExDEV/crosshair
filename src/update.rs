//! Updates Crosshair from its GitHub releases.
//!
//! Nothing here runs by itself. The settings window asks GitHub once when it
//! opens whether a newer release exists (nothing is downloaded), and only
//! downloads and installs when the user presses the Update button. The overlay
//! never uses the network.
//!
//! The new exe replaces the old one in the same place: a running exe can be
//! renamed but not overwritten, so the old one becomes `name.exe.old` (removed
//! on the next start). If anything fails the old exe is put back. Profiles are
//! never touched.

use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

use crate::win::{self, wide};

pub const REPOSITORY: &str = "ONCExDEV/crosshair";
pub const ASSET: &str = "crosshair.exe";

/// A newer release on GitHub.
#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

pub type Version = (u32, u32, u32);

pub fn parse(v: &str) -> Option<Version> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

pub fn current() -> Version {
    parse(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

pub fn show(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

// ------------------------------------------------------------------- network

fn is_github(host: &str) -> bool {
    host == "github.com" || host.ends_with(".github.com") || host.ends_with(".githubusercontent.com")
}

fn split_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://")?;
    Some(match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.to_string(), "/".to_string()),
    })
}

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// GET over HTTPS. Redirects are followed by hand so every hop can be checked
/// to still be on GitHub. The body is passed to `sink`, which returns false to
/// abort. Returns the HTTP status.
fn get(url: &str, sink: &mut dyn FnMut(&[u8]) -> bool) -> Result<u16, String> {
    unsafe {
        let agent = wide("Crosshair-updater");
        let session = Handle(WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err("Could not start a network session.".into());
        }
        let _ = WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 60_000);

        let mut current = url.to_string();
        for _ in 0..6 {
            let (host, path) = split_url(&current).ok_or("Unexpected download address.")?;
            if !is_github(&host) {
                return Err("The address is not on GitHub, so it was not used.".into());
            }
            let whost = wide(&host);
            let wpath = wide(&path);
            let conn = Handle(WinHttpConnect(session.0, PCWSTR(whost.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0));
            if conn.0.is_null() {
                return Err("Could not connect to GitHub.".into());
            }
            let req = Handle(WinHttpOpenRequest(conn.0, w!("GET"), PCWSTR(wpath.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null_mut(), WINHTTP_FLAG_SECURE));
            if req.0.is_null() {
                return Err("Could not start the request.".into());
            }
            let never = WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes();
            let _ = WinHttpSetOption(Some(req.0), WINHTTP_OPTION_REDIRECT_POLICY, Some(&never));
            if host == "api.github.com" {
                let hdr: Vec<u16> = "Accept: application/vnd.github+json\r\n".encode_utf16().collect();
                let _ = WinHttpAddRequestHeaders(req.0, &hdr, WINHTTP_ADDREQ_FLAG_ADD);
            }
            WinHttpSendRequest(req.0, None, None, 0, 0, 0).map_err(|_| "Could not reach GitHub. Check your internet connection.".to_string())?;
            WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|_| "GitHub did not answer.".to_string())?;

            let mut status: u32 = 0;
            let mut len = std::mem::size_of::<u32>() as u32;
            WinHttpQueryHeaders(req.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut c_void), &mut len, std::ptr::null_mut())
                .map_err(|_| "Unreadable answer from GitHub.".to_string())?;

            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let mut buf = vec![0u16; 2048];
                let mut blen = (buf.len() * 2) as u32;
                WinHttpQueryHeaders(req.0, WINHTTP_QUERY_LOCATION, PCWSTR::null(), Some(buf.as_mut_ptr() as *mut c_void), &mut blen, std::ptr::null_mut())
                    .map_err(|_| "GitHub sent a redirect without an address.".to_string())?;
                current = String::from_utf16_lossy(&buf[..(blen as usize / 2)]);
                continue;
            }

            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                let mut read = 0u32;
                WinHttpReadData(req.0, chunk.as_mut_ptr() as *mut c_void, chunk.len() as u32, &mut read).map_err(|_| "The download was interrupted.".to_string())?;
                if read == 0 {
                    break;
                }
                if !sink(&chunk[..read as usize]) {
                    return Err("Could not write the download to disk.".into());
                }
            }
            return Ok(status as u16);
        }
        Err("Too many redirects.".into())
    }
}

// -------------------------------------------------------------------- update

/// Returns the newest release if it is newer than this version, otherwise None.
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let mut body = Vec::new();
    let status = get(&format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"), &mut |b| {
        body.extend_from_slice(b);
        true
    })?;
    if status == 404 {
        return Ok(None);
    }
    if status != 200 {
        return Err(format!("GitHub answered with status {status}."));
    }
    let json: serde_json::Value = serde_json::from_slice(&body).map_err(|_| "Unexpected answer from GitHub.".to_string())?;
    let tag = json["tag_name"].as_str().ok_or("Unexpected answer from GitHub.")?;
    let latest = parse(tag).ok_or_else(|| format!("Unexpected version \"{tag}\" on GitHub."))?;
    if latest <= current() {
        return Ok(None);
    }
    for asset in json["assets"].as_array().into_iter().flatten() {
        if asset["name"].as_str() != Some(ASSET) {
            continue;
        }
        let url = asset["browser_download_url"].as_str().ok_or("The release has no download address.")?.to_string();
        let sha256 = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_string);
        return Ok(Some(UpdateInfo { version: show(latest), url, size: asset["size"].as_u64().unwrap_or(0), sha256 }));
    }
    Err(format!("Release {tag} has no {ASSET} to download."))
}

/// Downloads the release exe to `target` and checks it (size, exe header and
/// the SHA-256 checksum when GitHub provides one). Removes the file on failure.
pub fn download(info: &UpdateInfo, target: &Path, progress: &AtomicU64) -> Result<(), String> {
    let result = download_inner(info, target, progress);
    if result.is_err() {
        let _ = std::fs::remove_file(target);
    }
    result
}

fn download_inner(info: &UpdateInfo, target: &Path, progress: &AtomicU64) -> Result<(), String> {
    let mut file = File::create(target).map_err(|e| format!("Could not create the download file: {e}."))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let status = get(&info.url, &mut |b| {
        if file.write_all(b).is_err() {
            return false;
        }
        hasher.update(b);
        total += b.len() as u64;
        progress.store(total, Ordering::Relaxed);
        true
    })?;
    drop(file);
    if status != 200 {
        return Err(format!("The download failed (status {status})."));
    }
    if info.size > 0 && total != info.size {
        return Err("The download is incomplete.".into());
    }
    let mut head = [0u8; 2];
    File::open(target).and_then(|mut f| f.read_exact(&mut head)).map_err(|_| "The download could not be read.".to_string())?;
    if &head != b"MZ" {
        return Err("The download is not a program file.".into());
    }
    if let Some(expected) = &info.sha256 {
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("The download does not match GitHub's checksum.".into());
        }
    }
    Ok(())
}

/// Puts the new exe where the running one is. The old one is kept as
/// `name.exe.old` and put back if anything fails.
pub fn replace(new_file: &Path, target_exe: &Path) -> Result<(), String> {
    let old = old_path(target_exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target_exe, &old).map_err(|e| format!("Could not replace the exe ({e}). The folder may need administrator rights."))?;
    if let Err(e) = std::fs::rename(new_file, target_exe) {
        let _ = std::fs::rename(&old, target_exe);
        return Err(format!("Could not install the update ({e})."));
    }
    Ok(())
}

fn old_path(exe: &Path) -> std::path::PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    s.into()
}

fn download_path(exe: &Path) -> std::path::PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".download");
    s.into()
}

pub fn download_target(exe: &Path) -> std::path::PathBuf {
    download_path(exe)
}

/// Removes leftovers of an earlier update.
pub fn clean_up(exe: &Path) {
    let _ = std::fs::remove_file(old_path(exe));
    let _ = std::fs::remove_file(download_path(exe));
}

/// Stops the running overlay, starts the new exe and reopens the settings.
/// Does not return.
pub fn restart(exe: &Path) -> ! {
    if win::notify(win::MSG_QUIT) {
        for _ in 0..40 {
            if win::ctl_hwnd().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    let _ = std::process::Command::new(exe).arg("--background").spawn();
    let _ = std::process::Command::new(exe).args(["--settings", "--delay", "1500"]).spawn();
    std::process::exit(0);
}
