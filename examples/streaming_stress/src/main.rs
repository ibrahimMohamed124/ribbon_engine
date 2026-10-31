//! Ribbon engine streaming stress harness.
//!
//! This is an engine example, not a game. The headless phase exercises real AssetServer,
//! StreamingCoordinator, ECS, and Rapier lifecycles. The GPU phase opens a small native window
//! and exercises the real render-thread model/texture upload path and its per-frame budgets.

mod gpu;
mod headless;
mod report;

use std::error::Error;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub(crate) mode: Mode,
    pub(crate) regions: usize,
    pub(crate) cycles: usize,
    pub(crate) asset_count: usize,
    pub(crate) asset_bytes_kib: usize,
    pub(crate) frames_per_region: usize,
    pub(crate) tail_frames: usize,
    pub(crate) texture_size: usize,
    pub(crate) uploads_per_frame: usize,
    pub(crate) upload_budget_mib: usize,
    pub(crate) asset_root: PathBuf,
    pub(crate) output_dir: PathBuf,
    pub(crate) require_gpu: bool,
    pub(crate) expect_gpu_pressure: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Headless,
    Gpu,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Headless,
            regions: 48,
            cycles: 2,
            asset_count: 64,
            asset_bytes_kib: 256,
            frames_per_region: 24,
            tail_frames: 120,
            texture_size: 512,
            uploads_per_frame: 2,
            upload_budget_mib: 2,
            asset_root: PathBuf::from("target/streaming-stress-corpus"),
            output_dir: PathBuf::from("target/streaming-stress"),
            require_gpu: false,
            expect_gpu_pressure: false,
        }
    }
}

impl Options {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut options = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let value = |args: &mut std::iter::Skip<std::env::Args>, name: &str| {
                args.next().ok_or_else(|| format!("{name} expects a value"))
            };
            match arg.as_str() {
                "--mode" => {
                    options.mode = match value(&mut args, "--mode")?.as_str() {
                        "headless" => Mode::Headless,
                        "gpu" => Mode::Gpu,
                        other => {
                            return Err(
                                format!("unknown mode `{other}`; use headless or gpu").into()
                            )
                        }
                    };
                }
                "--regions" => options.regions = value(&mut args, "--regions")?.parse()?,
                "--cycles" => options.cycles = value(&mut args, "--cycles")?.parse()?,
                "--assets" => options.asset_count = value(&mut args, "--assets")?.parse()?,
                "--asset-bytes-kib" => {
                    options.asset_bytes_kib = value(&mut args, "--asset-bytes-kib")?.parse()?
                }
                "--frames-per-region" => {
                    options.frames_per_region = value(&mut args, "--frames-per-region")?.parse()?
                }
                "--tail-frames" => {
                    options.tail_frames = value(&mut args, "--tail-frames")?.parse()?
                }
                "--texture-size" => {
                    options.texture_size = value(&mut args, "--texture-size")?.parse()?
                }
                "--uploads-per-frame" => {
                    options.uploads_per_frame = value(&mut args, "--uploads-per-frame")?.parse()?
                }
                "--upload-budget-mib" => {
                    options.upload_budget_mib = value(&mut args, "--upload-budget-mib")?.parse()?
                }
                "--asset-root" => {
                    options.asset_root = PathBuf::from(value(&mut args, "--asset-root")?)
                }
                "--output-dir" => {
                    options.output_dir = PathBuf::from(value(&mut args, "--output-dir")?)
                }
                "--require-gpu" => options.require_gpu = true,
                "--expect-gpu-pressure" => options.expect_gpu_pressure = true,
                "--help" | "-h" => {
                    println!(
                        "ribbon_streaming_stress\n\n\
                         --mode headless|gpu         Run deterministic CPU streaming or real GPU window tests\n\
                         --regions N                 Number of generated regions (2..=512)\n\
                         --cycles N                  Number of forward/backward route cycles (1..=20)\n\
                         --assets N                  Number of GLB assets in generated corpus (2..=512)\n\
                         --asset-bytes-kib N         Headless decoded asset payload size per asset\n\
                         --frames-per-region N       GPU dwell frames for each route region\n\
                         --tail-frames N             GPU frames reserved for final unload/settle (48..=10000)\n\
                         --texture-size N            GPU stress texture side (power of two)\n\
                         --uploads-per-frame N       GPU upload operation budget per frame\n\
                         --upload-budget-mib N       Estimated upload byte budget per frame\n\
                         --asset-root PATH           Generated GLB root\n\
                         --output-dir PATH           CSV/Markdown output folder\n\
                         --require-gpu               Fail when no actual rendered GPU frames/uploads occur\n\
\
                         --expect-gpu-pressure        Reject pinned overcommit and evict a READY GPU texture\n\
\
                         Examples:\n\
                         cargo run -p ribbon_streaming_stress -- --mode headless --regions 96 --cycles 3\n\
                         cargo run -p ribbon_streaming_stress --release -- --mode gpu --regions 48 --cycles 2 --asset-root target/streaming-stress-corpus --require-gpu --expect-gpu-pressure"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument `{other}`; use --help").into()),
            }
        }
        if !(2..=512).contains(&options.regions) {
            return Err("--regions must be between 2 and 512".into());
        }
        if !(1..=20).contains(&options.cycles) {
            return Err("--cycles must be between 1 and 20".into());
        }
        if !(2..=512).contains(&options.asset_count) {
            return Err("--assets must be between 2 and 512".into());
        }
        if options.expect_gpu_pressure && options.mode != Mode::Gpu {
            return Err("--expect-gpu-pressure requires --mode gpu".into());
        }
        if options.asset_bytes_kib == 0 || options.asset_bytes_kib > 16 * 1024 {
            return Err("--asset-bytes-kib must be between 1 and 16384".into());
        }
        if options.frames_per_region == 0 || options.frames_per_region > 600 {
            return Err("--frames-per-region must be between 1 and 600".into());
        }
        if !(48..=10_000).contains(&options.tail_frames) {
            return Err(
                "--tail-frames must be between 48 and 10000 so unload/physics cleanup can settle"
                    .into(),
            );
        }
        if !(8..=512).contains(&options.texture_size) || !options.texture_size.is_power_of_two() {
            return Err("--texture-size must be a power of two between 8 and 512".into());
        }
        if options.uploads_per_frame == 0 || options.uploads_per_frame > 64 {
            return Err("--uploads-per-frame must be between 1 and 64".into());
        }
        if options.upload_budget_mib == 0 || options.upload_budget_mib > 4096 {
            return Err("--upload-budget-mib must be between 1 and 4096".into());
        }
        Ok(options)
    }
}

fn main() {
    let result = run();
    if let Err(error) = result {
        eprintln!("[STREAMING STRESS] FAIL: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = Options::parse()?;
    std::fs::create_dir_all(&options.output_dir)?;
    match options.mode {
        Mode::Headless => headless::run(&options),
        Mode::Gpu => gpu::run(&options),
    }
}
