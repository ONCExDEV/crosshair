//! Background process: transparent click-through overlay, tray icon, hotkeys.
//! Event driven. It sleeps in GetMessage and uses no CPU while idle.


use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use xh::config::{self, Config};
use xh::render::{self, Frame};
use xh::win::{self, wide};

const WM_TRAY: u32 = WM_APP + 20;

const HK_TOGGLE: i32 = 1;
const HK_NEXT: i32 = 2;
const HK_DRAG: i32 = 3;
const HK_SETTINGS: i32 = 4;
const HK_ESC: i32 = 10;
const HK_ENTER: i32 = 11;
const HK_NUDGE: i32 = 12; // 12..=15 = 1px L,R,U,D; 16..=19 = 10px

const ID_TOGGLE: u32 = 1;
const ID_DRAG: u32 = 2;
const ID_SETTINGS: u32 = 3;
const ID_AUTOSTART: u32 = 4;
const ID_EXIT: u32 = 5;
const ID_PROFILE: u32 = 100;

static DRAG: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

struct App {
    cfg: Config,
    drag: bool,
    /// Profile matched by the foreground game, if any.
    game: Option<usize>,
    /// Profile picked by hotkey/tray; cleared on the next foreground change.
    manual: Option<usize>,
    overlay: HWND,
    ctl: HWND,
    hook: HWINEVENTHOOK,
    icon: HICON,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

impl App {
    fn displayed(&self) -> usize {
        let auto = if self.cfg.auto_switch { self.game } else { None };
        self.manual.or(auto).unwrap_or(self.cfg.active).min(self.cfg.profiles.len() - 1)
    }

    fn visible(&self) -> bool {
        self.drag || (self.cfg.enabled && (!self.cfg.hide_outside_games || self.game.is_some()))
    }

    /// Show or hide the crosshair and remember the choice for the next start.
    fn toggle_enabled(&mut self) {
        let was = self.cfg.enabled;
        self.sync_from_disk();
        self.cfg.enabled = !was;
        self.save();
        self.refresh();
    }

    fn refresh(&mut self) {
        unsafe {
            if !self.visible() {
                let _ = ShowWindow(self.overlay, SW_HIDE);
                return;
            }
            let p = &self.cfg.profiles[self.displayed()];
            let frame = render::render(p, self.drag);
            let mons = win::monitors();
            let m = mons.get(p.monitor as usize).unwrap_or(&mons[0]);
            let x = m.cx() + p.offset_x - frame.w as i32 / 2;
            let y = m.cy() + p.offset_y - frame.h as i32 / 2;
            present(self.overlay, &frame, x, y);
            let _ = ShowWindow(self.overlay, SW_SHOWNA);
            self.raise();
        }
    }

    fn raise(&self) {
        unsafe {
            let _ = SetWindowPos(self.overlay, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    fn save(&self) {
        let _ = self.cfg.save();
    }

    /// Take the latest file from disk before saving a small change, so a
    /// profile edit made in the settings window can never be overwritten
    /// by a stale copy kept in memory here.
    fn sync_from_disk(&mut self) {
        self.cfg = Config::load();
    }

    fn pause_hotkeys(&self) {
        unsafe {
            for id in HK_TOGGLE..=HK_SETTINGS {
                let _ = UnregisterHotKey(self.ctl, id);
            }
        }
    }

    fn reload(&mut self) {
        self.cfg = Config::load();
        self.manual = None;
        self.register_hotkeys();
        self.sync_hook();
        if !self.drag {
            self.refresh();
        }
    }

    fn register_hotkeys(&self) {
        unsafe {
            for id in HK_TOGGLE..=HK_SETTINGS {
                let _ = UnregisterHotKey(self.ctl, id);
            }
            let hk = &self.cfg.hotkeys;
            for (id, h) in [(HK_TOGGLE, &hk.toggle), (HK_NEXT, &hk.next), (HK_DRAG, &hk.drag), (HK_SETTINGS, &hk.settings)] {
                if let Some(vk) = config::vk(&h.key) {
                    let mut m = MOD_NOREPEAT;
                    if h.ctrl {
                        m |= MOD_CONTROL;
                    }
                    if h.alt {
                        m |= MOD_ALT;
                    }
                    if h.shift {
                        m |= MOD_SHIFT;
                    }
                    let _ = RegisterHotKey(self.ctl, id, m, vk);
                }
            }
        }
    }

    /// The foreground hook is only installed when a feature needs it.
    fn sync_hook(&mut self) {
        let want = self.cfg.keep_on_top
            || self.cfg.hide_outside_games
            || (self.cfg.auto_switch && self.cfg.has_games());
        unsafe {
            if want && self.hook.is_invalid() {
                self.hook = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(on_foreground),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                self.update_game(GetForegroundWindow());
            } else if !want && !self.hook.is_invalid() {
                let _ = UnhookWinEvent(self.hook);
                self.hook = HWINEVENTHOOK::default();
                self.game = None;
            }
        }
    }

    fn update_game(&mut self, hwnd: HWND) {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let needs_name = self.cfg.hide_outside_games || (self.cfg.auto_switch && self.cfg.has_games());
        self.game = if needs_name && pid != 0 {
            win::exe_name(pid).and_then(|n| self.cfg.game_profile(&n))
        } else {
            None
        };
        self.manual = None;
    }

    fn set_tip(&self, text: &str) {
        unsafe {
            let n = nid(self.ctl, self.icon, text);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
        }
    }

    fn enter_drag(&mut self) {
        if self.drag {
            return;
        }
        self.drag = true;
        DRAG.store(true, Ordering::Relaxed);
        unsafe {
            let ex = GetWindowLongPtrW(self.overlay, GWL_EXSTYLE);
            SetWindowLongPtrW(self.overlay, GWL_EXSTYLE, ex & !(WS_EX_TRANSPARENT.0 as isize));
            let ctl = self.ctl;
            let _ = RegisterHotKey(ctl, HK_ESC, HOT_KEY_MODIFIERS(0), VK_ESCAPE.0 as u32);
            let _ = RegisterHotKey(ctl, HK_ENTER, HOT_KEY_MODIFIERS(0), VK_RETURN.0 as u32);
            for (i, vk) in [VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN].iter().enumerate() {
                let _ = RegisterHotKey(ctl, HK_NUDGE + i as i32, HOT_KEY_MODIFIERS(0), vk.0 as u32);
                let _ = RegisterHotKey(ctl, HK_NUDGE + 4 + i as i32, MOD_SHIFT, vk.0 as u32);
            }
        }
        self.set_tip("Crosshair: drag or use arrows (Shift = 10px), Enter to save, Esc to cancel");
        self.refresh();
    }

    fn exit_drag(&mut self, save: bool) {
        if !self.drag {
            return;
        }
        unsafe {
            if save {
                self.sync_from_disk();
                let mut r = RECT::default();
                if GetWindowRect(self.overlay, &mut r).is_ok() {
                    let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
                    let mons = win::monitors();
                    let idx = mons
                        .iter()
                        .position(|m| cx >= m.left && cx < m.right && cy >= m.top && cy < m.bottom)
                        .unwrap_or(0);
                    let m = &mons[idx];
                    let d = self.displayed();
                    let p = &mut self.cfg.profiles[d];
                    p.monitor = idx as u32;
                    p.offset_x = cx - m.cx();
                    p.offset_y = cy - m.cy();
                    self.save();
                }
            }
            let ex = GetWindowLongPtrW(self.overlay, GWL_EXSTYLE);
            SetWindowLongPtrW(self.overlay, GWL_EXSTYLE, ex | WS_EX_TRANSPARENT.0 as isize);
            for id in HK_ESC..=HK_NUDGE + 7 {
                let _ = UnregisterHotKey(self.ctl, id);
            }
        }
        self.drag = false;
        DRAG.store(false, Ordering::Relaxed);
        self.set_tip("Crosshair");
        self.refresh();
    }

    fn nudge(&self, idx: i32) {
        let step = if idx >= 4 { 10 } else { 1 };
        let (dx, dy) = match idx % 4 {
            0 => (-step, 0),
            1 => (step, 0),
            2 => (0, -step),
            _ => (0, step),
        };
        unsafe {
            let mut r = RECT::default();
            if GetWindowRect(self.overlay, &mut r).is_ok() {
                let _ = SetWindowPos(
                    self.overlay,
                    None,
                    r.left + dx,
                    r.top + dy,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    fn select_profile(&mut self, i: usize) {
        self.sync_from_disk();
        if i < self.cfg.profiles.len() {
            self.cfg.active = i;
            self.manual = Some(i);
            self.save();
            self.refresh();
        }
    }
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    with_app(|a| {
        a.update_game(hwnd);
        a.refresh();
    });
}

unsafe fn present(hwnd: HWND, f: &Frame, x: i32, y: i32) {
    let screen = GetDC(None);
    let mem = CreateCompatibleDC(screen);
    let bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: f.w as i32,
            biHeight: -(f.h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    if let Ok(bmp) = CreateDIBSection(mem, &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
        if !bits.is_null() {
            let data = f.to_bgra();
            std::ptr::copy_nonoverlapping(data.as_ptr(), bits as *mut u8, data.len());
            let old = SelectObject(mem, bmp);
            let size = SIZE { cx: f.w as i32, cy: f.h as i32 };
            let src = POINT { x: 0, y: 0 };
            let dst = POINT { x, y };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(hwnd, screen, Some(&dst), Some(&size), mem, Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(mem, old);
        }
        let _ = DeleteObject(bmp);
    }
    let _ = DeleteDC(mem);
    ReleaseDC(None, screen);
}

unsafe fn make_icon() -> HICON {
    const S: u32 = 32;
    let data = render::tray_icon(S);
    let bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: S as i32,
            biHeight: -(S as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let color = CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
    if !bits.is_null() {
        std::ptr::copy_nonoverlapping(data.as_ptr(), bits as *mut u8, data.len());
    }
    let mask = CreateBitmap(S as i32, S as i32, 1, 1, None);
    let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&info).unwrap_or_default();
    let _ = DeleteObject(color);
    let _ = DeleteObject(mask);
    icon
}

fn nid(hwnd: HWND, icon: HICON, tip: &str) -> NOTIFYICONDATAW {
    let mut n = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    for (i, c) in tip.encode_utf16().take(n.szTip.len() - 1).enumerate() {
        n.szTip[i] = c;
    }
    n
}

fn open_settings() {
    unsafe {
        if let Ok(h) = FindWindowW(None, win::SETTINGS_TITLE) {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
            return;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--settings").spawn();
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let Some((names, shown, enabled)) =
        with_app(|a| (a.cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), a.displayed(), a.cfg.enabled))
    else {
        return;
    };
    let menu = match CreatePopupMenu() {
        Ok(m) => m,
        Err(_) => return,
    };
    let add = |id: u32, text: &str, checked: bool| {
        let t = wide(text);
        let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(t.as_ptr()));
    };
    add(ID_TOGGLE, "Show crosshair", enabled);
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (i, n) in names.iter().enumerate() {
        add(ID_PROFILE + i as u32, n, i == shown);
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    add(ID_DRAG, "Move crosshair", false);
    add(ID_SETTINGS, "Settings", false);
    add(ID_AUTOSTART, "Start with Windows", win::autostart_enabled());
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    add(ID_EXIT, "Exit", false);

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None).0 as u32;
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);

    match cmd {
        0 => {}
        ID_TOGGLE => {
            with_app(|a| a.toggle_enabled());
        }
        ID_DRAG => {
            with_app(|a| if a.drag { a.exit_drag(true) } else { a.enter_drag() });
        }
        ID_SETTINGS => open_settings(),
        ID_AUTOSTART => {
            if let Ok(exe) = std::env::current_exe() {
                win::set_autostart(!win::autostart_enabled(), &exe);
            }
        }
        ID_EXIT => {
            let _ = DestroyWindow(hwnd);
        }
        id if id >= ID_PROFILE => {
            with_app(|a| a.select_profile((id - ID_PROFILE) as usize));
        }
        _ => {}
    }
}

fn on_hotkey(id: i32) {
    with_app(|a| match id {
        HK_TOGGLE => a.toggle_enabled(),
        HK_NEXT => {
            let n = a.cfg.profiles.len();
            let next = (a.displayed() + 1) % n;
            a.select_profile(next);
        }
        HK_DRAG => {
            if a.drag {
                a.exit_drag(true)
            } else {
                a.enter_drag()
            }
        }
        HK_SETTINGS => open_settings(),
        HK_ESC => a.exit_drag(false),
        HK_ENTER => a.exit_drag(true),
        i if (HK_NUDGE..HK_NUDGE + 8).contains(&i) => a.nudge(i - HK_NUDGE),
        _ => {}
    });
}

unsafe extern "system" fn overlay_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        WM_NCHITTEST => LRESULT(if DRAG.load(Ordering::Relaxed) { HTCAPTION as isize } else { HTTRANSPARENT as isize }),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(h, m, w, l),
    }
}

unsafe extern "system" fn ctl_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        WM_HOTKEY => {
            on_hotkey(w.0 as i32);
            LRESULT(0)
        }
        WM_TRAY => {
            match (l.0 as u32) & 0xFFFF {
                WM_RBUTTONUP => show_menu(h),
                WM_LBUTTONUP => {
                    with_app(|a| a.toggle_enabled());
                }
                WM_LBUTTONDBLCLK => open_settings(),
                _ => {}
            }
            LRESULT(0)
        }
        win::MSG_RELOAD => {
            with_app(|a| a.reload());
            LRESULT(0)
        }
        win::MSG_DRAG => {
            with_app(|a| if a.drag { a.exit_drag(true) } else { a.enter_drag() });
            LRESULT(0)
        }
        win::MSG_SETTINGS => {
            open_settings();
            LRESULT(0)
        }
        win::MSG_QUIT => {
            let _ = DestroyWindow(h);
            LRESULT(0)
        }
        win::MSG_PAUSE_KEYS => {
            with_app(|a| a.pause_hotkeys());
            LRESULT(0)
        }
        win::MSG_RESUME_KEYS => {
            with_app(|a| a.register_hotkeys());
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ if m == TASKBAR_CREATED.load(Ordering::Relaxed) && m != 0 => {
            with_app(|a| {
                let n = nid(a.ctl, a.icon, "Crosshair");
                let _ = Shell_NotifyIconW(NIM_ADD, &n);
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}

pub fn run() {
    unsafe {
        if !win::single_instance(w!("Local\\XhCrosshairDaemon")) {
            win::notify(win::MSG_SETTINGS);
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if let Ok(exe) = std::env::current_exe() {
            xh::update::clean_up(&exe);
        }

        let hinst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let overlay_class = w!("XhOverlay");
        let wc = WNDCLASSW { lpfnWndProc: Some(overlay_proc), hInstance: hinst, lpszClassName: overlay_class, ..Default::default() };
        RegisterClassW(&wc);
        let wc = WNDCLASSW { lpfnWndProc: Some(ctl_proc), hInstance: hinst, lpszClassName: win::CTL_CLASS, ..Default::default() };
        RegisterClassW(&wc);

        let Ok(ctl) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            win::CTL_CLASS,
            w!("Crosshair"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            hinst,
            None,
        ) else {
            return;
        };
        let Ok(overlay) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            overlay_class,
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            hinst,
            None,
        ) else {
            return;
        };

        if !config::path().exists() {
            let _ = Config::default().save();
        }
        let icon = make_icon();
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                cfg: Config::load(),
                drag: false,
                game: None,
                manual: None,
                overlay,
                ctl,
                hook: HWINEVENTHOOK::default(),
                icon,
            })
        });

        let n = nid(ctl, icon, "Crosshair");
        let _ = Shell_NotifyIconW(NIM_ADD, &n);

        with_app(|a| {
            a.register_hotkeys();
            a.sync_hook();
            a.refresh();
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let n = nid(ctl, icon, "");
        let _ = Shell_NotifyIconW(NIM_DELETE, &n);
    }
}
