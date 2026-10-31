use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Profile {
    pub name: String,

    pub cross: bool,
    pub cross_len: u32,
    pub cross_thick: u32,
    pub cross_gap: u32,
    pub t_style: bool,
    pub rotation: f32,
    /// Use a different length for the vertical lines.
    pub cross_split: bool,
    pub cross_len_v: u32,

    pub dot: bool,
    pub dot_size: u32,
    pub dot_round: bool,

    pub circle: bool,
    pub circle_radius: u32,
    pub circle_thick: u32,

    pub square: bool,
    /// Side length in pixels.
    pub square_size: u32,
    pub square_thick: u32,
    pub square_filled: bool,
    /// 45 turns the square into a diamond.
    pub square_rotation: f32,

    /// Four rounded corner brackets around the center.
    pub brackets: bool,
    /// Side length of the area the brackets frame.
    pub brackets_size: u32,
    /// Length of each bracket arm.
    pub brackets_arm: u32,
    pub brackets_thick: u32,
    pub brackets_radius: u32,
    pub brackets_rotation: f32,

    /// A "^" below the center.
    pub chevron: bool,
    pub chevron_size: u32,
    pub chevron_thick: u32,
    pub chevron_offset: u32,
    /// Put it above the center as a "v".
    pub chevron_flip: bool,

    pub use_image: bool,
    pub image_path: String,
    pub image_scale: f32,

    pub color: [u8; 4],
    pub opacity: f32,
    pub outline: bool,
    pub outline_thick: u32,
    pub outline_color: [u8; 4],

    /// 0 = primary monitor, otherwise index in the monitor list.
    pub monitor: u32,
    /// Offset in pixels from the centre of the monitor.
    pub offset_x: i32,
    pub offset_y: i32,

    /// Executable names (e.g. `cs2.exe`) that activate this profile.
    pub games: Vec<String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "New profile".into(),
            cross: true,
            cross_len: 6,
            cross_thick: 2,
            cross_gap: 4,
            t_style: false,
            rotation: 0.0,
            cross_split: false,
            cross_len_v: 6,
            dot: false,
            dot_size: 2,
            dot_round: false,
            circle: false,
            circle_radius: 12,
            circle_thick: 2,
            square: false,
            square_size: 16,
            square_thick: 2,
            square_filled: false,
            square_rotation: 0.0,
            brackets: false,
            brackets_size: 28,
            brackets_arm: 9,
            brackets_thick: 2,
            brackets_radius: 6,
            brackets_rotation: 0.0,
            chevron: false,
            chevron_size: 9,
            chevron_thick: 2,
            chevron_offset: 10,
            chevron_flip: false,
            use_image: false,
            image_path: String::new(),
            image_scale: 1.0,
            color: [0, 255, 102, 255],
            opacity: 1.0,
            outline: true,
            outline_thick: 1,
            outline_color: [0, 0, 0, 255],
            monitor: 0,
            offset_x: 0,
            offset_y: 0,
            games: Vec::new(),
        }
    }
}

impl Profile {
    fn sanitize(&mut self) {
        self.cross_len = self.cross_len.min(200);
        self.cross_thick = self.cross_thick.clamp(1, 40);
        self.cross_gap = self.cross_gap.min(100);
        self.dot_size = self.dot_size.clamp(1, 60);
        self.circle_radius = self.circle_radius.clamp(1, 300);
        self.circle_thick = self.circle_thick.clamp(1, 40);
        self.square_size = self.square_size.clamp(2, 200);
        self.square_thick = self.square_thick.clamp(1, 40);
        self.square_rotation = self.square_rotation.clamp(-180.0, 180.0);
        self.cross_len_v = self.cross_len_v.min(200);
        self.brackets_size = self.brackets_size.clamp(4, 300);
        self.brackets_arm = self.brackets_arm.clamp(1, 150);
        self.brackets_thick = self.brackets_thick.clamp(1, 20);
        self.brackets_radius = self.brackets_radius.min(60);
        self.brackets_rotation = self.brackets_rotation.clamp(-180.0, 180.0);
        self.chevron_size = self.chevron_size.clamp(2, 60);
        self.chevron_thick = self.chevron_thick.clamp(1, 20);
        self.chevron_offset = self.chevron_offset.min(200);
        self.outline_thick = self.outline_thick.min(10);
        self.opacity = self.opacity.clamp(0.05, 1.0);
        self.image_scale = self.image_scale.clamp(0.1, 8.0);
        self.rotation = self.rotation.clamp(-180.0, 180.0);
        self.offset_x = self.offset_x.clamp(-8000, 8000);
        self.offset_y = self.offset_y.clamp(-8000, 8000);
        self.games.retain(|g| !g.trim().is_empty());
    }
}

pub fn presets() -> Vec<Profile> {
    let b = Profile::default();
    let white = [255, 255, 255, 255];
    vec![
        Profile { name: "Classic".into(), ..b.clone() },
        Profile { name: "Dot".into(), cross: false, dot: true, dot_size: 4, dot_round: true, ..b.clone() },
        Profile { name: "Tiny cross".into(), cross_len: 3, cross_thick: 1, cross_gap: 2, color: white, ..b.clone() },
        Profile { name: "T-style".into(), t_style: true, cross_len: 8, cross_gap: 3, color: [0, 229, 255, 255], ..b.clone() },
        Profile { name: "Ring".into(), cross: false, dot: true, dot_size: 2, circle: true, circle_radius: 14, color: [255, 214, 0, 255], ..b.clone() },
        Profile { name: "Square".into(), cross: false, dot: true, dot_size: 2, square: true, square_size: 14, color: [255, 64, 64, 255], ..b.clone() },
        Profile { name: "X cross".into(), rotation: 45.0, cross_len: 7, cross_gap: 3, color: [255, 64, 64, 255], ..b.clone() },
        Profile {
            name: "Brackets".into(),
            cross: false,
            dot: true,
            dot_size: 3,
            brackets: true,
            brackets_size: 30,
            brackets_arm: 9,
            brackets_radius: 7,
            color: white,
            ..b.clone()
        },
        Profile {
            name: "Frame".into(),
            cross: false,
            dot: true,
            dot_size: 2,
            dot_round: true,
            brackets: true,
            brackets_size: 44,
            brackets_arm: 12,
            brackets_thick: 3,
            brackets_radius: 0,
            color: [0, 255, 102, 255],
            ..b.clone()
        },
        Profile { name: "Plus".into(), cross_len: 10, cross_thick: 1, cross_gap: 0, color: white, ..b.clone() },
        Profile { name: "Precision".into(), cross_len: 20, cross_thick: 1, cross_gap: 8, dot: true, dot_size: 2, color: [255, 64, 64, 255], ..b.clone() },
        Profile { name: "Thick cross".into(), cross_len: 12, cross_thick: 4, cross_gap: 5, color: white, ..b.clone() },
        Profile { name: "Tall cross".into(), cross_split: true, cross_len: 4, cross_len_v: 10, cross_gap: 3, color: [0, 229, 255, 255], ..b.clone() },
        Profile { name: "Hollow ring".into(), cross: false, circle: true, circle_radius: 8, circle_thick: 2, color: white, ..b.clone() },
        Profile { name: "Big circle".into(), cross: false, dot: true, dot_size: 2, circle: true, circle_radius: 26, circle_thick: 1, color: [0, 255, 102, 255], ..b.clone() },
        Profile {
            name: "Diamond".into(),
            cross: false,
            dot: true,
            dot_size: 2,
            square: true,
            square_size: 16,
            square_rotation: 45.0,
            color: [255, 0, 200, 255],
            ..b.clone()
        },
        Profile { name: "Pixel".into(), cross: false, square: true, square_size: 4, square_filled: true, color: white, ..b.clone() },
        Profile {
            name: "Chevron".into(),
            cross: false,
            dot: true,
            dot_size: 2,
            chevron: true,
            chevron_size: 8,
            chevron_offset: 8,
            color: [255, 214, 0, 255],
            ..b.clone()
        },
        Profile {
            name: "Sniper".into(),
            cross_len: 40,
            cross_thick: 1,
            cross_gap: 6,
            dot: true,
            dot_size: 2,
            circle: true,
            circle_radius: 60,
            circle_thick: 1,
            color: [255, 64, 64, 200],
            ..b
        },
    ]
}
/// The few profiles a fresh install starts with. The rest are in the
/// "new profile" menu.
pub fn default_profiles() -> Vec<Profile> {
    const NAMES: [&str; 5] = ["Classic", "Dot", "Brackets", "T-style", "Ring"];
    presets().into_iter().filter(|p| NAMES.contains(&p.name.as_str())).collect()
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

impl Hotkey {
    fn ca(key: &str) -> Self {
        Self { ctrl: true, alt: true, shift: false, key: key.into() }
    }
}

impl Default for Hotkey {
    fn default() -> Self {
        Self::ca("F1")
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Hotkeys {
    pub toggle: Hotkey,
    pub next: Hotkey,
    pub drag: Hotkey,
    pub settings: Hotkey,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle: Hotkey::ca("F1"),
            next: Hotkey::ca("F2"),
            drag: Hotkey::ca("F3"),
            settings: Hotkey::ca("F4"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Config {
    /// Whether the crosshair is shown. Remembered across restarts.
    pub enabled: bool,
    pub active: usize,
    /// Switch profile automatically when a listed game is in the foreground.
    pub auto_switch: bool,
    /// Only show the crosshair while a listed game is in the foreground.
    pub hide_outside_games: bool,
    /// Re-assert "always on top" when the foreground window changes.
    pub keep_on_top: bool,
    /// Open the settings window when the exe is started by hand.
    /// Starting with Windows always stays in the tray.
    pub open_settings_on_start: bool,
    pub hotkeys: Hotkeys,
    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            active: 0,
            auto_switch: true,
            hide_outside_games: false,
            keep_on_top: true,
            open_settings_on_start: true,
            hotkeys: Hotkeys::default(),
            profiles: default_profiles(),
        }
    }
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Crosshair")
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

impl Config {
    pub fn load() -> Self {
        let mut cfg = match std::fs::read_to_string(path()) {
            Ok(text) => toml::from_str::<Config>(&text).unwrap_or_else(|_| {
                // Hand-edited file that no longer parses: keep a copy so the
                // next save does not silently destroy the user's profiles.
                let _ = std::fs::copy(path(), dir().join("config.toml.bak"));
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        cfg.sanitize();
        cfg
    }

    pub fn sanitize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles = default_profiles();
        }
        for p in &mut self.profiles {
            p.sanitize();
        }
        self.active = self.active.min(self.profiles.len() - 1);
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(dir())?;
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = dir().join("config.toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path())
    }

    pub fn mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(path()).and_then(|m| m.modified()).ok()
    }

    /// Index of the first profile that lists this executable.
    pub fn game_profile(&self, exe: &str) -> Option<usize> {
        self.profiles
            .iter()
            .position(|p| p.games.iter().any(|g| g.trim().eq_ignore_ascii_case(exe)))
    }

    pub fn has_games(&self) -> bool {
        self.profiles.iter().any(|p| !p.games.is_empty())
    }
}

pub fn key_names() -> Vec<String> {
    let mut v: Vec<String> = (1..=12).map(|i| format!("F{i}")).collect();
    v.extend((b'A'..=b'Z').map(|c| (c as char).to_string()));
    v.extend((b'0'..=b'9').map(|c| (c as char).to_string()));
    v.extend(["Insert", "Delete", "Home", "End", "PageUp", "PageDown", "Left", "Up", "Right", "Down"].map(String::from));
    v
}

pub fn vk(name: &str) -> Option<u32> {
    match name {
        "Insert" => Some(0x2D),
        "Delete" => Some(0x2E),
        "Home" => Some(0x24),
        "End" => Some(0x23),
        "PageUp" => Some(0x21),
        "PageDown" => Some(0x22),
        "Left" => Some(0x25),
        "Up" => Some(0x26),
        "Right" => Some(0x27),
        "Down" => Some(0x28),
        n if n.len() >= 2 && n.starts_with('F') => {
            n[1..].parse::<u32>().ok().filter(|i| (1..=12).contains(i)).map(|i| 0x6F + i)
        }
        n if n.len() == 1 => {
            let c = n.as_bytes()[0].to_ascii_uppercase();
            (c.is_ascii_uppercase() || c.is_ascii_digit()).then_some(c as u32)
        }
        _ => None,
    }
}

const SHARE_PREFIX: &str = "XH1:";

pub fn export_code(p: &Profile) -> String {
    use base64::Engine;
    let mut q = p.clone();
    q.games.clear();
    let text = toml::to_string(&q).unwrap_or_default();
    format!("{SHARE_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text))
}

pub fn import_code(code: &str) -> Option<Profile> {
    use base64::Engine;
    let body = code.trim().strip_prefix(SHARE_PREFIX)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body).ok()?;
    let mut p: Profile = toml::from_str(std::str::from_utf8(&bytes).ok()?).ok()?;
    p.sanitize();
    Some(p)
}
