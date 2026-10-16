use blocks_engine::render::lighting::light_with_style;
use blocks_engine::render::{RenderStyle, RENDER_DIST};

struct Sample {
    name: &'static str,
    classic: f32,
    before: f32,
    moody: f32,
    target: &'static str,
    pass: bool,
}

fn main() {
    let classic = RenderStyle::classic();
    let moody = RenderStyle::moody();
    let mut before = RenderStyle::moody();
    before.tonemap = blocks_engine::render::Tonemap::Exp { exposure: 1.15 };
    before.gamma = 2.2;
    before.ambient = 0.035;
    before.texture_strength = 0.55;
    let surface = |style: &RenderStyle, gray: f32, distance: f32, flashlight: f32| {
        let illumination = light_with_style(flashlight, distance, 1.0, flashlight, style);
        style.level_for(gray * illumination)
    };
    let light_ratio =
        |style: &RenderStyle| surface(style, 0.55, 3.0, 1.0) / surface(style, 0.55, 3.0, 0.0);
    let emitter = |style: &RenderStyle, gray: f32, distance: f32| {
        style.level_for(gray * (1.0 - distance / (RENDER_DIST * 1.4)))
    };
    let c = |gray, distance, beam| surface(&classic, gray, distance, beam);
    let b = |gray, distance, beam| surface(&before, gray, distance, beam);
    let m = |gray, distance, beam| surface(&moody, gray, distance, beam);
    let samples = [
        Sample {
            name: "wall dark, d=1",
            classic: c(0.55, 1.0, 0.0),
            before: b(0.55, 1.0, 0.0),
            moody: m(0.55, 1.0, 0.0),
            target: "2.0..3.0",
            pass: (2.0..=3.0).contains(&m(0.55, 1.0, 0.0)),
        },
        Sample {
            name: "wall dark, d=3",
            classic: c(0.55, 3.0, 0.0),
            before: b(0.55, 3.0, 0.0),
            moody: m(0.55, 3.0, 0.0),
            target: "0.8..1.6",
            pass: (0.8..=1.6).contains(&m(0.55, 3.0, 0.0)),
        },
        Sample {
            name: "wall dark, d=12",
            classic: c(0.55, 12.0, 0.0),
            before: b(0.55, 12.0, 0.0),
            moody: m(0.55, 12.0, 0.0),
            target: "<= 0.6",
            pass: m(0.55, 12.0, 0.0) <= 0.6,
        },
        Sample {
            name: "wall beam, d=3",
            classic: c(0.55, 3.0, 1.0),
            before: b(0.55, 3.0, 1.0),
            moody: m(0.55, 3.0, 1.0),
            target: "8.0..10.0",
            pass: (8.0..=10.0).contains(&m(0.55, 3.0, 1.0)),
        },
        Sample {
            name: "wall beam, d=6",
            classic: c(0.55, 6.0, 1.0),
            before: b(0.55, 6.0, 1.0),
            moody: m(0.55, 6.0, 1.0),
            target: "5.0..7.0",
            pass: (5.0..=7.0).contains(&m(0.55, 6.0, 1.0)),
        },
        Sample {
            name: "enemy body dark, d=3",
            classic: c(0.10, 3.0, 0.0),
            before: b(0.10, 3.0, 0.0),
            moody: m(0.10, 3.0, 0.0),
            target: "<= 0.6",
            pass: m(0.10, 3.0, 0.0) <= 0.6,
        },
        Sample {
            name: "enemy body beam, d=3",
            classic: c(0.10, 3.0, 1.0),
            before: b(0.10, 3.0, 1.0),
            moody: m(0.10, 3.0, 1.0),
            target: "3.0..4.5",
            pass: (3.0..=4.5).contains(&m(0.10, 3.0, 1.0)),
        },
        Sample {
            name: "enemy eyes, d=4",
            classic: emitter(&classic, 1.0, 4.0),
            before: emitter(&before, 1.0, 4.0),
            moody: emitter(&moody, 1.0, 4.0),
            target: ">= 11.0",
            pass: emitter(&moody, 1.0, 4.0) >= 11.0,
        },
        Sample {
            name: "page, d=6",
            classic: emitter(&classic, 0.92, 6.0),
            before: emitter(&before, 0.92, 6.0),
            moody: emitter(&moody, 0.92, 6.0),
            target: ">= 10.5",
            pass: emitter(&moody, 0.92, 6.0) >= 10.5,
        },
        Sample {
            name: "beam/dark contrast, d=3",
            classic: light_ratio(&classic),
            before: light_ratio(&before),
            moody: light_ratio(&moody),
            target: ">= 6:1",
            pass: light_ratio(&moody) >= 6.0,
        },
        Sample {
            name: "upper palette, gray=1.7",
            classic: classic.level_for(1.7),
            before: before.level_for(1.7),
            moody: moody.level_for(1.7),
            target: ">= 12.0",
            pass: moody.level_for(1.7) >= 12.0,
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
            sample.classic,
            sample.before,
            sample.moody,
            sample.target,
            if sample.pass { "PASS" } else { "FAIL" }
        );
    }
    println!("Moody constants: exposure=2.0, gamma=1.6, ambient=0.010, lamp=0.12, fog=18.0, texture=0.60, AO=0.45");
    if !all_pass {
        std::process::exit(1);
    }
}
