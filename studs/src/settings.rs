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

use crate::game::Difficulty;
use crate::input::{key_name, parse_key, Action, KeyBindings};
use crate::renderer::{DEFAULT_HEIGHT, DEFAULT_HFOV, DEFAULT_WIDTH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisualStyle {
    Classic,
    Moody,
    #[default]
    Realistic,
}

impl VisualStyle {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "classic" => Some(Self::Classic),
            "moody" => Some(Self::Moody),
            "realistic" => Some(Self::Realistic),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Moody => "moody",
            Self::Realistic => "realistic",
        }
    }

    pub fn render_style(self, quality: QualityPreset) -> engine_scene::RenderStyle {
        let mut style = match self {
            Self::Classic => engine_scene::RenderStyle::classic(),
            Self::Moody => engine_scene::RenderStyle::moody(),
            Self::Realistic => engine_scene::RenderStyle::realistic(),
        };
        match quality {
            QualityPreset::Low => {
                style.pbr_enabled = false;
                style.ao_strength *= 0.35;
                style.flashlight_shadows = false;
            }
            QualityPreset::Medium => {
                style.pbr_enabled = false;
                style.ao_strength *= 0.7;
                style.flashlight_shadows = false;
            }
            QualityPreset::High | QualityPreset::Realistic => {}
        }
        if self != Self::Realistic {
            style.flashlight_shadows = false;
        }
        style
    }

    pub fn toggle(&mut self) {
        *self = match self {
            Self::Classic => Self::Moody,
            Self::Moody => Self::Realistic,
            Self::Realistic => Self::Classic,
        };
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QualityPreset {
    Low,
    Medium,
    High,
    #[default]
    Realistic,
}

impl QualityPreset {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "realistic" => Some(Self::Realistic),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Realistic => "realistic",
        }
    }

    pub fn toggle(&mut self, direction: i32) {
        const VALUES: [QualityPreset; 4] = [
            QualityPreset::Low,
            QualityPreset::Medium,
            QualityPreset::High,
            QualityPreset::Realistic,
        ];
        let i = VALUES.iter().position(|value| value == self).unwrap_or(1) as i32;
        *self = VALUES[(i + direction).rem_euclid(VALUES.len() as i32) as usize];
    }
}

#[derive(Clone)]
pub struct Settings {
    /// 0..=1
    pub master_volume: f32,
    pub effects_volume: f32,
    pub ambient_volume: f32,
    pub mute_all: bool,
    /// Horizontal field of view in degrees.
    pub hfov_degrees: f32,
    /// Internal render resolution (the window scales it up).
    pub render_width: usize,
    pub render_height: usize,
    /// Window scale factor, capped so the initial window stays near 1280x720.
    pub window_scale: u32,
    /// Linear display brightness: 0.5 is dim, 1.0 is unchanged, 1.5 is bright.
    pub brightness: f32,
    pub visual_style: VisualStyle,
    pub quality_preset: QualityPreset,
    pub quality_preset_explicit: bool,
    pub reduce_flashing: bool,
    /// Best completion time in seconds, indexed by Easy / Normal / Hard.
    pub best_times: [f32; 3],
    pub flashlight_on_start: bool,
    pub mouse_look_on_start: bool,
    pub invert_y: bool,
    /// Radians per pixel of mouse movement; 0 disables mouse look.
    pub mouse_sensitivity: f32,
    pub bindings: KeyBindings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            master_volume: 0.8,
            effects_volume: 1.0,
            ambient_volume: 1.0,
            mute_all: false,
            hfov_degrees: DEFAULT_HFOV.to_degrees(),
            render_width: DEFAULT_WIDTH,
            render_height: DEFAULT_HEIGHT,
            window_scale: 4,
            brightness: 1.0,
            visual_style: VisualStyle::default(),
            quality_preset: QualityPreset::default(),
            quality_preset_explicit: false,
            reduce_flashing: false,
            best_times: [0.0; 3],
            flashlight_on_start: true,
            mouse_look_on_start: true,
            invert_y: false,
            mouse_sensitivity: 0.0025,
            bindings: KeyBindings::default(),
        }
    }
}

/// Version used for settings migration and binding compatibility.
pub const SETTINGS_VERSION: u32 = 6;

fn is_movement_or_look(action: Action) -> bool {
    matches!(
        action,
        Action::Forward
            | Action::Back
            | Action::Left
            | Action::Right
            | Action::LookUp
            | Action::LookDown
            | Action::LookLeft
            | Action::LookRight
    )
}

fn parse_finite(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().filter(|x| x.is_finite())
}

impl Settings {
    pub fn scale_limit(width: usize, height: usize) -> u32 {
        let fit = (1280 / width.max(1)).min(720 / height.max(1));
        if fit >= 4 {
            4
        } else if fit >= 2 {
            2
        } else {
            1
        }
    }

    fn difficulty_index(difficulty: Difficulty) -> usize {
        match difficulty {
            Difficulty::Easy => 0,
            Difficulty::Normal => 1,
            Difficulty::Hard => 2,
        }
    }

    pub fn best_time(&self, difficulty: Difficulty) -> Option<f32> {
        let time = self.best_times[Self::difficulty_index(difficulty)];
        (time > 0.0 && time.is_finite()).then_some(time)
    }

    /// Record a new personal best. Returns true when the stored value changed.
    pub fn record_best_time(&mut self, difficulty: Difficulty, time: f32) -> bool {
        if !time.is_finite() || time <= 0.0 {
            return false;
        }
        let best = &mut self.best_times[Self::difficulty_index(difficulty)];
        if *best == 0.0 || time < *best {
            *best = time;
            true
        } else {
            false
        }
    }

    pub fn hfov_radians(&self) -> f32 {
        self.hfov_degrees.to_radians()
    }

    pub fn use_cpu_fallback_quality(&mut self) -> bool {
        if self.quality_preset_explicit {
            return false;
        }
        self.quality_preset = QualityPreset::Medium;
        true
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
        let stored_version = text.lines().find_map(|raw| {
            let line = raw.split('#').next().unwrap_or("").trim();
            let (key, value) = line.split_once('=')?;
            (key.trim() == "settings_version")
                .then(|| value.trim().parse::<u32>().ok())
                .flatten()
        });
        let legacy = stored_version.unwrap_or(0) < 2;
        let legacy_quit_binding = stored_version.unwrap_or(0) < 3;
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
                "effects_volume" => {
                    if let Some(v) = parse_finite(value) {
                        s.effects_volume = v.clamp(0.0, 1.0);
                    }
                }
                "ambient_volume" => {
                    if let Some(v) = parse_finite(value) {
                        s.ambient_volume = v.clamp(0.0, 1.0);
                    }
                }
                "mute_all" => {
                    if let Ok(value) = value.parse::<bool>() {
                        s.mute_all = value;
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
                        if matches!(v, 1 | 2 | 4) {
                            s.window_scale = v;
                        }
                    }
                }
                "brightness" => {
                    if let Some(v) = parse_finite(value) {
                        s.brightness = v.clamp(0.5, 1.5);
                    }
                }
                "visual_style" => {
                    if let Some(style) = VisualStyle::parse(value) {
                        s.visual_style = style;
                    }
                }
                "quality_preset" => {
                    if let Some(preset) = QualityPreset::parse(value) {
                        s.quality_preset = preset;
                        s.quality_preset_explicit = true;
                    }
                }
                "quality_preset_explicit" => {
                    if let Ok(explicit) = value.parse::<bool>() {
                        s.quality_preset_explicit = explicit;
                    }
                }
                "reduce_flashing" => {
                    if let Ok(value) = value.parse::<bool>() {
                        s.reduce_flashing = value;
                    }
                }
                "best_time_easy" | "best_time_normal" | "best_time_hard" => {
                    if let Some(v) = parse_finite(value) {
                        let index = match key {
                            "best_time_easy" => 0,
                            "best_time_normal" => 1,
                            _ => 2,
                        };
                        s.best_times[index] = v.clamp(0.0, 86_400.0);
                    }
                }
                "flashlight_on_start" => {
                    if let Ok(v) = value.parse::<bool>() {
                        s.flashlight_on_start = v;
                    }
                }
                "mouse_look_on_start" => {
                    if let Ok(v) = value.parse::<bool>() {
                        s.mouse_look_on_start = v;
                    }
                }
                "invert_y" => {
                    if let Ok(v) = value.parse::<bool>() {
                        s.invert_y = v;
                    }
                }
                "mouse_sensitivity" => {
                    if let Some(v) = parse_finite(value) {
                        s.mouse_sensitivity = v.clamp(0.0, 0.05);
                    }
                }
                _ => {
                    if let Some(action) = key.strip_prefix("key.").and_then(Action::from_name) {
                        // Old files hold the previous default movement/look keys
                        // (IJKL / WASD-look); keep the new defaults instead.
                        if (legacy && is_movement_or_look(action))
                            || (legacy_quit_binding && action == Action::Quit)
                        {
                            continue;
                        }
                        let keys: Vec<_> = value.split(',').filter_map(parse_key).collect();
                        if !keys.is_empty() {
                            s.bindings.set(action, keys);
                        }
                    }
                }
            }
        }
        s.window_scale = s
            .window_scale
            .min(Self::scale_limit(s.render_width, s.render_height));
        // Before mouse look had its own toggle, a saved zero sensitivity could
        // make the cursor hide while producing no camera movement. Version 4
        // migrates those old files to the usable default; zero remains available
        // in current files as an explicit way to disable mouse movement.
        if stored_version.unwrap_or(0) < 4 && s.mouse_sensitivity == 0.0 {
            s.mouse_sensitivity = Settings::default().mouse_sensitivity;
        }
        // Version 6 enables mouse look by default. Migrate older settings files
        // so a previously saved `false` does not keep mouse look disabled.
        if stored_version.unwrap_or(0) < 6 {
            s.mouse_look_on_start = true;
        }
        s
    }

    /// Serialise to the text format understood by [`Settings::parse`].
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# raycast_blocks settings\n");
        out.push_str(&format!("settings_version = {}\n", SETTINGS_VERSION));
        out.push_str(&format!("master_volume = {}\n", self.master_volume));
        out.push_str(&format!("effects_volume = {}\n", self.effects_volume));
        out.push_str(&format!("ambient_volume = {}\n", self.ambient_volume));
        out.push_str(&format!("mute_all = {}\n", self.mute_all));
        out.push_str(&format!("hfov_degrees = {}\n", self.hfov_degrees));
        out.push_str(&format!("render_width = {}\n", self.render_width));
        out.push_str(&format!("render_height = {}\n", self.render_height));
        out.push_str(&format!("window_scale = {}\n", self.window_scale));
        out.push_str(&format!("brightness = {}\n", self.brightness));
        out.push_str(&format!("visual_style = {}\n", self.visual_style.as_str()));
        out.push_str(&format!(
            "quality_preset = {}\n",
            self.quality_preset.as_str()
        ));
        out.push_str(&format!(
            "quality_preset_explicit = {}\n",
            self.quality_preset_explicit
        ));
        out.push_str(&format!("reduce_flashing = {}\n", self.reduce_flashing));
        out.push_str(&format!("best_time_easy = {}\n", self.best_times[0]));
        out.push_str(&format!("best_time_normal = {}\n", self.best_times[1]));
        out.push_str(&format!("best_time_hard = {}\n", self.best_times[2]));
        out.push_str(&format!(
            "flashlight_on_start = {}\n",
            self.flashlight_on_start
        ));
        out.push_str(&format!(
            "mouse_look_on_start = {}\n",
            self.mouse_look_on_start
        ));
        out.push_str(&format!("invert_y = {}\n", self.invert_y));
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
