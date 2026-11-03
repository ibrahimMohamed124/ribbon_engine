//! R8 glyph-atlas packing and dirty-region extraction.
use cosmic_text::CacheKey;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedGlyph {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphPlacement {
    pub packed: PackedGlyph,
    pub left: i32,
    pub top: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingGlyph {
    pub key: CacheKey,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub alpha: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct AtlasRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct GlyphAtlas {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    row: u32,
    pixels: Vec<u8>,
    glyphs: HashMap<CacheKey, GlyphPlacement>,
    dirty: Vec<PackedGlyph>,
    full_dirty: bool,
    grew: bool,
    overflowed: bool,
}
impl Default for GlyphAtlas {
    fn default() -> Self {
        Self {
            width: 1024,
            height: 1024,
            x: 0,
            y: 0,
            row: 0,
            pixels: vec![0; 1024 * 1024],
            glyphs: HashMap::new(),
            dirty: Vec::new(),
            full_dirty: true,
            grew: false,
            overflowed: false,
        }
    }
}

impl GlyphAtlas {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    #[cfg(test)]
    fn pack(&mut self, width: u32, height: u32) -> Option<PackedGlyph> {
        self.pack_with_padding(width, height, 0)
    }

    fn pack_with_padding(&mut self, width: u32, height: u32, padding: u32) -> Option<PackedGlyph> {
        let padded_width = width.checked_add(padding.saturating_mul(2))?;
        let padded_height = height.checked_add(padding.saturating_mul(2))?;
        if padded_width == 0
            || padded_height == 0
            || padded_width > self.width
            || padded_height > self.height
        {
            return None;
        }
        let mut x = self.x;
        let mut y = self.y;
        let mut row = self.row;
        if x.saturating_add(padded_width) > self.width {
            x = 0;
            y = y.saturating_add(row);
            row = 0;
        }
        if y.saturating_add(padded_height) > self.height {
            return None;
        }
        let packed = PackedGlyph {
            x: x + padding,
            y: y + padding,
            w: width,
            h: height,
        };
        self.x = x + padded_width;
        self.y = y;
        self.row = row.max(padded_height);
        Some(packed)
    }

    pub(crate) fn prepare_glyphs(
        &mut self,
        pending: &[PendingGlyph],
    ) -> HashMap<CacheKey, GlyphPlacement> {
        let mut unique = Vec::with_capacity(pending.len());
        let mut seen = HashSet::with_capacity(pending.len());
        for glyph in pending {
            if glyph.width > 0 && glyph.height > 0 && seen.insert(glyph.key) {
                unique.push(glyph);
            }
        }

        let mut failed = false;
        for glyph in &unique {
            if self.glyphs.contains_key(&glyph.key) {
                continue;
            }
            if self.insert_pending(glyph).is_none() {
                failed = true;
                break;
            }
        }
        if failed {
            if !self.grew {
                self.grow_to_2048();
                let mut retry_failed = false;
                for glyph in &unique {
                    if !self.glyphs.contains_key(&glyph.key) && self.insert_pending(glyph).is_none()
                    {
                        retry_failed = true;
                        break;
                    }
                }
                if retry_failed {
                    self.reset_at_current_size();
                    for glyph in &unique {
                        self.overflowed |= self.insert_pending(glyph).is_none();
                    }
                }
            } else {
                self.reset_at_current_size();
                for glyph in &unique {
                    self.overflowed |= self.insert_pending(glyph).is_none();
                }
            }
        }
        unique
            .iter()
            .filter_map(|glyph| self.glyphs.get(&glyph.key).copied().map(|v| (glyph.key, v)))
            .collect()
    }

    fn insert_pending(&mut self, glyph: &PendingGlyph) -> Option<GlyphPlacement> {
        let expected = (glyph.width as usize).saturating_mul(glyph.height as usize);
        if glyph.alpha.len() < expected {
            return None;
        }
        let packed = self.pack_with_padding(glyph.width, glyph.height, 1)?;
        for row in 0..glyph.height as usize {
            let src_start = row * glyph.width as usize;
            let dst_start = (packed.y as usize + row) * self.width as usize + packed.x as usize;
            self.pixels[dst_start..dst_start + glyph.width as usize]
                .copy_from_slice(&glyph.alpha[src_start..src_start + glyph.width as usize]);
        }
        self.glyphs.insert(
            glyph.key,
            GlyphPlacement {
                packed,
                left: glyph.left,
                top: glyph.top,
            },
        );
        self.dirty.push(PackedGlyph {
            x: packed.x - 1,
            y: packed.y - 1,
            w: packed.w + 2,
            h: packed.h + 2,
        });
        Some(GlyphPlacement {
            packed,
            left: glyph.left,
            top: glyph.top,
        })
    }

    fn grow_to_2048(&mut self) {
        if self.width >= 2048 && self.height >= 2048 {
            return;
        }
        let mut grown = vec![0; 2048 * 2048];
        for row in 0..self.height as usize {
            let old = row * self.width as usize;
            let new = row * 2048;
            grown[new..new + self.width as usize]
                .copy_from_slice(&self.pixels[old..old + self.width as usize]);
        }
        self.pixels = grown;
        self.width = 2048;
        self.height = 2048;
        self.grew = true;
        self.full_dirty = true;
        self.dirty.clear();
    }

    fn reset_at_current_size(&mut self) {
        self.pixels.fill(0);
        self.glyphs.clear();
        self.x = 0;
        self.y = 0;
        self.row = 0;
        self.dirty.clear();
        self.full_dirty = true;
    }

    pub(crate) fn snapshot(&self) -> Vec<u8> {
        self.pixels.clone()
    }

    pub(crate) fn take_overflowed(&mut self) -> bool {
        std::mem::take(&mut self.overflowed)
    }

    pub(crate) fn take_updates(&mut self) -> (Option<Vec<u8>>, Vec<AtlasRegion>) {
        if self.full_dirty {
            self.full_dirty = false;
            self.dirty.clear();
            return (Some(self.pixels.clone()), Vec::new());
        }
        let mut regions = Vec::with_capacity(self.dirty.len());
        for rect in self.dirty.drain(..) {
            let mut pixels = Vec::with_capacity((rect.w * rect.h) as usize);
            for row in 0..rect.h as usize {
                let start = (rect.y as usize + row) * self.width as usize + rect.x as usize;
                pixels.extend_from_slice(&self.pixels[start..start + rect.w as usize]);
            }
            regions.push(AtlasRegion {
                x: rect.x,
                y: rect.y,
                width: rect.w,
                height: rect.h,
                pixels,
            });
        }
        (None, regions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_atlas_has_a_valid_initial_r8_surface() {
        let atlas = GlyphAtlas::default();
        assert_eq!(atlas.dimensions(), (1024, 1024));
        assert_eq!(atlas.pixels.len(), 1024 * 1024);
        assert!(atlas.full_dirty);
    }

    #[test]
    fn shelf_packing_produces_non_overlapping_regions() {
        let mut atlas = GlyphAtlas::new();
        let a = atlas.pack(500, 500).expect("first region");
        let b = atlas.pack(500, 500).expect("second region");
        assert!(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y);
        assert!(b.x + b.w <= 1024 && b.y + b.h <= 1024);
    }

    #[test]
    fn atlas_growth_preserves_contents_and_requests_a_full_upload() {
        let mut atlas = GlyphAtlas::new();
        let first = atlas.pack(16, 16).expect("initial glyph rectangle");
        let index = (first.y as usize * atlas.width as usize) + first.x as usize;
        atlas.pixels[index] = 173;
        atlas.grow_to_2048();
        assert_eq!(atlas.dimensions(), (2048, 2048));
        let grown_index = (first.y as usize * atlas.width as usize) + first.x as usize;
        assert_eq!(atlas.pixels[grown_index], 173);
        let (full, regions) = atlas.take_updates();
        assert!(full.is_some());
        assert!(regions.is_empty());
    }
}
