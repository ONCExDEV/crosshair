//! Small Windows helpers shared by the daemon and the settings app.
//! Nothing in here touches another process: no OpenProcess, no injection,
//! no memory access, no screen capture.

use std::path::Path;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Messages understood by the daemon's control window.
pub const MSG_RELOAD: u32 = WM_APP + 1;
pub const MSG_DRAG: u32 = WM_APP + 2;
pub const MSG_SETTINGS: u32 = WM_APP + 3;
/// Release / re-register the global hotkeys while the settings window records a new one.
pub const MSG_PAUSE_KEYS: u32 = WM_APP + 4;
pub const MSG_RESUME_KEYS: u32 = WM_APP + 5;
/// Ask the overlay to exit (used right before an update replaces the exe).
pub const MSG_QUIT: u32 = WM_APP + 6;

pub const CTL_CLASS: PCWSTR = w!("XhCtl");
pub const SETTINGS_TITLE: PCWSTR = w!("Crosshair Settings");

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub struct MonitorInfo {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub primary: bool,
}

impl MonitorInfo {
    pub fn w(&self) -> i32 {
        self.right - self.left
    }
    pub fn h(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn cx(&self) -> i32 {
        self.left + self.w() / 2
    }
    pub fn cy(&self) -> i32 {
        self.top + self.h() / 2
    }
}

/// Monitors with the primary one first.
pub fn monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn cb(hm: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        let v = &mut *(lp.0 as *mut Vec<MonitorInfo>);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(hm, &mut mi).as_bool() {
            let r = mi.rcMonitor;
            v.push(MonitorInfo {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
                primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        TRUE
    }
    let mut v: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v.sort_by_key(|m| !m.primary);
    if v.is_empty() {
        v.push(MonitorInfo { left: 0, top: 0, right: 1920, bottom: 1080, primary: true });
    }
    v
}

/// pid -> exe name from a process snapshot. This reads the system process
/// list only; it never opens a handle to any other process.
pub fn process_list() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let n = &e.szExeFile;
                let len = n.iter().position(|&c| c == 0).unwrap_or(n.len());
                out.push((e.th32ProcessID, String::from_utf16_lossy(&n[..len])));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub fn exe_name(pid: u32) -> Option<String> {
    process_list().into_iter().find(|(p, _)| *p == pid).map(|(_, n)| n)
}

/// Executables that currently own a visible, titled window.
pub fn running_apps() -> Vec<String> {
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let pids = &mut *(lp.0 as *mut Vec<u32>);
        if IsWindowVisible(hwnd).as_bool()
            && GetWindowTextLengthW(hwnd) > 0
            && GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 == 0
        {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            pids.push(pid);
        }
        TRUE
    }
    let mut pids: Vec<u32> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut pids as *mut _ as isize));
    }
    let procs = process_list();
    let mut names: Vec<String> = procs
        .into_iter()
        .filter(|(p, _)| pids.contains(p))
        .map(|(_, n)| n)
        .filter(|n| {
            let l = n.to_ascii_lowercase();
            !l.starts_with("crosshair") && l != "explorer.exe" && l != "textinputhost.exe" && l != "applicationframehost.exe"
        })
        .collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

/// The daemon's hidden control window. FindWindow does not return it, so
/// walk the top-level windows and compare class names.
pub fn ctl_hwnd() -> Option<HWND> {
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let mut buf = [0u16; 16];
        let n = GetClassNameW(hwnd, &mut buf);
        if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "XhCtl" {
            *(lp.0 as *mut Option<HWND>) = Some(hwnd);
            return FALSE;
        }
        TRUE
    }
    let mut found: Option<HWND> = None;
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// Post a message to the running daemon. Returns false if it is not running.
pub fn notify(msg: u32) -> bool {
    match ctl_hwnd() {
        Some(h) => unsafe { PostMessageW(h, msg, WPARAM(0), LPARAM(0)).is_ok() },
        None => false,
    }
}

/// True if this is the first instance with that name.
pub fn single_instance(name: PCWSTR) -> bool {
    unsafe {
        match CreateMutexW(None, false, name) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => true,
        }
    }
}

pub fn focus_settings() {
    unsafe {
        if let Ok(h) = FindWindowW(None, SETTINGS_TITLE) {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
    }
}

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Crosshair");

pub fn autostart_enabled() -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_READ, &mut key).is_err() {
            return false;
        }
        let r = RegQueryValueExW(key, RUN_VALUE, None, None, None, None);
        let _ = RegCloseKey(key);
        r.is_ok()
    }
}

pub fn set_autostart(on: bool, exe: &Path) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_SET_VALUE, &mut key).is_err() {
            return;
        }
        if on {
            // --background keeps the start with Windows quiet: tray only.
            let val = wide(&format!("\"{}\" --background", exe.display()));
            let bytes = std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2);
            let _ = RegSetValueExW(key, RUN_VALUE, 0, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(key, RUN_VALUE);
        }
        let _ = RegCloseKey(key);
    }
}
