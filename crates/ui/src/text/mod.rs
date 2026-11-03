//! Deterministic font shaping, glyph rasterization, and R8 atlas preparation.
mod atlas;
use atlas::GlyphAtlas;

use crate::draw::TextDraw;
use crate::{DrawBatch, TextDirection, UiVertex};
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem as CosmicFontSystem, Metrics, Shaping, SwashCache,
    SwashContent,
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    pub start: usize,
    pub end: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    /// True when this glyph belongs to a right-to-left bidi run.
    pub rtl: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaidOutText {
    pub glyphs: Vec<PositionedGlyph>,
    pub width: f32,
    pub height: f32,
    pub is_rtl: bool,
}

#[derive(Default)]
pub(crate) struct PreparedText {
    pub vertices: Vec<UiVertex>,
    pub batches: Vec<DrawBatch>,
}

#[derive(Clone, Copy)]
struct PendingPlacement {
    key: cosmic_text::CacheKey,
    x: i32,
    y: i32,
    color: [f32; 4],
    clip: Option<[u32; 4]>,
    order: u64,
}

pub struct TextSystem {
    fonts: CosmicFontSystem,
    swash: SwashCache,
    atlas: GlyphAtlas,
    metrics_cache: HashMap<(String, u32, u32, TextDirection), (LaidOutText, u64)>,
    // Reuse shaped Buffer allocations for recently rendered text commands.
    buffers: HashMap<(String, u32, u32, u32), (Buffer, u64)>,
    frame: u64,
    atlas_warning_pending: bool,
    last_atlas_warning: Option<Instant>,
}
impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}
impl TextSystem {
    pub fn new() -> Self {
        Self::with_fonts(vec![
            include_bytes!("../../assets/fonts/NotoSans-Regular.ttf").to_vec(),
            include_bytes!("../../assets/fonts/NotoSansArabic-Regular.ttf").to_vec(),
        ])
    }

    pub fn with_fonts(fonts: Vec<Vec<u8>>) -> Self {
        let sources: Vec<cosmic_text::fontdb::Source> = fonts
            .into_iter()
            .map(|bytes| cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(bytes)))
            .collect();
        let fonts = if sources.is_empty() {
            CosmicFontSystem::new()
        } else {
            CosmicFontSystem::new_with_fonts(sources)
        };
        Self {
            fonts,
            swash: SwashCache::new(),
            atlas: GlyphAtlas::new(),
            metrics_cache: HashMap::new(),
            buffers: HashMap::new(),
            frame: 0,
            atlas_warning_pending: false,
            last_atlas_warning: None,
        }
    }

    pub fn measure(&mut self, text: &str, size: f32, max_width: f32) -> (f32, f32) {
        let layout = self.layout(text, size, max_width, TextDirection::Auto);
        (layout.width, layout.height)
    }

    /// Shape a string using Cosmic Text's advanced bidi/shaping path.
    pub fn layout(
        &mut self,
        text: &str,
        size: f32,
        max_width: f32,
        direction: TextDirection,
    ) -> LaidOutText {
        let size = (finite_size(size) * 2.0).round() * 0.5;
        let max_width = finite_width(max_width);
        let key = (
            text.to_owned(),
            (size * 2.0).round().max(1.0) as u32,
            max_width.round().max(1.0) as u32,
            direction,
        );
        self.frame = self.frame.wrapping_add(1);
        if let Some((value, used)) = self.metrics_cache.get_mut(&key) {
            *used = self.frame;
            return value.clone();
        }
        let value = self.shape_metrics(text, size, max_width, direction);
        self.metrics_cache.insert(key, (value.clone(), self.frame));
        if self.frame.is_multiple_of(120) {
            let frame = self.frame;
            self.metrics_cache
                .retain(|_, (_, used)| frame.saturating_sub(*used) <= 120);
        }
        value
    }

    /// Returns the logical-pixel X coordinate of a byte-index caret position.
    pub fn caret_x_for_byte(&mut self, text: &str, size: f32, byte_index: usize) -> f32 {
        let layout = self.layout(text, size, 16_384.0, TextDirection::Auto);
        let byte_index = text
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .take_while(|index| *index <= byte_index.min(text.len()))
            .last()
            .unwrap_or(0);
        let mut fallback = 0.0;
        for glyph in &layout.glyphs {
            let leading = if glyph.rtl {
                glyph.x + glyph.width
            } else {
                glyph.x
            };
            let trailing = if glyph.rtl {
                glyph.x
            } else {
                glyph.x + glyph.width
            };
            if byte_index == glyph.start {
                return leading;
            }
            if byte_index == glyph.end {
                return trailing;
            }
            if byte_index > glyph.start && byte_index < glyph.end {
                return glyph.x + glyph.width * 0.5;
            }
            if byte_index > glyph.end {
                fallback = trailing;
            }
        }
        fallback
    }

    /// Maps a logical-pixel X position to the nearest UTF-8 byte boundary.
    pub fn byte_for_x(&mut self, text: &str, size: f32, x: f32) -> usize {
        let layout = self.layout(text, size, 16_384.0, TextDirection::Auto);
        let mut best_index = 0;
        let mut best_distance = f32::INFINITY;
        for glyph in &layout.glyphs {
            let left_index = if glyph.rtl { glyph.end } else { glyph.start };
            let right_index = if glyph.rtl { glyph.start } else { glyph.end };
            let left = (x - glyph.x).abs();
            if left < best_distance {
                best_distance = left;
                best_index = left_index;
            }
            let right = (x - (glyph.x + glyph.width)).abs();
            if right < best_distance {
                best_distance = right;
                best_index = right_index;
            }
        }
        text.char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .take_while(|index| *index <= best_index.min(text.len()))
            .last()
            .unwrap_or(0)
    }

    fn shape_metrics(
        &mut self,
        text: &str,
        size: f32,
        max_width: f32,
        direction: TextDirection,
    ) -> LaidOutText {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(&mut self.fonts, Some(max_width), None);
        buffer.set_text(
            &mut self.fonts,
            text,
            Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        let mut output = LaidOutText::default();
        let mut rtl_seen = false;
        for run in buffer.layout_runs() {
            output.width = output.width.max(run.line_w);
            output.height += run.line_height;
            let line_rtl = match direction {
                TextDirection::Auto => run.rtl,
                TextDirection::Ltr => false,
                TextDirection::Rtl => true,
            };
            rtl_seen |= line_rtl;
            for glyph in run.glyphs {
                output.glyphs.push(PositionedGlyph {
                    glyph_id: glyph.glyph_id,
                    start: glyph.start,
                    end: glyph.end,
                    x: glyph.x,
                    y: run.line_y,
                    width: glyph.w,
                    rtl: line_rtl,
                });
            }
        }
        output.is_rtl = rtl_seen;
        output
    }

    pub(crate) fn prepare(&mut self, texts: &[TextDraw], physical_scale: f32) -> PreparedText {
        let scale = if physical_scale.is_finite() {
            physical_scale.clamp(0.25, 8.0)
        } else {
            1.0
        };
        let mut pending_pixels = Vec::new();
        let mut pending_positions = Vec::new();
        let mut seen = HashSet::new();
        self.frame = self.frame.wrapping_add(1);

        for text in texts {
            let size = (finite_size(text.size) * 2.0).round() * 0.5;
            let max_width = finite_width(text.rect.w);
            let max_height = finite_width(text.rect.h);
            let buffer_key = (
                text.text.clone(),
                (size * 2.0).round().max(1.0) as u32,
                max_width.round().max(1.0) as u32,
                max_height.round().max(1.0) as u32,
            );
            let mut buffer = self
                .buffers
                .remove(&buffer_key)
                .map(|(buffer, _)| buffer)
                .unwrap_or_else(|| Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25)));
            buffer.set_size(&mut self.fonts, Some(max_width), Some(max_height));
            buffer.set_text(
                &mut self.fonts,
                &text.text,
                Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
            );
            buffer.shape_until_scroll(&mut self.fonts, false);

            for run in buffer.layout_runs() {
                let line_rtl = match text.direction {
                    TextDirection::Auto => run.rtl,
                    TextDirection::Ltr => false,
                    TextDirection::Rtl => true,
                };
                let align_offset = if line_rtl {
                    (text.rect.w - run.line_w).max(0.0)
                } else {
                    0.0
                };
                for glyph in run.glyphs {
                    let physical = glyph.physical(
                        (
                            (text.rect.x + align_offset) * scale,
                            (text.rect.y + run.line_y) * scale,
                        ),
                        scale,
                    );
                    let key = physical.cache_key;
                    if !seen.insert(key) {
                        pending_positions.push(PendingPlacement {
                            key,
                            x: physical.x,
                            y: physical.y,
                            color: text.color.0,
                            clip: text.clip,
                            order: text.order,
                        });
                        continue;
                    }
                    let image = self.swash.get_image(&mut self.fonts, key).as_ref();
                    let Some(image) = image else {
                        continue;
                    };
                    let alpha = match &image.content {
                        SwashContent::Mask => image.data.clone(),
                        SwashContent::Color => image
                            .data
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|pixel| pixel[3])
                            .collect(),
                        SwashContent::SubpixelMask => continue,
                    };
                    let width = image.placement.width;
                    let height = image.placement.height;
                    if width == 0 || height == 0 || alpha.len() < (width * height) as usize {
                        continue;
                    }
                    pending_pixels.push(atlas::PendingGlyph {
                        key,
                        width,
                        height,
                        left: image.placement.left,
                        top: image.placement.top,
                        alpha,
                    });
                    pending_positions.push(PendingPlacement {
                        key,
                        x: physical.x,
                        y: physical.y,
                        color: text.color.0,
                        clip: text.clip,
                        order: text.order,
                    });
                }
            }
            self.buffers.insert(buffer_key, (buffer, self.frame));
        }
        if self.frame.is_multiple_of(120) {
            let frame = self.frame;
            self.buffers
                .retain(|_, (_, used)| frame.saturating_sub(*used) <= 120);
        }

        let placements = self.atlas.prepare_glyphs(&pending_pixels);
        self.atlas_warning_pending |= self.atlas.take_overflowed();
        let (atlas_width, atlas_height) = self.atlas.dimensions();
        let mut output = PreparedText::default();
        for item in pending_positions {
            let Some(placement) = placements.get(&item.key).copied() else {
                continue;
            };
            let packed = placement.packed;
            if packed.w == 0 || packed.h == 0 {
                continue;
            }
            let x = (item.x + placement.left) as f32 / scale;
            let y = (item.y - placement.top) as f32 / scale;
            let w = packed.w as f32 / scale;
            let h = packed.h as f32 / scale;
            let u0 = packed.x as f32 / atlas_width as f32;
            let v0 = packed.y as f32 / atlas_height as f32;
            let u1 = (packed.x + packed.w) as f32 / atlas_width as f32;
            let v1 = (packed.y + packed.h) as f32 / atlas_height as f32;
            let start = output.vertices.len() as u32;
            let p0 = [x, y];
            let p1 = [x + w, y];
            let p2 = [x + w, y + h];
            let p3 = [x, y + h];
            for (position, uv) in [
                (p0, [u0, v0]),
                (p1, [u1, v0]),
                (p2, [u1, v1]),
                (p0, [u0, v0]),
                (p2, [u1, v1]),
                (p3, [u0, v1]),
            ] {
                output.vertices.push(UiVertex {
                    position,
                    uv,
                    color: item.color,
                    kind: 1,
                    local: [uv[0], uv[1]],
                    rect_size: [w, h],
                    radius: 0.0,
                    border: 0.0,
                });
            }
            let end = output.vertices.len() as u32;
            if let Some(last) = output.batches.last_mut() {
                if last.clip == item.clip
                    && last.order == item.order
                    && last.vertex_range.end == start
                {
                    last.vertex_range.end = end;
                    continue;
                }
            }
            output.batches.push(DrawBatch {
                clip: item.clip,
                vertex_range: start..end,
                order: item.order,
            });
        }
        output
    }

    /// Returns true when a glyph-atlas overflow warning is due. Repeated reports
    /// are rate-limited to one every ten seconds.
    pub(crate) fn take_atlas_overflow_warning(&mut self) -> bool {
        if !self.atlas_warning_pending {
            return false;
        }
        let now = Instant::now();
        if self
            .last_atlas_warning
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(10))
        {
            return false;
        }
        self.last_atlas_warning = Some(now);
        self.atlas_warning_pending = false;
        true
    }

    pub(crate) fn atlas_dimensions(&self) -> (u32, u32) {
        self.atlas.dimensions()
    }

    pub(crate) fn atlas_snapshot(&self) -> Vec<u8> {
        self.atlas.snapshot()
    }

    pub(crate) fn take_atlas_updates(&mut self) -> (Option<Vec<u8>>, Vec<atlas::AtlasRegion>) {
        self.atlas.take_updates()
    }
}

fn finite_size(size: f32) -> f32 {
    if size.is_finite() {
        size.clamp(1.0, 256.0)
    } else {
        16.0
    }
}
fn finite_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(1.0, 16_384.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_arabic_and_measures_latin() {
        let mut text = TextSystem::new();
        assert!(text.measure("Ribbon", 18.0, 300.0).0 > 0.0);
        let arabic = text.layout("مرحبا", 18.0, 300.0, TextDirection::Auto);
        assert!(arabic.is_rtl);
        assert!(!arabic.glyphs.is_empty());
    }

    #[test]
    fn caret_mapping_returns_utf8_boundaries() {
        let mut text_system = TextSystem::new();
        let text = "Aéب";
        let x = text_system.caret_x_for_byte(text, 18.0, "Aé".len());
        let byte = text_system.byte_for_x(text, 18.0, x);
        assert!(text.is_char_boundary(byte));
    }

    #[test]
    fn rtl_caret_hit_testing_returns_valid_text_boundaries() {
        let mut text_system = TextSystem::new();
        let text = "مرحبا";
        let layout = text_system.layout(text, 18.0, 300.0, TextDirection::Auto);
        assert!(layout.is_rtl);
        let start_x = text_system.caret_x_for_byte(text, 18.0, 0);
        let end_x = text_system.caret_x_for_byte(text, 18.0, text.len());
        assert!(
            start_x > end_x,
            "RTL logical start should be on the visual right"
        );
        assert_eq!(text_system.byte_for_x(text, 18.0, start_x), 0);
        assert_eq!(text_system.byte_for_x(text, 18.0, end_x), text.len());
        for (byte, _) in text.char_indices() {
            let x = text_system.caret_x_for_byte(text, 18.0, byte);
            let hit = text_system.byte_for_x(text, 18.0, x);
            assert!(text.is_char_boundary(hit));
        }
    }

    #[test]
    fn arabic_and_latin_text_generate_atlas_vertices_and_r8_pixels() {
        let mut system = TextSystem::new();
        let mut draw = crate::UiDrawList::default();
        draw.text(
            "Ribbon مرحبا",
            crate::Rect {
                x: 8.0,
                y: 12.0,
                w: 280.0,
                h: 32.0,
            },
            18.0,
            crate::Color([1.0; 4]),
            TextDirection::Auto,
        );
        let prepared = system.prepare(&draw.texts, 1.0);
        assert!(!prepared.vertices.is_empty());
        assert!(!prepared.batches.is_empty());
        assert!(system.atlas_snapshot().iter().any(|pixel| *pixel > 0));
        assert_eq!(prepared.vertices.len() % 6, 0);
    }
}
