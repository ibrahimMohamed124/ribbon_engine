//! Deterministic bundled-font text shaping and measurement.
use std::collections::HashMap;
pub struct TextSystem {
    fonts: cosmic_text::FontSystem,
    cache: HashMap<(String, u32, u32), (f32, f32, bool)>,
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
        Self {
            fonts: cosmic_text::FontSystem::new_with_fonts(sources),
            cache: HashMap::new(),
        }
    }
    pub fn measure(&mut self, text: &str, size: f32, max_width: f32) -> (f32, f32) {
        let (w, h, _) = self.layout(text, size, max_width, crate::TextDirection::Auto);
        (w, h)
    }
    pub fn layout(
        &mut self,
        text: &str,
        size: f32,
        max_width: f32,
        _dir: crate::TextDirection,
    ) -> (f32, f32, bool) {
        let key = (text.to_owned(), (size * 2.) as u32, max_width as u32);
        if let Some(v) = self.cache.get(&key) {
            return *v;
        }
        use cosmic_text::{Attrs, Buffer, Metrics, Shaping};
        let mut b = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        b.set_size(&mut self.fonts, Some(max_width), None);
        b.set_text(&mut self.fonts, text, Attrs::new(), Shaping::Advanced);
        b.shape_until_scroll(&mut self.fonts, false);
        let lines = b.layout_runs().collect::<Vec<_>>();
        let w = lines.iter().map(|l| l.line_w).fold(0.0f32, f32::max);
        let h = lines.iter().map(|l| l.line_height).sum();
        let rtl = text
            .chars()
            .find(|c| c.is_alphabetic())
            .is_some_and(|c| matches!(c as u32, 0x0590..=0x08ff));
        let v = (w, h, rtl);
        self.cache.insert(key, v);
        v
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shapes_arabic_and_measures_latin() {
        let mut t = TextSystem::new();
        assert!(t.measure("Ribbon", 18., 300.).0 > 0.);
        assert!(t.layout("مرحبا", 18., 300., crate::TextDirection::Auto).2);
    }
}
