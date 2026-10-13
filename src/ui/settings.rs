use crate::settings::Settings;

pub const TITLE: &str = "SETTINGS / AUDIO / GAME / DISPLAY";
pub const HINT: &str = "LEFT / RIGHT CHANGE   ESC BACK";
pub const ITEM_COUNT: usize = 12;

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
                format!("{:.4}", settings.mouse_sensitivity)
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
            settings.mouse_sensitivity =
                (settings.mouse_sensitivity + direction as f32 * 0.0005).clamp(0.0, 0.05)
        }
        5 => settings.invert_y = !settings.invert_y,
        6 => {
            settings.hfov_degrees =
                (settings.hfov_degrees + direction as f32 * 5.0).clamp(40.0, 140.0)
        }
        7 => settings.flashlight_on_start = !settings.flashlight_on_start,
        8 => settings.mouse_look_on_start = !settings.mouse_look_on_start,
        9 => {
            const SCALES: [u32; 5] = [1, 2, 4, 8, 16];
            settings.window_scale = cycle(&SCALES, settings.window_scale, direction);
        }
        10 => {
            const RESOLUTIONS: [(usize, usize); 3] = [(320, 180), (480, 270), (640, 360)];
            (settings.render_width, settings.render_height) = cycle(
                &RESOLUTIONS,
                (settings.render_width, settings.render_height),
                direction,
            );
        }
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
