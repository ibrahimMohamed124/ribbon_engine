//! UI colors and default dark theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub [f32; 4]);
#[derive(Clone, Debug)]
pub struct Theme {
    pub background: Color,
    pub panel: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub focus: Color,
    pub font_size: f32,
    pub heading_size: f32,
    pub padding: f32,
    pub spacing: f32,
    pub corner_radius: f32,
}
impl Theme {
    pub fn dark() -> Self {
        Self {
            background: Color([0.025, 0.035, 0.055, 0.96]),
            panel: Color([0.065, 0.085, 0.12, 0.94]),
            text: Color([0.93, 0.95, 0.98, 1.0]),
            muted: Color([0.60, 0.66, 0.75, 1.0]),
            accent: Color([0.28, 0.66, 0.92, 1.0]),
            focus: Color([0.45, 0.84, 1.0, 1.0]),
            font_size: 16.0,
            heading_size: 26.0,
            padding: 12.0,
            spacing: 8.0,
            corner_radius: 5.0,
        }
    }
}
impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct WidgetStyle {
    pub text: Option<Color>,
    pub fill: Option<Color>,
}
