use std::error::Error;
use std::io;

use studs::app::StudsRendererMode;

fn main() -> Result<(), Box<dyn Error>> {
    let mut backend = if cfg!(feature = "gpu") {
        StudsRendererMode::Gpu
    } else {
        StudsRendererMode::Cpu
    };
    let mut seed = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--renderer=") {
            backend = parse_backend(value)?;
        } else if arg == "--renderer" {
            let value = args.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--renderer requires cpu, gpu, or auto",
                )
            })?;
            backend = parse_backend(&value)?;
        } else if let Some(value) = arg.strip_prefix("--seed=") {
            seed = Some(value.parse::<u32>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--seed must be an unsigned 32-bit integer",
                )
            })?);
        } else if arg == "--seed" {
            let value = args.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--seed requires an unsigned integer",
                )
            })?;
            seed = Some(value.parse::<u32>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--seed must be an unsigned 32-bit integer",
                )
            })?);
        } else if arg == "--help" || arg == "-h" {
            println!(
                "STUDS\n\nOptions:\n  --renderer cpu|gpu|auto  Select rendering backend (default: gpu when compiled in)\n  --seed <u32>             Use a deterministic maze seed\n  -h, --help               Show this help"
            );
            return Ok(());
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown argument: {arg} (use --help)"),
            )
            .into());
        }
    }

    studs::app::build_app_with_renderer(seed, false, backend)?.run();
    Ok(())
}

fn parse_backend(value: &str) -> Result<StudsRendererMode, io::Error> {
    match value.to_ascii_lowercase().as_str() {
        "cpu" => Ok(StudsRendererMode::Cpu),
        "gpu" | "auto" => Ok(StudsRendererMode::Gpu),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid renderer '{value}'; expected cpu, gpu, or auto"),
        )),
    }
}

#[cfg(test)]
mod renderer_argument_tests {
    use super::{parse_backend, StudsRendererMode};

    #[test]
    fn cpu_argument_selects_cpu_backend() {
        assert_eq!(parse_backend("cpu").unwrap(), StudsRendererMode::Cpu);
        assert_eq!(parse_backend("CPU").unwrap(), StudsRendererMode::Cpu);
    }

    #[test]
    fn gpu_and_auto_arguments_select_the_game_mesh_renderer_mode() {
        assert_eq!(parse_backend("gpu").unwrap(), StudsRendererMode::Gpu);
        assert_eq!(parse_backend("auto").unwrap(), StudsRendererMode::Gpu);
        assert_eq!(parse_backend("AuTo").unwrap(), StudsRendererMode::Gpu);
    }

    #[test]
    fn invalid_renderer_argument_is_rejected_with_expected_choices() {
        let error = parse_backend("raycaster").expect_err("invalid renderer must be rejected");
        assert!(error.to_string().contains("cpu, gpu, or auto"));
    }
}
