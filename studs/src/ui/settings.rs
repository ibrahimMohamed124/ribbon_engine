use crate::settings::Settings;

pub const TITLE: &str = "SETTINGS / AUDIO / GAME / DISPLAY";
pub const HINT: &str = "CLICK SELECT   L/R CHANGE   ESC BACK";
pub const ITEM_COUNT: usize = 14;

pub fn label(index: usize) -> &'static str {
    match index {
        0 => "MASTER VOLUME",
        1 => "SOUND EFFECTS",
        2 => "AMBIENT VOLUME",
        3 => "MUTE ALL",
        4 => "MOUSE SENSITIVITY",
        5 => "INVERT MOUSE Y",
        6 => "FIELD OF VIEW",
        7 => "FLASHLIGHT ON START",
        8 => "MOUSE LOOK ON START",
        9 => "WINDOW SCALE",
        10 => "RENDER RESOLUTION",
        11 => "BRIGHTNESS",
        12 => "VISUAL STYLE",
        _ => "BACK",
    }
}

pub fn value(settings: &Settings, index: usize) -> String {
    match index {
        0 => format!("{:>3.0}%", settings.master_volume * 100.0),
        1 => format!("{:>3.0}%", settings.effects_volume * 100.0),
        2 => format!("{:>3.0}%", settings.ambient_volume * 100.0),
        3 => if settings.mute_all { "ON" } else { "OFF" }.to_owned(),
        4 => {
            if settings.mouse_sensitivity == 0.0 {
                "OFF".to_owned()
            } else {
                format!(
                    "{}/10",
                    (settings.mouse_sensitivity * 1000.0)
                        .round()
                        .clamp(1.0, 10.0) as u32
                )
            }
        }
        5 => if settings.invert_y { "ON" } else { "OFF" }.to_owned(),
        6 => format!("{:.0} DEG", settings.hfov_degrees),
        7 => if settings.flashlight_on_start {
            "ON"
        } else {
            "OFF"
        }
        .to_owned(),
        8 => if settings.mouse_look_on_start {
            "ON"
        } else {
            "OFF"
        }
        .to_owned(),
        9 => format!("{}X", settings.window_scale),
        10 => format!("{}X{}", settings.render_width, settings.render_height),
        11 => format!("{:.0}%", settings.brightness * 100.0),
        12 => settings.visual_style.as_str().to_ascii_uppercase(),
        _ => String::new(),
    }
}

/// Adjust a setting. Returns true when the window or framebuffer must be rebuilt.
pub fn adjust(settings: &mut Settings, index: usize, direction: i32) -> (bool, bool) {
    let old_scale = settings.window_scale;
    let old_resolution = (settings.render_width, settings.render_height);
    match index {
        0 => {
            settings.master_volume =
                (settings.master_volume + direction as f32 * 0.05).clamp(0.0, 1.0)
        }
        1 => {
            settings.effects_volume =
                (settings.effects_volume + direction as f32 * 0.05).clamp(0.0, 1.0)
        }
        2 => {
            settings.ambient_volume =
                (settings.ambient_volume + direction as f32 * 0.05).clamp(0.0, 1.0)
        }
        3 => settings.mute_all = !settings.mute_all,
        4 => {
            let level = (settings.mouse_sensitivity * 1000.0).round() as i32;
            settings.mouse_sensitivity = (level + direction).clamp(0, 10) as f32 / 1000.0;
        }
        5 => settings.invert_y = !settings.invert_y,
        6 => {
            settings.hfov_degrees =
                (settings.hfov_degrees + direction as f32 * 5.0).clamp(40.0, 140.0)
        }
        7 => settings.flashlight_on_start = !settings.flashlight_on_start,
        8 => settings.mouse_look_on_start = !settings.mouse_look_on_start,
        9 => {
            let max_scale = Settings::scale_limit(settings.render_width, settings.render_height);
            let scales: Vec<u32> = [1, 2, 4]
                .into_iter()
                .filter(|&scale| scale <= max_scale)
                .collect();
            settings.window_scale = cycle(&scales, settings.window_scale, direction);
        }
        10 => {
            const RESOLUTIONS: [(usize, usize); 3] = [(320, 180), (480, 270), (640, 360)];
            (settings.render_width, settings.render_height) = cycle(
                &RESOLUTIONS,
                (settings.render_width, settings.render_height),
                direction,
            );
            settings.window_scale = settings.window_scale.min(Settings::scale_limit(
                settings.render_width,
                settings.render_height,
            ));
        }
        11 => settings.brightness = (settings.brightness + direction as f32 * 0.1).clamp(0.5, 1.5),
        12 => settings.visual_style.toggle(),
        _ => return (false, false),
    }
    let window_changed = settings.window_scale != old_scale
        || (settings.render_width, settings.render_height) != old_resolution;
    (true, window_changed)
}

fn cycle<T: Copy + PartialEq>(values: &[T], current: T, direction: i32) -> T {
    let index = values
        .iter()
        .position(|&value| value == current)
        .unwrap_or(0) as i32;
    values[(index + direction).rem_euclid(values.len() as i32) as usize]
}
