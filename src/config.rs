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
    let base = Profile::default();
    vec![
        Profile { name: "Classic".into(), ..base.clone() },
        Profile {
            name: "Dot".into(),
            cross: false,
            dot: true,
            dot_size: 4,
            dot_round: true,
            ..base.clone()
        },
        Profile {
            name: "Tiny cross".into(),
            cross_len: 3,
            cross_thick: 1,
            cross_gap: 2,
            color: [255, 255, 255, 255],
            ..base.clone()
        },
        Profile {
            name: "T-style".into(),
            t_style: true,
            cross_len: 8,
            cross_gap: 3,
            color: [0, 229, 255, 255],
            ..base.clone()
        },
        Profile {
            name: "Ring".into(),
            cross: false,
            dot: true,
            dot_size: 2,
            circle: true,
            circle_radius: 14,
            color: [255, 214, 0, 255],
            ..base.clone()
        },
        Profile {
            name: "Square".into(),
            cross: false,
            dot: true,
            dot_size: 2,
            square: true,
            square_size: 14,
            color: [255, 64, 64, 255],
            ..base.clone()
        },
        Profile {
            name: "X cross".into(),
            rotation: 45.0,
            cross_len: 7,
            cross_gap: 3,
            color: [255, 64, 64, 255],
            ..base
        },
    ]
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
            hotkeys: Hotkeys::default(),
            profiles: presets(),
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
            self.profiles = presets();
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
