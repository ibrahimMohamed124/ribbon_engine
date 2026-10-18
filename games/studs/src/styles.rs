use plugin_rendering::{Palette, RenderStyle, Tonemap, RENDER_DIST};

pub fn classic() -> RenderStyle {
    RenderStyle::default()
}

pub fn moody() -> RenderStyle {
    RenderStyle {
        tonemap: Tonemap::Exp { exposure: 2.0 },
        gamma: 1.6,
        palette: Palette::ramp(0x05_08_06, 0xcf_e3_d4),
        texture_strength: 0.6,
        ao_strength: 0.45,
        ambient: 0.010,
        ambient_fill: 0.12,
        fog_distance: RENDER_DIST,
        pbr_enabled: false,
        billboards_enabled: false,
        limit_flicker: false,
        spot_light_shadows: false,
    }
}

pub fn realistic() -> RenderStyle {
    RenderStyle {
        tonemap: Tonemap::Exp { exposure: 1.45 },
        gamma: 1.0,
        palette: Palette::ramp(0x02_0b_0b, 0xb9_c1_b8),
        texture_strength: 0.0,
        ao_strength: 0.72,
        ambient: 0.006,
        ambient_fill: 0.16,
        fog_distance: RENDER_DIST * 0.82,
        pbr_enabled: true,
        billboards_enabled: true,
        limit_flicker: false,
        spot_light_shadows: true,
    }
}
