use voxelworks_core::gfx::Canvas;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub fn centered_panel(fb: &impl Canvas, width: i32, height: i32) -> Rect {
    centered_panel_for_size(fb.width(), fb.height(), width, height)
}

pub fn centered_panel_for_size(
    frame_width: usize,
    frame_height: usize,
    width: i32,
    height: i32,
) -> Rect {
    let width = width.min(frame_width as i32 - 12).max(1);
    let height = height.min(frame_height as i32 - 12).max(1);
    Rect {
        x: (frame_width as i32 - width) / 2,
        y: (frame_height as i32 - height) / 2,
        width,
        height,
    }
}

impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x as f32
            && y >= self.y as f32
            && x < (self.x + self.width) as f32
            && y < (self.y + self.height) as f32
    }
}

pub fn text_x(panel: Rect, value_width: i32) -> i32 {
    panel.x + (panel.width - value_width) / 2
}
