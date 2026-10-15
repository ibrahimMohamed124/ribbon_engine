use engine_core::gfx::{fill_rect, Canvas, Effect};

pub const INK: u32 = 0x0008_0a0d;
pub const PANEL: u32 = 0x0010_1418;
pub const PANEL_EDGE: u32 = 0x0037_4148;
pub const TEXT: u32 = 0x00d9_dddf;
pub const MUTED: u32 = 0x0078_8289;
pub const ACCENT: u32 = 0x00c2_4545;
pub const SELECTED: u32 = 0x001b_252c;

pub fn text(fb: &mut impl Canvas, value: &str, x: i32, y: i32, scale: i32, color: u32) {
    fb.text(value, x, y, scale, color);
}

pub fn fog_background(fb: &mut impl Canvas, frame: u32) {
    fb.effect(Effect::FogBackground { frame });
}

pub fn dim(fb: &mut impl Canvas, amount: f32) {
    fb.dim(amount);
}

pub fn panel(fb: &mut impl Canvas, x: i32, y: i32, w: i32, h: i32) {
    fill_rect(fb, x, y, w, h, PANEL);
    fill_rect(fb, x, y, w, 1, PANEL_EDGE);
    fill_rect(fb, x, y + h - 1, w, 1, PANEL_EDGE);
    fill_rect(fb, x, y, 1, h, PANEL_EDGE);
    fill_rect(fb, x + w - 1, y, 1, h, PANEL_EDGE);
    fill_rect(fb, x + 10, y + 10, 2, 14, ACCENT);
}

pub fn centered_text(fb: &mut impl Canvas, value: &str, y: i32, scale: i32, color: u32) {
    let width = value.chars().count() as i32 * 6 * scale - scale;
    fb.text(value, (fb.width() as i32 - width) / 2, y, scale, color);
}

pub fn label_value(fb: &mut impl Canvas, label: &str, value: &str, x: i32, y: i32, width: i32) {
    fb.text(label, x, y, 1, TEXT);
    let value_width = value.chars().count() as i32 * 6 - 1;
    fb.text(value, x + width - value_width, y, 1, MUTED);
}
