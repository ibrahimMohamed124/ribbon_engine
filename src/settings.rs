//! User settings, persisted as a small `key = value` text file.
//!
//! Parsing is lenient on purpose: unknown keys, malformed values and comments
//! never fail loading, they just leave the default in place. Numbers are
//! clamped to sane ranges.
//!
//! ```text
//! # settings.cfg
//! master_volume = 0.8
//! hfov_degrees = 77
//! key.forward = I
//! key.look_up = W,Up
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::input::{key_name, parse_key, Action, KeyBindings};
use crate::renderer::{DEFAULT_HEIGHT, DEFAULT_HFOV, DEFAULT_WIDTH};

#[derive(Clone)]
pub struct Settings {
    /// 0..=1
    pub master_volume: f32,
    /// Horizontal field of view in degrees.
    pub hfov_degrees: f32,
    /// Internal render resolution (the window scales it up).
    pub render_width: usize,
    pub render_height: usize,
    /// Window scale factor: 1, 2, 4, 8 or 16.
    pub window_scale: u32,
    pub flashlight_on_start: bool,
    /// Radians per pixel of mouse movement; 0 disables mouse look.
    pub mouse_sensitivity: f32,
    pub bindings: KeyBindings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            master_volume: 0.8,
            hfov_degrees: DEFAULT_HFOV.to_degrees(),
            render_width: DEFAULT_WIDTH,
            render_height: DEFAULT_HEIGHT,
            window_scale: 4,
            flashlight_on_start: true,
            mouse_sensitivity: 0.0,
            bindings: KeyBindings::default(),
        }
    }
}

fn parse_finite(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().filter(|x| x.is_finite())
}

impl Settings {
    pub fn hfov_radians(&self) -> f32 {
        self.hfov_degrees.to_radians()
    }

    /// Where the settings file lives by default.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("raycast_blocks").join("settings.cfg")
    }

    /// Parse settings text; anything missing or invalid keeps its default.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            let (key, value) = match line.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => continue,
            };
            match key {
                "master_volume" => {
                    if let Some(v) = parse_finite(value) {
                        s.master_volume = v.clamp(0.0, 1.0);
                    }
                }
                "hfov_degrees" => {
                    if let Some(v) = parse_finite(value) {
                        s.hfov_degrees = v.clamp(40.0, 140.0);
                    }
                }
                "render_width" => {
                    if let Ok(v) = value.parse::<usize>() {
                        s.render_width = v.clamp(80, 1920);
                    }
                }
                "render_height" => {
                    if let Ok(v) = value.parse::<usize>() {
                        s.render_height = v.clamp(45, 1080);
                    }
                }
                "window_scale" => {
                    if let Ok(v) = value.parse::<u32>() {
                        if matches!(v, 1 | 2 | 4 | 8 | 16) {
                            s.window_scale = v;
                        }
                    }
                }
                "flashlight_on_start" => {
                    if let Ok(v) = value.parse::<bool>() {
                        s.flashlight_on_start = v;
                    }
                }
                "mouse_sensitivity" => {
                    if let Some(v) = parse_finite(value) {
                        s.mouse_sensitivity = v.clamp(0.0, 0.05);
                    }
                }
                _ => {
                    if let Some(action) = key.strip_prefix("key.").and_then(Action::from_name) {
                        let keys: Vec<_> = value.split(',').filter_map(parse_key).collect();
                        if !keys.is_empty() {
                            s.bindings.set(action, keys);
                        }
                    }
                }
            }
        }
        s
    }

    /// Serialise to the text format understood by [`Settings::parse`].
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# raycast_blocks settings\n");
        out.push_str(&format!("master_volume = {}\n", self.master_volume));
        out.push_str(&format!("hfov_degrees = {}\n", self.hfov_degrees));
        out.push_str(&format!("render_width = {}\n", self.render_width));
        out.push_str(&format!("render_height = {}\n", self.render_height));
        out.push_str(&format!("window_scale = {}\n", self.window_scale));
        out.push_str(&format!("flashlight_on_start = {}\n", self.flashlight_on_start));
        out.push_str(&format!("mouse_sensitivity = {}\n", self.mouse_sensitivity));
        out.push_str("\n# key bindings: comma-separated key names\n");
        for action in Action::ALL {
            let names: Vec<&str> = self
                .bindings
                .keys(action)
                .iter()
                .filter_map(|k| key_name(*k))
                .collect();
            out.push_str(&format!("key.{} = {}\n", action.name(), names.join(",")));
        }
        out
    }

    pub fn load(path: &Path) -> io::Result<Settings> {
        Ok(Settings::parse(&fs::read_to_string(path)?))
    }

    /// Load from `path`, falling back to defaults if it is missing or unreadable.
    pub fn load_or_default(path: &Path) -> Settings {
        Settings::load(path).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        fs::write(path, self.to_text())
    }
}
