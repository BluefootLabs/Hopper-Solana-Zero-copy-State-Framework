use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process;

use crate::workspace;

pub fn cmd_init(args: &[String]) {
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_init_usage();
        if args.is_empty() {
            process::exit(1);
        }
        return;
    }

    let mut destination = None;
    let mut crate_name = None;
    let mut local_path = None;
    let mut force = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                if i + 1 >= args.len() {
                    eprintln!("--name requires a crate name");
                    process::exit(1);
                }
                crate_name = Some(args[i + 1].clone());
                i += 2;
            }
            "--local-path" => {
                if i + 1 >= args.len() {
                    eprintln!("--local-path requires a path");
                    process::exit(1);
                }
                local_path = Some(args[i + 1].clone());
                i += 2;
            }
            "--force" => {
                force = true;
                i += 1;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown init flag: {other}");
                process::exit(1);
            }
            other => {
                if destination.is_some() {
                    eprintln!("Unexpected extra init argument: {other}");
                    process::exit(1);
                }
                destination = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }

    let destination = destination.unwrap_or_else(|| {
        eprintln!("Missing required <path> for hopper init");
        process::exit(1);
    });

    let inferred_name = crate_name.unwrap_or_else(|| infer_crate_name(&destination));
    let crate_name = normalize_crate_name(&inferred_name);

    if crate_name.is_empty() {
        eprintln!("Could not infer a valid Rust crate name from {}", destination.display());
        process::exit(1);
    }

    if let Err(err) = scaffold_project(&destination, &crate_name, local_path.as_deref(), force) {
        eprintln!("hopper init failed: {err}");
        process::exit(1);
    }

    println!("Initialized Hopper project at {}", destination.display());
    println!("Next steps:");
    println!("  cd {}", destination.display());
    println!("  hopper build --host");
    println!("  hopper test");
    println!("  hopper build");
}

pub fn cmd_build(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_build_usage();
        return;
    }

    let cwd = workspace::current_dir().unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    let project_root = workspace::find_project_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    let mut use_host = false;
    let mut cargo_args = Vec::new();
    for arg in args {
        if arg == "--host" {
            use_host = true;
        } else if arg == "--sbf" {
            use_host = false;
        } else {
            cargo_args.push(arg.clone());
        }
    }

    let mut command_args = vec![if use_host { "build" } else { "build-sbf" }.to_string()];
    command_args.extend(cargo_args);
    run_cargo_command(&project_root, &command_args);
}

pub fn cmd_test(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_test_usage();
        return;
    }

    let cwd = workspace::current_dir().unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    let project_root = workspace::find_project_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    let mut command_args = vec!["test".to_string()];
    command_args.extend(args.iter().cloned());
    run_cargo_command(&project_root, &command_args);
}

fn run_cargo_command(project_root: &Path, args: &[String]) {
    let display = workspace::display_command("cargo", args);
    let status = workspace::run_status("cargo", args, project_root).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    if !status.success() {
        let code = status.code().unwrap_or(1);
        eprintln!("Command failed: {display}");
        process::exit(code);
    }
}

fn scaffold_project(
    destination: &Path,
    crate_name: &str,
    local_path: Option<&str>,
    force: bool,
) -> Result<(), String> {
    let dependency = render_hopper_dependency(local_path);
    let cargo_toml = render_cargo_toml(crate_name, &dependency);
    let source = render_lib_rs();
    let readme = render_readme(crate_name);
    let bench_readme = render_bench_readme();
    let gitignore = "/target\n";

    workspace::write_text_file(&destination.join("Cargo.toml"), &cargo_toml, force)?;
    workspace::write_text_file(&destination.join("src").join("lib.rs"), &source, force)?;
    workspace::write_text_file(&destination.join("README.md"), &readme, force)?;
    workspace::write_text_file(&destination.join("bench").join("README.md"), &bench_readme, force)?;
    workspace::write_text_file(&destination.join(".gitignore"), gitignore, force)?;

    Ok(())
}

fn infer_crate_name(destination: &Path) -> String {
    destination
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string()
}

fn normalize_crate_name(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut last_was_separator = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            output.push('_');
            last_was_separator = true;
        }
    }
    output.trim_matches('_').to_string()
}

fn render_hopper_dependency(local_path: Option<&str>) -> String {
    match local_path {
        Some(path) => format!("hopper = {{ path = \"{}\", default-features = false }}", path.replace('\\', "/")),
        None => "hopper = { version = \"0.1.0\", default-features = false }".to_string(),
    }
}

fn render_cargo_toml(crate_name: &str, dependency: &str) -> String {
    format!(
        "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\nlicense = \"Apache-2.0\"\npublish = false\ndescription = \"Hopper program scaffold\"\n\n[lib]\ncrate-type = [\"cdylib\", \"lib\"]\n\n[dependencies]\n{dependency}\n\n[lints.rust]\nunexpected_cfgs = {{ level = \"allow\", check-cfg = ['cfg(target_os, values(\"solana\"))'] }}\n"
    )
}

fn render_lib_rs() -> String {
    r##"#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

hopper_layout! {
    /// Minimal configuration account for a fresh Hopper program.
    pub struct Config, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        bump:      u8                     = 1,
    }
}

hopper_error! {
    base = 7000;
    UnsupportedInstruction,
}

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    _program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    match instruction_data.first().copied() {
        Some(0) => process_init(accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn process_init(accounts: &[AccountView]) -> ProgramResult {
    let authority = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    authority.check_signer()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_constants_are_stable() {
        assert_eq!(Config::DISC, 1);
        assert_eq!(Config::VERSION, 1);
        assert!(Config::LEN >= 33);
    }
}
"##
            .to_string()
}

fn render_readme(crate_name: &str) -> String {
    format!(
        "# {crate_name}\n\nGenerated with `hopper init`. This scaffold defaults to Hopper Native and the Hopper language surface via `hopper::prelude::*`.\n\n## Verify\n\n```bash\nhopper build --host\nhopper test\nhopper build\n```\n\n## Benchmark Stub\n\nUse `hopper profile bench` from a Hopper workspace to run the framework primitive benchmark lab.\n"
    )
}

fn render_bench_readme() -> String {
    "# Benchmark Stub\n\nThis directory is reserved for scenario-specific benchmarks once the program has real instruction flows worth profiling. Hopper's framework-wide primitive lab is available through `hopper profile bench`.\n".to_string()
}

fn print_init_usage() {
    eprintln!("Usage: hopper init <path> [--name <crate-name>] [--local-path <hopper-path>] [--force]");
    eprintln!();
    eprintln!("Create a new Hopper-native program scaffold.");
}

fn print_build_usage() {
    eprintln!("Usage: hopper build [--host|--sbf] [cargo build args]");
    eprintln!();
    eprintln!("Build the current Hopper project. Default mode is SBF (`cargo build-sbf`).");
}

fn print_test_usage() {
    eprintln!("Usage: hopper test [cargo test args]");
    eprintln!();
    eprintln!("Run the current Hopper project's host-side test suite (`cargo test`).");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_normalization_is_stable() {
        assert_eq!(normalize_crate_name("My Hopper Program"), "my_hopper_program");
        assert_eq!(normalize_crate_name("hopper-vault"), "hopper_vault");
    }

    #[test]
    fn local_path_dependency_is_rendered() {
        let dep = render_hopper_dependency(Some("../hopper"));
        assert!(dep.contains("path"));
        assert!(dep.contains("default-features = false"));
    }
}
