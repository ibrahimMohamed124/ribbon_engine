//! CPU-side UI vertex stream and clip batching.
use crate::{Color, Rect};
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UiVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}
#[derive(Clone, Debug, Default)]
pub struct UiDrawList {
    pub vertices: Vec<UiVertex>,
    pub batches: Vec<DrawBatch>,
    clip: Option<[u32; 4]>,
}
#[derive(Clone, Debug, Default)]
pub struct DrawBatch {
    pub clip: Option<[u32; 4]>,
    pub vertex_range: std::ops::Range<u32>,
}
impl UiDrawList {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.batches.clear();
        self.clip = None
    }
    pub fn rect(&mut self, r: Rect, c: Color) {
        let start = self.vertices.len() as u32;
        let a = [r.x, r.y];
        let b = [r.x + r.w, r.y];
        let d = [r.x, r.y + r.h];
        let e = [r.x + r.w, r.y + r.h];
        for p in [a, b, e, a, e, d] {
            self.vertices.push(UiVertex {
                position: p,
                color: c.0,
            });
        }
        self.batches.push(DrawBatch {
            clip: self.clip,
            vertex_range: start..self.vertices.len() as u32,
        });
    }
    pub fn clip_push(&mut self, rect: Rect) {
        self.clip = Some([
            rect.x.max(0.0) as u32,
            rect.y.max(0.0) as u32,
            rect.w.max(0.0) as u32,
            rect.h.max(0.0) as u32,
        ]);
    }
    pub fn clip_pop(&mut self) {
        self.clip = None
    }
}
