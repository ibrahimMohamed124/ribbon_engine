use voxelworks_render_cpu::render::lighting::light_with_style;
use voxelworks_render_cpu::render::{Palette, RenderStyle, Tonemap, RENDER_DIST};

fn graded_style() -> RenderStyle {
    RenderStyle {
        tonemap: Tonemap::Exp { exposure: 2.0 },
        gamma: 1.6,
        palette: Palette::ramp(0x05_08_06, 0xcf_e3_d4),
        texture_strength: 0.6,
        ao_strength: 0.45,
        ambient: 0.010,
        ..RenderStyle::default()
    }
}

struct Sample {
    name: &'static str,
    baseline: f32,
    before: f32,
    graded: f32,
    target: &'static str,
    pass: bool,
}

fn main() {
    let baseline = RenderStyle::default();
    let graded = graded_style();
    let mut before = graded_style();
    before.tonemap = voxelworks_render_cpu::render::Tonemap::Exp { exposure: 1.15 };
    before.gamma = 2.2;
    before.ambient = 0.035;
    before.texture_strength = 0.55;
    let surface = |style: &RenderStyle, gray: f32, distance: f32, directed_source: f32| {
        let light_level = light_with_style(directed_source, distance, 1.0, directed_source, style);
        style.level_for(gray * light_level)
    };
    let light_ratio =
        |style: &RenderStyle| surface(style, 0.55, 3.0, 1.0) / surface(style, 0.55, 3.0, 0.0);
    let emitter = |style: &RenderStyle, gray: f32, distance: f32| {
        style.level_for(gray * (1.0 - distance / (RENDER_DIST * 1.4)))
    };
    let c = |gray, distance, beam| surface(&baseline, gray, distance, beam);
    let b = |gray, distance, beam| surface(&before, gray, distance, beam);
    let m = |gray, distance, beam| surface(&graded, gray, distance, beam);
    let samples = [
        Sample {
            name: "wall dark, d=1",
            baseline: c(0.55, 1.0, 0.0),
            before: b(0.55, 1.0, 0.0),
            graded: m(0.55, 1.0, 0.0),
            target: "2.0..3.0",
            pass: (2.0..=3.0).contains(&m(0.55, 1.0, 0.0)),
        },
        Sample {
            name: "wall dark, d=3",
            baseline: c(0.55, 3.0, 0.0),
            before: b(0.55, 3.0, 0.0),
            graded: m(0.55, 3.0, 0.0),
            target: "0.8..1.6",
            pass: (0.8..=1.6).contains(&m(0.55, 3.0, 0.0)),
        },
        Sample {
            name: "wall dark, d=12",
            baseline: c(0.55, 12.0, 0.0),
            before: b(0.55, 12.0, 0.0),
            graded: m(0.55, 12.0, 0.0),
            target: "<= 0.6",
            pass: m(0.55, 12.0, 0.0) <= 0.6,
        },
        Sample {
            name: "wall beam, d=3",
            baseline: c(0.55, 3.0, 1.0),
            before: b(0.55, 3.0, 1.0),
            graded: m(0.55, 3.0, 1.0),
            target: "8.0..10.0",
            pass: (8.0..=10.0).contains(&m(0.55, 3.0, 1.0)),
        },
        Sample {
            name: "wall beam, d=6",
            baseline: c(0.55, 6.0, 1.0),
            before: b(0.55, 6.0, 1.0),
            graded: m(0.55, 6.0, 1.0),
            target: "5.0..7.0",
            pass: (5.0..=7.0).contains(&m(0.55, 6.0, 1.0)),
        },
        Sample {
            name: "actor body dark, d=3",
            baseline: c(0.10, 3.0, 0.0),
            before: b(0.10, 3.0, 0.0),
            graded: m(0.10, 3.0, 0.0),
            target: "<= 0.6",
            pass: m(0.10, 3.0, 0.0) <= 0.6,
        },
        Sample {
            name: "actor body beam, d=3",
            baseline: c(0.10, 3.0, 1.0),
            before: b(0.10, 3.0, 1.0),
            graded: m(0.10, 3.0, 1.0),
            target: "3.0..4.5",
            pass: (3.0..=4.5).contains(&m(0.10, 3.0, 1.0)),
        },
        Sample {
            name: "actor emitters, d=4",
            baseline: emitter(&baseline, 1.0, 4.0),
            before: emitter(&before, 1.0, 4.0),
            graded: emitter(&graded, 1.0, 4.0),
            target: ">= 11.0",
            pass: emitter(&graded, 1.0, 4.0) >= 11.0,
        },
        Sample {
            name: "emitter, d=6",
            baseline: emitter(&baseline, 0.92, 6.0),
            before: emitter(&before, 0.92, 6.0),
            graded: emitter(&graded, 0.92, 6.0),
            target: ">= 10.5",
            pass: emitter(&graded, 0.92, 6.0) >= 10.5,
        },
        Sample {
            name: "beam/dark contrast, d=3",
            baseline: light_ratio(&baseline),
            before: light_ratio(&before),
            graded: light_ratio(&graded),
            target: ">= 6:1",
            pass: light_ratio(&graded) >= 6.0,
        },
        Sample {
            name: "upper palette, gray=1.7",
            baseline: baseline.level_for(1.7),
            before: before.level_for(1.7),
            graded: graded.level_for(1.7),
            target: ">= 12.0",
            pass: graded.level_for(1.7) >= 12.0,
        },
    ];

    println!(
        "{:<29} {:>9} {:>10} {:>10} {:>12} {:>7}",
        "Reference surface", "Classic", "Before", "After", "After target", "Result"
    );
    let mut all_pass = true;
    for sample in samples {
        all_pass &= sample.pass;
        println!(
            "{:<29} {:>9.3} {:>10.3} {:>10.3} {:>12} {:>7}",
            sample.name,
            sample.baseline,
            sample.before,
            sample.graded,
            sample.target,
            if sample.pass { "PASS" } else { "FAIL" }
        );
    }
    println!("Moody constants: exposure=2.0, gamma=1.6, ambient=0.010, lamp=0.12, fog=18.0, texture=0.60, AO=0.45");
    if !all_pass {
        std::process::exit(1);
    }
}
