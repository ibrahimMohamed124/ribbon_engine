//! CPU-side UI vertex stream, text commands, and nested clip batching.
use crate::{Color, Rect, TextDirection};

#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UiVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// 0 = rounded/solid shape, 1 = glyph mask sampled from the R8 atlas.
    pub kind: u32,
    /// Normalized coordinates within the primitive rectangle.
    pub local: [f32; 2],
    pub rect_size: [f32; 2],
    pub radius: f32,
    /// When non-zero, render only the rounded outer border ring.
    pub border: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct TextDraw {
    pub text: String,
    pub rect: Rect,
    pub size: f32,
    pub color: Color,
    pub direction: TextDirection,
    pub clip: Option<[u32; 4]>,
    pub(crate) order: u64,
}

#[derive(Clone, Debug, Default)]
pub struct UiDrawList {
    pub vertices: Vec<UiVertex>,
    pub batches: Vec<DrawBatch>,
    pub(crate) texts: Vec<TextDraw>,
    clip: Option<[u32; 4]>,
    clip_stack: Vec<Option<[u32; 4]>>,
    next_order: u64,
}

#[derive(Clone, Debug, Default)]
pub struct DrawBatch {
    pub clip: Option<[u32; 4]>,
    pub vertex_range: std::ops::Range<u32>,
    pub(crate) order: u64,
}

impl UiDrawList {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.batches.clear();
        self.texts.clear();
        self.clip_stack.clear();
        self.clip = None;
        self.next_order = 0;
    }

    pub fn rect(&mut self, rect: Rect, color: Color) {
        self.rounded_rect(rect, color, 0.0);
    }

    pub fn rounded_rect(&mut self, rect: Rect, color: Color, radius: f32) {
        self.shape(rect, color, radius, 0.0);
    }

    pub fn rounded_border(&mut self, rect: Rect, color: Color, radius: f32, width: f32) {
        self.shape(rect, color, radius, width);
    }

    fn shape(&mut self, rect: Rect, color: Color, radius: f32, border: f32) {
        if !rect.x.is_finite()
            || !rect.y.is_finite()
            || !rect.w.is_finite()
            || !rect.h.is_finite()
            || rect.w <= 0.0
            || rect.h <= 0.0
        {
            return;
        }
        let start = self.vertices.len() as u32;
        let a = [rect.x, rect.y];
        let b = [rect.x + rect.w, rect.y];
        let d = [rect.x, rect.y + rect.h];
        let e = [rect.x + rect.w, rect.y + rect.h];
        for (position, local) in [
            (a, [0.0, 0.0]),
            (b, [1.0, 0.0]),
            (e, [1.0, 1.0]),
            (a, [0.0, 0.0]),
            (e, [1.0, 1.0]),
            (d, [0.0, 1.0]),
        ] {
            self.vertices.push(UiVertex {
                position,
                uv: [0.0, 0.0],
                color: color.0,
                kind: 0,
                local,
                rect_size: [rect.w, rect.h],
                radius: radius.max(0.0).min(rect.w.min(rect.h) * 0.5),
                border: border.max(0.0),
            });
        }
        let order = self.next_order;
        self.next_order = self.next_order.wrapping_add(1);
        self.batches.push(DrawBatch {
            clip: self.clip,
            vertex_range: start..self.vertices.len() as u32,
            order,
        });
    }

    pub(crate) fn text(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        color: Color,
        direction: TextDirection,
    ) {
        if text.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 || size <= 0.0 {
            return;
        }
        let order = self.next_order;
        self.next_order = self.next_order.wrapping_add(1);
        self.texts.push(TextDraw {
            text: text.to_owned(),
            rect,
            size,
            color,
            direction,
            clip: self.clip,
            order,
        });
    }

    pub fn clip_push(&mut self, rect: Rect) {
        self.clip_stack.push(self.clip);
        let left = rect.x.max(0.0).floor().min(u32::MAX as f32) as u32;
        let top = rect.y.max(0.0).floor().min(u32::MAX as f32) as u32;
        let right = (rect.x + rect.w).max(0.0).ceil().min(u32::MAX as f32) as u32;
        let bottom = (rect.y + rect.h).max(0.0).ceil().min(u32::MAX as f32) as u32;
        let next = [
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        ];
        self.clip = Some(if let Some(old) = self.clip {
            let x = old[0].max(next[0]);
            let y = old[1].max(next[1]);
            let r = old[0]
                .saturating_add(old[2])
                .min(next[0].saturating_add(next[2]));
            let b = old[1]
                .saturating_add(old[3])
                .min(next[1].saturating_add(next[3]));
            [x, y, r.saturating_sub(x), b.saturating_sub(y)]
        } else {
            next
        });
    }

    pub fn clip_pop(&mut self) {
        self.clip = self.clip_stack.pop().flatten();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_clips_intersect_and_restore_parent() {
        let mut draw = UiDrawList::default();
        draw.clip_push(Rect {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 100.0,
        });
        draw.rect(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            Color([1.0; 4]),
        );
        assert_eq!(draw.batches[0].clip, Some([10, 10, 100, 100]));
        draw.clip_push(Rect {
            x: 50.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        });
        draw.rect(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            Color([1.0; 4]),
        );
        assert_eq!(draw.batches[1].clip, Some([50, 10, 60, 40]));
        draw.clip_pop();
        draw.rect(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            Color([1.0; 4]),
        );
        assert_eq!(draw.batches[2].clip, Some([10, 10, 100, 100]));
    }

    #[test]
    fn shape_and_text_commands_keep_their_original_order() {
        let mut draw = UiDrawList::default();
        draw.rect(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            Color([1.0; 4]),
        );
        draw.text(
            "first",
            Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            12.0,
            Color([1.0; 4]),
            TextDirection::Auto,
        );
        draw.rounded_rect(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            Color([0.0; 4]),
            3.0,
        );
        assert_eq!(draw.batches[0].order, 0);
        assert_eq!(draw.texts[0].order, 1);
        assert_eq!(draw.batches[1].order, 2);
        assert_eq!(draw.batches[0].vertex_range, 0..6);
        assert_eq!(draw.batches[1].vertex_range, 6..12);
        assert!(draw.vertices.iter().all(|v| v.rect_size == [10.0, 10.0]));
        assert!(draw.vertices[..6].iter().all(|v| v.radius == 0.0));
        assert!(draw.vertices[6..12].iter().all(|v| v.radius == 3.0));
    }
}
