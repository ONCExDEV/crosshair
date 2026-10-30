//! Settings window. Runs only while open, so its RAM is released on close.
//! Changes are saved to config.toml and the daemon is pinged to reload.


use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, vec2, Align, Align2, Color32, FontFamily, FontId, Layout, RichText, Stroke, TextStyle, Ui};
use windows::core::w;
use xh::config::{self, Config, Hotkey, Profile};
use xh::update::{self, UpdateInfo};
use xh::{render, win};

const ACCENT: Color32 = Color32::from_rgb(91, 91, 240);
const BG: Color32 = Color32::from_rgb(17, 17, 20);
const SIDE: Color32 = Color32::from_rgb(21, 21, 25);
const CARD: Color32 = Color32::from_rgb(24, 24, 29);
const INPUT: Color32 = Color32::from_rgb(14, 14, 17);
const BORDER: Color32 = Color32::from_rgb(42, 42, 52);
const TEXT: Color32 = Color32::from_rgb(236, 236, 241);
const MUTED: Color32 = Color32::from_rgb(138, 138, 154);
const SELECTED: Color32 = Color32::from_rgb(38, 38, 48);

const BGS: [(&str, Color32); 4] = [
    ("Dark", Color32::from_rgb(20, 22, 26)),
    ("Light", Color32::from_rgb(205, 210, 216)),
    ("Green", Color32::from_rgb(45, 90, 61)),
    ("Sky", Color32::from_rgb(91, 134, 176)),
];

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Shape,
    Color,
    Position,
    Games,
    Share,
}

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Profile,
    General,
}

/// State of the update banner. Nothing is downloaded before the user presses Update.
enum Upd {
    Idle,
    Available(UpdateInfo),
    Downloading(u64),
    Failed(String),
}

enum UpdMsg {
    Checked(Result<Option<UpdateInfo>, String>),
    Installed(Result<(), String>),
}

struct App {
    cfg: Config,
    sel: usize,
    page: Page,
    tab: Tab,
    dirty_since: Option<Instant>,
    last_mtime: Option<SystemTime>,
    last_poll: Instant,
    daemon_running: bool,
    autostart: bool,

    logo: egui::TextureHandle,
    tex: Option<egui::TextureHandle>,
    tex_key: String,
    bg: usize,
    zoom: bool,

    running: Vec<String>,
    new_game: String,
    import_text: String,
    status: String,
    mons: Vec<win::MonitorInfo>,
    /// Title of the hotkey row that is waiting for a key press.
    listening: Option<String>,

    upd: Upd,
    upd_rx: Option<mpsc::Receiver<UpdMsg>>,
    upd_progress: Arc<AtomicU64>,
}

fn c32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

fn from32(c: Color32) -> [u8; 4] {
    c.to_srgba_unmultiplied()
}

fn daemon_exe() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| "crosshair.exe".into())
}

fn arg(name: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == name {
            return it.next();
        }
    }
    None
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let logo = tiny_skia::Pixmap::decode_png(include_bytes!("../docs/logo.png")).expect("logo");
        let img = egui::ColorImage::from_rgba_premultiplied([logo.width() as usize, logo.height() as usize], logo.data());
        let mut app = Self {
            cfg: Config::load(),
            sel: 0,
            page: Page::Profile,
            tab: Tab::Shape,
            dirty_since: None,
            last_mtime: Config::mtime(),
            last_poll: Instant::now(),
            daemon_running: win::ctl_hwnd().is_some(),
            autostart: win::autostart_enabled(),
            logo: ctx.load_texture("logo", img, egui::TextureOptions::LINEAR),
            tex: None,
            tex_key: String::new(),
            bg: 0,
            zoom: false,
            running: win::running_apps(),
            new_game: String::new(),
            import_text: String::new(),
            status: String::new(),
            mons: win::monitors(),
            listening: None,
            upd: Upd::Idle,
            upd_rx: None,
            upd_progress: Arc::new(AtomicU64::new(0)),
        };
        // One quiet check when the window opens. Nothing is downloaded.
        app.spawn_check(ctx);
        // Optional flags, handy for screenshots: --page general --tab color --sel 2 --bg 2
        if arg("--page").as_deref() == Some("general") {
            app.page = Page::General;
        }
        app.tab = match arg("--tab").as_deref() {
            Some("color") => Tab::Color,
            Some("position") => Tab::Position,
            Some("games") => Tab::Games,
            Some("share") => Tab::Share,
            _ => Tab::Shape,
        };
        if let Some(n) = arg("--sel").and_then(|s| s.parse::<usize>().ok()) {
            app.sel = n.min(app.cfg.profiles.len() - 1);
        }
        app.zoom = arg("--zoom").is_some();
        if let Some(n) = arg("--bg").and_then(|s| s.parse::<usize>().ok()) {
            app.bg = n.min(BGS.len() - 1);
        }
        app
    }

    fn spawn_check(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        self.upd_rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(UpdMsg::Checked(update::check()));
            ctx.request_repaint();
        });
    }

    /// Download, verify and install. Only runs after the user pressed Update.
    fn start_update(&mut self, info: UpdateInfo, ctx: &egui::Context) {
        self.upd = Upd::Downloading(info.size);
        self.upd_progress.store(0, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.upd_rx = Some(rx);
        let progress = self.upd_progress.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = match std::env::current_exe() {
                Ok(exe) => {
                    let file = update::download_target(&exe);
                    let r = update::download(&info, &file, &progress).and_then(|_| update::replace(&file, &exe));
                    if r.is_err() {
                        update::clean_up(&exe);
                    }
                    r
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(UpdMsg::Installed(result));
            ctx.request_repaint();
        });
    }

    fn poll_update(&mut self) {
        let Some(rx) = &self.upd_rx else { return };
        let msgs: Vec<UpdMsg> = rx.try_iter().collect();
        for m in msgs {
            match m {
                // Offline or no release: stay quiet, the banner just stays hidden.
                UpdMsg::Checked(Ok(Some(info))) => self.upd = Upd::Available(info),
                UpdMsg::Checked(_) => {}
                UpdMsg::Installed(Ok(())) => {
                    if self.dirty_since.is_some() {
                        self.commit();
                    }
                    if let Ok(exe) = std::env::current_exe() {
                        update::restart(&exe);
                    }
                }
                UpdMsg::Installed(Err(e)) => self.upd = Upd::Failed(e),
            }
        }
    }

    fn update_banner(&mut self, ui: &mut Ui) {
        let mut start: Option<UpdateInfo> = None;
        let mut retry = false;
        match &self.upd {
            Upd::Idle => return,
            Upd::Available(info) => {
                let text = format!("Version {} is available (you have {}). Nothing is downloaded until you press Update.", info.version, update::show(update::current()));
                info_banner(ui, &text, false, |ui| {
                    if primary(ui, "Update") {
                        start = Some(info.clone());
                    }
                });
            }
            Upd::Downloading(size) => {
                let done = self.upd_progress.load(Ordering::Relaxed);
                let pct = if *size > 0 { (done * 100 / size).min(100) } else { 0 };
                info_banner(ui, &format!("Downloading the update... {pct}%"), false, |_| {});
                ui.ctx().request_repaint_after(Duration::from_millis(150));
            }
            Upd::Failed(msg) => {
                info_banner(ui, &format!("Update failed: {msg} Nothing was changed."), true, |ui| {
                    if ui.button("Try again").clicked() {
                        retry = true;
                    }
                });
            }
        }
        ui.add_space(8.0);
        if let Some(info) = start {
            self.start_update(info, ui.ctx());
        }
        if retry {
            self.upd = Upd::Idle;
            self.spawn_check(ui.ctx());
        }
    }

    fn poll_external(&mut self) {
        if self.last_poll.elapsed() < Duration::from_millis(700) {
            return;
        }
        self.last_poll = Instant::now();
        self.daemon_running = win::ctl_hwnd().is_some();
        if self.dirty_since.is_none() {
            let m = Config::mtime();
            if m != self.last_mtime {
                self.last_mtime = m;
                self.cfg = Config::load();
                self.sel = self.sel.min(self.cfg.profiles.len() - 1);
            }
        }
    }

    /// Write the config to disk and tell the overlay to reload it.
    fn commit(&mut self) {
        self.cfg.sanitize();
        if self.cfg.save().is_ok() {
            self.last_mtime = Config::mtime();
        }
        win::notify(win::MSG_RELOAD);
        self.dirty_since = None;
    }

    fn flush(&mut self, changed: bool, ctx: &egui::Context) {
        if changed {
            self.dirty_since = Some(Instant::now());
        }
        if let Some(t) = self.dirty_since {
            if t.elapsed() >= Duration::from_millis(120) {
                self.commit();
            } else {
                ctx.request_repaint_after(Duration::from_millis(130));
            }
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let p = &self.cfg.profiles[self.sel];
        let key = toml::to_string(p).unwrap_or_default();
        if key == self.tex_key && self.tex.is_some() {
            return;
        }
        let f = render::render(p, false);
        let img = egui::ColorImage::from_rgba_premultiplied([f.w as usize, f.h as usize], &f.rgba);
        match &mut self.tex {
            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
            None => self.tex = Some(ctx.load_texture("preview", img, egui::TextureOptions::NEAREST)),
        }
        self.tex_key = key;
    }

    // ---------------------------------------------------------------- sidebar

    fn nav_row(ui: &mut Ui, text: &str, selected: bool, dot: bool) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), egui::Sense::click());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if selected {
            ui.painter().rect_filled(rect, 8.0, SELECTED);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 8.0, Color32::from_rgb(28, 28, 34));
        }
        let color = if selected { TEXT } else { Color32::from_rgb(190, 190, 204) };
        ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(15.0), color);
        if dot {
            ui.painter().circle_filled(rect.right_center() - vec2(14.0, 0.0), 4.0, ACCENT);
        }
        resp
    }

    fn sidebar(&mut self, ui: &mut Ui, changed: &mut bool) {
        ui.horizontal(|ui| {
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(vec2(28.0, 28.0)));
            ui.label(RichText::new("Crosshair").size(18.0).strong().color(TEXT));
        });
        caption(ui, "Profiles");

        let n = self.cfg.profiles.len();
        for i in 0..n {
            let name = self.cfg.profiles[i].name.clone();
            let selected = self.page == Page::Profile && self.sel == i;
            let r = Self::nav_row(ui, &name, selected, self.cfg.active == i);
            if r.clicked() {
                self.sel = i;
                self.page = Page::Profile;
            }
        }

        ui.add_space(6.0);
        ui.menu_button(RichText::new("+  New profile").color(TEXT), |ui| {
            if ui.button("Blank").clicked() {
                self.cfg.profiles.push(Profile::default());
                self.sel = self.cfg.profiles.len() - 1;
                self.page = Page::Profile;
                *changed = true;
                ui.close_menu();
            }
            ui.separator();
            for p in config::presets() {
                if ui.button(&p.name).clicked() {
                    self.cfg.profiles.push(p);
                    self.sel = self.cfg.profiles.len() - 1;
                    self.page = Page::Profile;
                    *changed = true;
                    ui.close_menu();
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Duplicate").clicked() {
                let mut p = self.cfg.profiles[self.sel].clone();
                p.name = format!("{} copy", p.name);
                p.games.clear();
                self.cfg.profiles.push(p);
                self.sel = self.cfg.profiles.len() - 1;
                self.page = Page::Profile;
                *changed = true;
            }
            if ui.add_enabled(n > 1, egui::Button::new("Delete")).clicked() {
                self.cfg.profiles.remove(self.sel);
                self.sel = self.sel.min(self.cfg.profiles.len() - 1);
                self.cfg.active = self.cfg.active.min(self.cfg.profiles.len() - 1);
                *changed = true;
            }
        });

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).size(12.0).color(MUTED));
            ui.add_space(4.0);
            if Self::nav_row(ui, "Settings", self.page == Page::General, false).clicked() {
                self.page = Page::General;
            }
        });
    }

    // ---------------------------------------------------------------- profile

    fn preview(&mut self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 170.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10.0, BGS[self.bg].1);
        painter.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, BORDER));
        if let Some(t) = &self.tex {
            let s = t.size_vec2() * if self.zoom { 2.0 } else { 1.0 };
            let r = egui::Rect::from_center_size(rect.center(), s);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(t.id(), r, uv, Color32::WHITE);
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let opts: Vec<(usize, &str)> = BGS.iter().enumerate().map(|(i, (n, _))| (i, *n)).collect();
            seg(ui, &mut self.bg, &opts);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Right-to-left layout reverses the visual order.
                seg(ui, &mut self.zoom, &[(true, "2x"), (false, "1x")]);
            });
        });
    }

    fn profile_page(&mut self, ui: &mut Ui, changed: &mut bool) {
        self.update_texture(ui.ctx());
        let is_active = self.cfg.active == self.sel;
        ui.horizontal(|ui| {
            let p = &mut self.cfg.profiles[self.sel];
            let edit = egui::TextEdit::singleline(&mut p.name)
                .font(FontId::proportional(24.0))
                .frame(false)
                .desired_width(320.0)
                .hint_text("Profile name");
            *changed |= ui.add(edit).changed();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if is_active {
                    pill(ui, "In use");
                } else if primary(ui, "Use this profile") {
                    self.cfg.active = self.sel;
                    *changed = true;
                }
            });
        });
        ui.add_space(8.0);
        self.preview(ui);
        ui.add_space(10.0);
        seg(ui, &mut self.tab, &[(Tab::Shape, "Shape"), (Tab::Color, "Color"), (Tab::Position, "Position"), (Tab::Games, "Games"), (Tab::Share, "Share")]);
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().slider_width = 190.0;
            match self.tab {
                Tab::Shape => self.tab_shape(ui, changed),
                Tab::Color => self.tab_color(ui, changed),
                Tab::Position => self.tab_position(ui, changed),
                Tab::Games => self.tab_games(ui, changed),
                Tab::Share => self.tab_share(ui, changed),
            }
            ui.add_space(12.0);
        });
    }

    fn tab_shape(&mut self, ui: &mut Ui, changed: &mut bool) {
        let p = &mut self.cfg.profiles[self.sel];
        ui.add_space(6.0);
        group(ui, "Cross", "Four lines around the center", &mut p.cross, changed, |ui, ch| {
            row(ui, "Length", "", |ui| *ch |= slider(ui, &mut p.cross_len, 0..=40, " px"));
            row(ui, "Thickness", "", |ui| *ch |= slider(ui, &mut p.cross_thick, 1..=12, " px"));
            row(ui, "Gap", "Space around the center", |ui| *ch |= slider(ui, &mut p.cross_gap, 0..=30, " px"));
            row(ui, "Rotation", "45° makes an X", |ui| *ch |= slider(ui, &mut p.rotation, -180.0..=180.0, "°"));
            row(ui, "T-style", "Hide the top line", |ui| *ch |= toggle(ui, &mut p.t_style));
        });
        ui.add_space(10.0);
        group(ui, "Dot", "A point in the center", &mut p.dot, changed, |ui, ch| {
            row(ui, "Size", "", |ui| *ch |= slider(ui, &mut p.dot_size, 1..=20, " px"));
            row(ui, "Round", "Off gives a square dot", |ui| *ch |= toggle(ui, &mut p.dot_round));
        });
        ui.add_space(10.0);
        group(ui, "Circle", "A ring around the center", &mut p.circle, changed, |ui, ch| {
            row(ui, "Radius", "", |ui| *ch |= slider(ui, &mut p.circle_radius, 2..=80, " px"));
            row(ui, "Thickness", "", |ui| *ch |= slider(ui, &mut p.circle_thick, 1..=12, " px"));
        });
        ui.add_space(10.0);
        group(ui, "Square", "A square frame around the center", &mut p.square, changed, |ui, ch| {
            row(ui, "Size", "Side length", |ui| *ch |= slider(ui, &mut p.square_size, 2..=120, " px"));
            row(ui, "Thickness", "", |ui| *ch |= slider(ui, &mut p.square_thick, 1..=12, " px"));
            row(ui, "Filled", "Solid square instead of a frame", |ui| *ch |= toggle(ui, &mut p.square_filled));
        });
        ui.add_space(10.0);
        group(ui, "Custom image", "A PNG that replaces the shapes above", &mut p.use_image, changed, |ui, ch| {
            row(ui, "File", "", |ui| {
                if ui.button("Browse").clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("PNG image", &["png"]).pick_file() {
                        p.image_path = f.display().to_string();
                        *ch = true;
                    }
                }
                *ch |= ui.add(egui::TextEdit::singleline(&mut p.image_path).desired_width(200.0).hint_text("C:\\path\\to\\image.png")).changed();
            });
            row(ui, "Scale", "", |ui| *ch |= slider(ui, &mut p.image_scale, 0.1..=4.0, "x"));
        });
    }

    fn tab_color(&mut self, ui: &mut Ui, changed: &mut bool) {
        let p = &mut self.cfg.profiles[self.sel];
        ui.add_space(6.0);
        card(ui, |ui| {
            row(ui, "Color", "", |ui| {
                let mut c = c32(p.color);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend).changed() {
                    p.color = from32(c);
                    *changed = true;
                }
                for sw in [[255, 0, 200], [255, 255, 255], [255, 214, 0], [0, 229, 255], [255, 64, 64], [0, 255, 102]] {
                    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
                    ui.painter().rect_filled(r, 10.0, Color32::from_rgb(sw[0], sw[1], sw[2]));
                    ui.painter().rect_stroke(r, 10.0, Stroke::new(1.0_f32, BORDER));
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        p.color = [sw[0], sw[1], sw[2], p.color[3]];
                        *changed = true;
                    }
                }
            });
            row(ui, "Opacity", "", |ui| *changed |= slider(ui, &mut p.opacity, 0.05..=1.0, ""));
        });
        ui.add_space(10.0);
        group(ui, "Outline", "A dark edge that keeps the crosshair visible", &mut p.outline, changed, |ui, ch| {
            row(ui, "Thickness", "", |ui| *ch |= slider(ui, &mut p.outline_thick, 1..=6, " px"));
            row(ui, "Color", "", |ui| {
                let mut c = c32(p.outline_color);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend).changed() {
                    p.outline_color = from32(c);
                    *ch = true;
                }
            });
        });
    }

    fn tab_position(&mut self, ui: &mut Ui, changed: &mut bool) {
        let mut pending_drag = false;
        let p = &mut self.cfg.profiles[self.sel];
        ui.add_space(6.0);
        card(ui, |ui| {
            row(ui, "Monitor", "", |ui| {
                let label = |i: usize, m: &win::MonitorInfo| {
                    let name = if m.primary { "Primary".to_string() } else { format!("Monitor {}", i + 1) };
                    format!("{name}  ({}x{})", m.w(), m.h())
                };
                let cur = self.mons.get(p.monitor as usize).map(|m| label(p.monitor as usize, m)).unwrap_or_else(|| "Primary".into());
                egui::ComboBox::from_id_salt("monitor").selected_text(cur).show_ui(ui, |ui| {
                    for (i, m) in self.mons.iter().enumerate() {
                        *changed |= ui.selectable_value(&mut p.monitor, i as u32, label(i, m)).changed();
                    }
                });
            });
            row(ui, "Offset X", "Pixels from the center of the monitor", |ui| {
                *changed |= ui.add(egui::DragValue::new(&mut p.offset_x).speed(1.0).suffix(" px")).changed();
            });
            row(ui, "Offset Y", "", |ui| {
                *changed |= ui.add(egui::DragValue::new(&mut p.offset_y).speed(1.0).suffix(" px")).changed();
            });
            row(ui, "Center", "Reset to the exact middle of the screen", |ui| {
                if ui.button("Center").clicked() {
                    p.offset_x = 0;
                    p.offset_y = 0;
                    *changed = true;
                }
            });
            row(ui, "Place with mouse", "Drag it, or use the arrow keys (Shift = 10 px). Enter saves, Esc cancels.", |ui| {
                if primary(ui, "Move") {
                    pending_drag = true;
                }
            });
        });
        if pending_drag {
            // Make sure the daemon has the latest values before it starts.
            self.dirty_since = Some(Instant::now() - Duration::from_secs(1));
            self.flush(false, ui.ctx());
            win::notify(win::MSG_DRAG);
        }
    }

    fn tab_games(&mut self, ui: &mut Ui, changed: &mut bool) {
        ui.add_space(6.0);
        ui.label(RichText::new("When one of these programs is in the foreground, this profile is used automatically. Only the program name is compared; the game itself is never touched.").color(MUTED).size(13.0));
        ui.add_space(8.0);
        let p = &mut self.cfg.profiles[self.sel];
        let mut remove = None;
        card(ui, |ui| {
            if p.games.is_empty() {
                row(ui, "No programs yet", "Add one below", |_| {});
            }
            for (i, g) in p.games.iter().enumerate() {
                row(ui, g, "", |ui| {
                    if ui.button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            row(ui, "Add program", "For example game.exe", |ui| {
                if ui.button("Add").clicked() && !self.new_game.trim().is_empty() {
                    p.games.push(self.new_game.trim().to_string());
                    self.new_game.clear();
                    *changed = true;
                }
                ui.add(egui::TextEdit::singleline(&mut self.new_game).hint_text("game.exe").desired_width(160.0));
            });
            row(ui, "Running apps", "Pick one that is open right now", |ui| {
                if ui.button("Refresh").clicked() {
                    self.running = win::running_apps();
                }
                egui::ComboBox::from_id_salt("running").selected_text("Choose").show_ui(ui, |ui| {
                    for n in &self.running {
                        if ui.selectable_label(false, n).clicked() && !p.games.iter().any(|g| g.eq_ignore_ascii_case(n)) {
                            p.games.push(n.clone());
                            *changed = true;
                        }
                    }
                });
            });
        });
        if let Some(i) = remove {
            p.games.remove(i);
            *changed = true;
        }
        if !self.cfg.auto_switch {
            ui.add_space(8.0);
            ui.colored_label(Color32::from_rgb(255, 180, 60), "Auto-switch is off in Settings, so this list is not used.");
        }
    }

    fn tab_share(&mut self, ui: &mut Ui, changed: &mut bool) {
        ui.add_space(6.0);
        card(ui, |ui| {
            let code = config::export_code(&self.cfg.profiles[self.sel]);
            row(ui, "Share this profile", "Send the code to a friend", |ui| {
                if primary(ui, "Copy") {
                    ui.ctx().copy_text(code.clone());
                    self.status = "Copied to clipboard".into();
                }
                ui.add(egui::TextEdit::singleline(&mut code.clone()).desired_width(200.0));
            });
            row(ui, "Import a profile", "Paste a code that starts with XH1:", |ui| {
                if ui.button("Import").clicked() {
                    match config::import_code(&self.import_text) {
                        Some(p) => {
                            self.cfg.profiles.push(p);
                            self.sel = self.cfg.profiles.len() - 1;
                            self.import_text.clear();
                            self.status = "Profile imported".into();
                            *changed = true;
                        }
                        None => self.status = "That code is not valid".into(),
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut self.import_text).hint_text("XH1:...").desired_width(200.0));
            });
        });
        if !self.status.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(&self.status).color(ACCENT));
        }
    }

    // ---------------------------------------------------------------- general

    fn general_page(&mut self, ui: &mut Ui, changed: &mut bool) {
        ui.label(RichText::new("Settings").size(24.0).color(TEXT));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            caption(ui, "Behavior");
            card(ui, |ui| {
                row(ui, "Start with Windows", "Launches quietly in the tray", |ui| {
                    if toggle(ui, &mut self.autostart) {
                        win::set_autostart(self.autostart, &daemon_exe());
                    }
                });
                row(ui, "Switch profile per game", "Uses the Games list of each profile", |ui| *changed |= toggle(ui, &mut self.cfg.auto_switch));
                row(ui, "Only show in listed games", "Hide the crosshair everywhere else", |ui| *changed |= toggle(ui, &mut self.cfg.hide_outside_games));
                row(ui, "Keep above game windows", "Re-raises the overlay when the foreground changes", |ui| *changed |= toggle(ui, &mut self.cfg.keep_on_top));
            });

            caption(ui, "Hotkeys");
            let hk = &mut self.cfg.hotkeys;
            let listening = &mut self.listening;
            card(ui, |ui| {
                hotkey_row(ui, "Show / hide", "Turn the crosshair on or off from anywhere", &mut hk.toggle, listening, changed);
                hotkey_row(ui, "Next profile", "Cycle through your profiles", &mut hk.next, listening, changed);
                hotkey_row(ui, "Move crosshair", "Drag it, or use the arrow keys", &mut hk.drag, listening, changed);
                hotkey_row(ui, "Open settings", "Open this window from anywhere", &mut hk.settings, listening, changed);
            });
            if self.listening.is_some() {
                ui.add_space(6.0);
                ui.label(RichText::new("Press the new shortcut now. Esc cancels.").size(12.5).color(MUTED));
            }

            caption(ui, "Safety");
            card(ui, |ui| {
                row(ui, "Only draws a window", "No injection, no game hooks, no memory reading, no screen capture.", |_| {});
                row(ui, "Never touches other processes", "Game detection reads the foreground program name from the system process list.", |_| {});
                row(ui, "Borderless or windowed only", "Exclusive fullscreen is not supported.", |_| {});
                row(ui, "No guarantees", "Some games or platforms forbid overlays. Check the rules of each game.", |_| {});
            });

            caption(ui, "About");
            card(ui, |ui| {
                row(ui, &format!("Crosshair {}", update::show(update::current())), "Checks GitHub once when this window opens. Installs only when you press Update.", |_| {});
                row(ui, "Config folder", "Your profiles are stored here", |ui| {
                    if ui.button("Open").clicked() {
                        let _ = std::process::Command::new("explorer").arg(config::dir()).spawn();
                    }
                });
                row(ui, "Source code", "Free and open source (MIT)", |ui| {
                    ui.hyperlink_to("GitHub", "https://github.com/ONCExDEV/crosshair");
                });
            });
            ui.add_space(12.0);
        });
    }
}

// -------------------------------------------------------------------- widgets

fn caption(ui: &mut Ui, text: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(text.to_uppercase()).size(11.5).strong().color(MUTED));
    ui.add_space(2.0);
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(16.0, 4.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.data_mut(|d| d.insert_temp(egui::Id::new("row_first"), true));
            add(ui);
        });
}

/// A settings row: title and description on the left, controls on the right.
/// Controls are added right to left.
fn row(ui: &mut Ui, title: &str, sub: &str, add: impl FnOnce(&mut Ui)) {
    let id = egui::Id::new("row_first");
    let first = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(true));
    if !first {
        ui.separator();
    }
    ui.data_mut(|d| d.insert_temp(id, false));
    ui.horizontal(|ui| {
        ui.set_min_height(44.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(if sub.is_empty() { 11.0 } else { 3.0 });
            ui.label(RichText::new(title).size(15.0).color(TEXT));
            if !sub.is_empty() {
                ui.label(RichText::new(sub).size(12.5).color(MUTED));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), add);
    });
}

/// A card with a header toggle. The body rows only show while it is on.
fn group(ui: &mut Ui, title: &str, sub: &str, on: &mut bool, ch: &mut bool, body: impl FnOnce(&mut Ui, &mut bool)) {
    card(ui, |ui| {
        row(ui, title, sub, |ui| *ch |= toggle(ui, on));
        if *on {
            body(ui, ch);
        }
    });
}

fn toggle(ui: &mut Ui, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(38.0, 22.0), egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut changed = false;
    if resp.clicked() {
        *on = !*on;
        changed = true;
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let track = Color32::from_rgb(58, 58, 70).lerp_to_gamma(ACCENT, t);
    ui.painter().rect_filled(rect, 11.0, track);
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    changed
}

fn seg<T: PartialEq + Copy>(ui: &mut Ui, cur: &mut T, opts: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::none()
        .fill(INPUT)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(8.0)
        .inner_margin(egui::Margin::same(3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (v, label) in opts {
                    let sel = *cur == *v;
                    let text = RichText::new(*label).color(if sel { TEXT } else { MUTED });
                    let b = egui::Button::new(text)
                        .fill(if sel { Color32::from_rgb(48, 48, 60) } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .rounding(6.0)
                        .min_size(vec2(0.0, 24.0));
                    if ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !sel {
                        *cur = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

fn primary(ui: &mut Ui, text: &str) -> bool {
    let b = egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(ACCENT).stroke(Stroke::NONE).min_size(vec2(0.0, 28.0));
    ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn info_banner(ui: &mut Ui, text: &str, error: bool, add: impl FnOnce(&mut Ui)) {
    let (fill, stroke, color) = if error {
        (Color32::from_rgb(48, 24, 26), Color32::from_rgb(120, 50, 55), Color32::from_rgb(255, 160, 160))
    } else {
        (Color32::from_rgb(26, 28, 56), ACCENT, Color32::from_rgb(190, 192, 255))
    };
    egui::Frame::none().fill(fill).stroke(Stroke::new(1.0_f32, stroke)).rounding(8.0).inner_margin(egui::Margin::symmetric(12.0, 6.0)).show(ui, |ui| {
        // The button goes first (right side) so the text wraps in what is left.
        let size = vec2(ui.available_width(), 40.0);
        ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
            add(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
            });
        });
    });
}

fn pill(ui: &mut Ui, text: &str) {
    egui::Frame::none()
        .fill(Color32::from_rgb(28, 30, 60))
        .stroke(Stroke::new(1.0_f32, ACCENT))
        .rounding(14.0)
        .inner_margin(egui::Margin::symmetric(12.0, 4.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(13.0).strong().color(Color32::from_rgb(160, 160, 255)));
        });
}

fn slider<N: egui::emath::Numeric>(ui: &mut Ui, v: &mut N, range: RangeInclusive<N>, suffix: &str) -> bool {
    // Rows lay out right to left; keep the slider itself left to right so
    // the value box sits after the rail.
    let width = ui.spacing().slider_width + 96.0;
    ui.allocate_ui_with_layout(vec2(width, 28.0), Layout::left_to_right(Align::Center), |ui| {
        ui.add(egui::Slider::new(v, range).suffix(suffix).trailing_fill(true)).changed()
    })
    .inner
}

fn hotkey_text(h: &Hotkey) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if h.ctrl {
        parts.push("Ctrl");
    }
    if h.alt {
        parts.push("Alt");
    }
    if h.shift {
        parts.push("Shift");
    }
    parts.push(&h.key);
    parts.join(" + ")
}

/// Name used in the config file for a pressed key, if we support it.
fn key_name(k: egui::Key) -> Option<String> {
    use egui::Key as K;
    const DIGITS: [(K, &str); 10] =
        [(K::Num0, "0"), (K::Num1, "1"), (K::Num2, "2"), (K::Num3, "3"), (K::Num4, "4"), (K::Num5, "5"), (K::Num6, "6"), (K::Num7, "7"), (K::Num8, "8"), (K::Num9, "9")];
    const NAMED: [(K, &str); 10] = [
        (K::Insert, "Insert"),
        (K::Delete, "Delete"),
        (K::Home, "Home"),
        (K::End, "End"),
        (K::PageUp, "PageUp"),
        (K::PageDown, "PageDown"),
        (K::ArrowLeft, "Left"),
        (K::ArrowUp, "Up"),
        (K::ArrowRight, "Right"),
        (K::ArrowDown, "Down"),
    ];
    if let Some((_, n)) = DIGITS.iter().chain(NAMED.iter()).find(|(key, _)| *key == k) {
        return Some((*n).to_string());
    }
    let n = k.name();
    let is_letter = n.len() == 1 && n.chars().all(|c| c.is_ascii_uppercase());
    let is_f = n.starts_with('F') && n[1..].parse::<u32>().map(|i| (1..=12).contains(&i)).unwrap_or(false);
    (is_letter || is_f).then(|| n.to_string())
}

enum Captured {
    Cancel,
    Keys(Hotkey),
}

/// The next key press while listening. Esc cancels.
fn capture_hotkey(ctx: &egui::Context) -> Option<Captured> {
    ctx.input(|i| {
        for e in &i.events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                if *key == egui::Key::Escape {
                    return Some(Captured::Cancel);
                }
                if let Some(name) = key_name(*key) {
                    return Some(Captured::Keys(Hotkey { ctrl: modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift, key: name }));
                }
            }
        }
        None
    })
}

/// A hotkey row. Click the button, then press the new shortcut.
fn hotkey_row(ui: &mut Ui, title: &str, sub: &str, h: &mut Hotkey, listening: &mut Option<String>, changed: &mut bool) {
    let mine = listening.as_deref() == Some(title);
    row(ui, title, sub, |ui| {
        let text = if mine { "Listening...".to_string() } else { hotkey_text(h) };
        let b = egui::Button::new(RichText::new(text).strong().color(if mine { Color32::from_rgb(150, 160, 255) } else { TEXT }))
            .fill(INPUT)
            .stroke(Stroke::new(if mine { 2.0_f32 } else { 1.0_f32 }, if mine { ACCENT } else { BORDER }))
            .rounding(8.0)
            .min_size(vec2(220.0, 34.0));
        if ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            if mine {
                *listening = None;
                win::notify(win::MSG_RESUME_KEYS);
            } else {
                if listening.is_none() {
                    // Release the global hotkeys, otherwise the daemon would
                    // swallow the combination before this window sees it.
                    win::notify(win::MSG_PAUSE_KEYS);
                }
                *listening = Some(title.to_string());
            }
        }
        if mine {
            match capture_hotkey(ui.ctx()) {
                Some(Captured::Keys(new)) => {
                    *h = new;
                    *changed = true;
                    *listening = None;
                    win::notify(win::MSG_RESUME_KEYS);
                }
                Some(Captured::Cancel) => {
                    *listening = None;
                    win::notify(win::MSG_RESUME_KEYS);
                }
                None => {}
            }
        }
    });
}

// ---------------------------------------------------------------------- style

fn setup_style(ctx: &egui::Context) {
    // Segoe UI ships with every Windows install, so it costs nothing to ship.
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(bytes));
        if let Some(f) = fonts.families.get_mut(&FontFamily::Proportional) {
            f.insert(0, "segoe".into());
        }
    }
    ctx.set_fonts(fonts);

    let mut s = (*ctx.style()).clone();
    s.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
    s.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
    s.text_styles.insert(TextStyle::Heading, FontId::proportional(24.0));
    s.text_styles.insert(TextStyle::Small, FontId::proportional(12.5));
    s.spacing.item_spacing = vec2(10.0, 8.0);
    s.spacing.button_padding = vec2(14.0, 5.0);
    s.spacing.interact_size.y = 28.0;
    s.spacing.combo_width = 140.0;

    let v = &mut s.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_rounding = egui::Rounding::same(10.0);
    v.menu_rounding = egui::Rounding::same(8.0);
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = CARD;
    v.hyperlink_color = Color32::from_rgb(140, 140, 255);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    let r = egui::Rounding::same(7.0);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.rounding = r;
    for (w, fill, stroke) in [
        (&mut v.widgets.inactive, Color32::from_rgb(34, 34, 42), BORDER),
        (&mut v.widgets.hovered, Color32::from_rgb(44, 44, 56), Color32::from_rgb(64, 64, 80)),
        (&mut v.widgets.active, Color32::from_rgb(54, 54, 70), ACCENT),
    ] {
        w.weak_bg_fill = fill;
        w.bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0_f32, stroke);
        w.rounding = r;
        w.fg_stroke = Stroke::new(1.0_f32, TEXT);
    }
    v.widgets.open.weak_bg_fill = Color32::from_rgb(44, 44, 56);
    v.widgets.open.rounding = r;
    ctx.set_style(s);
}

impl eframe::App for App {
    /// Closing the window right after a change must not lose that change.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.dirty_since.is_some() {
            self.commit();
        }
        if self.listening.is_some() {
            win::notify(win::MSG_RESUME_KEYS);
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_external();
        self.poll_update();
        let mut changed = false;

        egui::SidePanel::left("side")
            .exact_width(220.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(SIDE).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(egui::Margin::symmetric(14.0, 16.0)))
            .show(ctx, |ui| self.sidebar(ui, &mut changed));

        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(26.0, 18.0))).show(ctx, |ui| {
            self.update_banner(ui);
            if !self.daemon_running {
                egui::Frame::none().fill(Color32::from_rgb(44, 34, 16)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(110, 80, 30))).rounding(8.0).inner_margin(egui::Margin::symmetric(12.0, 6.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("The crosshair is not running.").color(Color32::from_rgb(255, 200, 110)));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary(ui, "Start") {
                                let _ = std::process::Command::new(daemon_exe()).spawn();
                            }
                        });
                    });
                });
                ui.add_space(8.0);
            }
            match self.page {
                Page::Profile => self.profile_page(ui, &mut changed),
                Page::General => self.general_page(ui, &mut changed),
            }
        });

        self.flush(changed, ctx);
        ctx.request_repaint_after(Duration::from_millis(700));
    }
}

pub fn run() {
    // After an update the old settings window needs a moment to exit.
    if let Some(ms) = arg("--delay").and_then(|s| s.parse::<u64>().ok()) {
        std::thread::sleep(Duration::from_millis(ms.min(5000)));
    }
    if let Ok(exe) = std::env::current_exe() {
        update::clean_up(&exe);
    }
    if !win::single_instance(w!("Local\\XhCrosshairSettings")) {
        win::focus_settings();
        return;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Crosshair Settings")
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([820.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    let _ = eframe::run_native(
        "Crosshair Settings",
        options,
        Box::new(|cc| {
            setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    );
}
