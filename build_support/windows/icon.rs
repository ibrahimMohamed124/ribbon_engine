// Shared Windows launcher-icon embedding for the workspace's executable packages.
//
// Runtime window icons are set by `ribbon_platform`; this build helper additionally
// embeds the same `.ico` as a PE resource so Explorer shortcuts and the executable file
// use the Ribbon Engine identity on Windows.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
    );
    let Some(workspace_root) = manifest_dir
        .ancestors()
        .find(|candidate| candidate.join("assets/engine_icon.ico").is_file())
    else {
        println!(
            "cargo:warning=Ribbon Engine launcher icon was not found; window icon still works at runtime"
        );
        return;
    };
    // This file is shared into each package's build.rs with include!(). Explicitly track
    // the included file itself: once a build script emits rerun-if-changed directives,
    // Cargo no longer watches the package directory automatically. Without this path,
    // updates to the shared helper can leave stale resource-compiler warnings/results.
    let helper_path = workspace_root.join("build_support/windows/icon.rs");
    println!("cargo:rerun-if-changed={}", helper_path.display());

    let icon_path = workspace_root.join("assets/engine_icon.ico");
    println!("cargo:rerun-if-changed={}", icon_path.display());
    println!("cargo:rerun-if-env-changed=WindowsSdkDir");
    println!("cargo:rerun-if-env-changed=WindowsSDKVersion");
    println!("cargo:rerun-if-env-changed=PATH");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let resource_source = out_dir.join("ribbon_engine_icon.rc");
    let resource_output = out_dir.join("ribbon_engine_icon.res");
    // Forward slashes are accepted by Microsoft's resource compiler and avoid quoting
    // issues with Windows path separators. Escaping quotes keeps unusual install paths safe.
    let icon_for_rc = icon_path
        .canonicalize()
        .unwrap_or(icon_path)
        .to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"");
    if let Err(error) = fs::write(&resource_source, format!("101 ICON \"{icon_for_rc}\"\r\n")) {
        println!("cargo:warning=Could not write Ribbon Engine icon resource script: {error}");
        return;
    }

    let mut compiled = compile_with_rc(
        Path::new("rc.exe"),
        &resource_source,
        &resource_output,
        &out_dir,
    );

    // `rc.exe` is installed by the Windows SDK but is commonly absent from PATH in a
    // regular PowerShell. Search the SDK installation/version directories before falling
    // back to other resource compilers.
    if !compiled {
        for candidate in windows_sdk_rc_candidates() {
            if compile_with_rc(&candidate, &resource_source, &resource_output, &out_dir) {
                compiled = true;
                break;
            }
        }
    }

    // LLVM's resource compiler accepts the MSVC-style command-line options as well.
    if !compiled {
        compiled = compile_with_rc(
            Path::new("llvm-rc.exe"),
            &resource_source,
            &resource_output,
            &out_dir,
        );
    }

    // GNU Windows toolchains commonly provide windres instead of rc.exe.
    if !compiled {
        compiled = matches!(
            Command::new("windres.exe")
                .arg("-i")
                .arg(&resource_source)
                .arg("-o")
                .arg(&resource_output)
                .current_dir(&out_dir)
                .status(),
            Ok(status) if status.success()
        );
    }

    if !compiled {
        println!(
            "cargo:warning=Could not compile Ribbon Engine .ico resource. Searched PATH and Windows SDK locations; install the Windows 10/11 SDK or add rc.exe/windres.exe to PATH. Runtime titlebar/taskbar icon remains enabled."
        );
        return;
    }

    // Apply the resource to binary targets only. `rustc-link-arg-examples` is rejected
    // by Cargo when a package has no explicit `[[example]]` targets; workspace packages
    // such as `voxel_room` are binaries themselves, not examples of their own package.
    // The resource is ignored for non-Windows targets above.
    println!("cargo:rustc-link-arg-bins={}", resource_output.display());
}

fn compile_with_rc(
    compiler: &Path,
    source: &Path,
    output: &Path,
    working_directory: &Path,
) -> bool {
    matches!(
        Command::new(compiler)
            .arg("/nologo")
            .arg("/fo")
            .arg(output)
            .arg(source)
            .current_dir(working_directory)
            .status(),
        Ok(status) if status.success()
    )
}

fn windows_sdk_rc_candidates() -> Vec<PathBuf> {
    let mut sdk_roots = Vec::new();
    if let Some(root) = env::var_os("WindowsSdkDir") {
        sdk_roots.push(PathBuf::from(root));
    }
    if let Some(program_files_x86) = env::var_os("ProgramFiles(x86)") {
        sdk_roots.push(PathBuf::from(program_files_x86).join("Windows Kits/10"));
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        sdk_roots.push(PathBuf::from(program_files).join("Windows Kits/10"));
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let architectures: &[&str] = match target_arch.as_str() {
        "x86_64" => &["x64", "x86", "arm64", "arm"],
        "x86" => &["x86", "x64", "arm", "arm64"],
        "aarch64" => &["arm64", "x64", "x86", "arm"],
        "arm" => &["arm", "arm64", "x86", "x64"],
        _ => &["x64", "x86", "arm64", "arm"],
    };
    let requested_version = env::var("WindowsSDKVersion")
        .ok()
        .map(|version| {
            version
                .trim()
                .trim_end_matches(['\\', '/'])
                .to_owned()
        });
    let mut candidates = Vec::new();

    for root in sdk_roots {
        let bin = if root.file_name().is_some_and(|name| name == "bin") {
            root
        } else {
            root.join("bin")
        };
        if !bin.is_dir() {
            continue;
        }

        let mut versions = Vec::new();
        if let Some(version) = requested_version.as_deref() {
            let version_path = bin.join(version);
            if version_path.is_dir() {
                versions.push(version_path);
            }
        }
        if let Ok(entries) = fs::read_dir(&bin) {
            let mut discovered = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            discovered.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
            for version in discovered {
                if !versions.contains(&version) {
                    versions.push(version);
                }
            }
        }

        for version in versions {
            for architecture in architectures {
                let candidate = version.join(architecture).join("rc.exe");
                if candidate.is_file() && !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
    }

    candidates
}
