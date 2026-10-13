use crate::renderer::Framebuffer;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub fn centered_panel(fb: &Framebuffer, width: i32, height: i32) -> Rect {
    let width = width.min(fb.width as i32 - 12).max(1);
    let height = height.min(fb.height as i32 - 12).max(1);
    Rect {
        x: (fb.width as i32 - width) / 2,
        y: (fb.height as i32 - height) / 2,
        width,
        height,
    }
}

pub fn text_x(panel: Rect, value_width: i32) -> i32 {
    panel.x + (panel.width - value_width) / 2
}
