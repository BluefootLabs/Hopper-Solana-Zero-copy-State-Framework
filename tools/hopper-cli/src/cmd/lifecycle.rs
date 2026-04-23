use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use crate::workspace;
use toml::Value;

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
    let workspace_root = workspace::find_workspace_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    let mut use_host = false;
    let mut cargo_args: Vec<String> = Vec::new();
    for arg in args {
        if arg == "--host" {
            use_host = true;
        } else if arg == "--sbf" {
            use_host = false;
        } else {
            cargo_args.push(arg.clone());
        }
    }
    let watch_mode = crate::cmd::watch::extract_watch_flag(&mut cargo_args);

    let run_once = {
        let project_root = project_root.clone();
        let workspace_root = workspace_root.clone();
        let cargo_args = cargo_args.clone();
        move || {
            if use_host {
                let mut command_args = vec!["build".to_string()];
                command_args.extend(cargo_args.iter().cloned());
                run_cargo_command(&project_root, &command_args);
            } else {
                match normalize_sbf_build_args(&project_root, &workspace_root, &cargo_args) {
                    Ok(command_args) => run_cargo_command(&workspace_root, &command_args),
                    Err(err) => eprintln!("hopper build failed: {err}"),
                }
            }
        }
    };

    if watch_mode {
        crate::cmd::watch::watch(&project_root, run_once);
    } else {
        run_once();
    }
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

    let mut passthrough: Vec<String> = args.iter().cloned().collect();
    let watch_mode = crate::cmd::watch::extract_watch_flag(&mut passthrough);

    let run_once = {
        let project_root = project_root.clone();
        let passthrough = passthrough.clone();
        move || {
            let mut command_args = vec!["test".to_string()];
            command_args.extend(passthrough.iter().cloned());
            run_cargo_command(&project_root, &command_args);
        }
    };

    if watch_mode {
        crate::cmd::watch::watch(&project_root, run_once);
    } else {
        run_once();
    }
}

pub fn cmd_deploy(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_deploy_usage();
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
    let workspace_root = workspace::find_workspace_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    let (common, solana_args) = parse_lifecycle_args(args).unwrap_or_else(|err| {
        eprintln!("hopper deploy failed: {err}");
        process::exit(1);
    });

    if !common.no_build {
        build_sbf(&project_root, &workspace_root, common.package.as_deref());
    }

    let artifact = resolve_sbf_artifact(&project_root, &workspace_root, common.package.as_deref())
        .unwrap_or_else(|err| {
            eprintln!("hopper deploy failed: {err}");
            process::exit(1);
        });

    let mut command_args = vec![
        "program".to_string(),
        "deploy".to_string(),
        artifact.display().to_string(),
    ];
    if !solana_args.iter().any(|arg| arg == "--use-rpc") {
        command_args.push("--use-rpc".to_string());
    }
    command_args.extend(solana_args);
    run_external_command("solana", &workspace_root, &command_args);
}

pub fn cmd_dump(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_dump_usage();
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
    let workspace_root = workspace::find_workspace_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    let (common, dump_options) = parse_dump_args(args).unwrap_or_else(|err| {
        eprintln!("hopper dump failed: {err}");
        process::exit(1);
    });

    if !common.no_build {
        build_sbf(&project_root, &workspace_root, common.package.as_deref());
    }

    let artifact = resolve_sbf_artifact(&project_root, &workspace_root, common.package.as_deref())
        .unwrap_or_else(|err| {
            eprintln!("hopper dump failed: {err}");
            process::exit(1);
        });

    let output = run_objdump(&workspace_root, &artifact, dump_options.tool.as_deref())
        .unwrap_or_else(|err| {
            eprintln!("hopper dump failed: {err}");
            process::exit(1);
        });

    if let Some(out_path) = dump_options.out {
        workspace::write_text_file(&out_path, &output, true).unwrap_or_else(|err| {
            eprintln!("hopper dump failed: {err}");
            process::exit(1);
        });
        println!("Wrote disassembly to {}", out_path.display());
    } else {
        print!("{output}");
    }
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

fn run_external_command(program: &str, cwd: &Path, args: &[String]) {
    let display = workspace::display_command(program, args);
    let status = workspace::run_status(program, args, cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    if !status.success() {
        let code = status.code().unwrap_or(1);
        eprintln!("Command failed: {display}");
        process::exit(code);
    }
}

#[derive(Default)]
struct CommonLifecycleOptions {
    no_build: bool,
    package: Option<String>,
}

#[derive(Default)]
struct DumpOptions {
    out: Option<PathBuf>,
    tool: Option<String>,
}

fn parse_lifecycle_args(args: &[String]) -> Result<(CommonLifecycleOptions, Vec<String>), String> {
    let mut common = CommonLifecycleOptions::default();
    let mut passthrough = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--no-build" => {
                common.no_build = true;
                i += 1;
            }
            "-p" | "--package" => {
                if i + 1 >= args.len() {
                    return Err(format!("{} requires a package name", args[i]));
                }
                common.package = Some(args[i + 1].clone());
                i += 2;
            }
            other => {
                passthrough.push(other.to_string());
                i += 1;
            }
        }
    }

    Ok((common, passthrough))
}

fn parse_dump_args(args: &[String]) -> Result<(CommonLifecycleOptions, DumpOptions), String> {
    let mut common = CommonLifecycleOptions::default();
    let mut dump = DumpOptions::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--no-build" => {
                common.no_build = true;
                i += 1;
            }
            "-p" | "--package" => {
                if i + 1 >= args.len() {
                    return Err(format!("{} requires a package name", args[i]));
                }
                common.package = Some(args[i + 1].clone());
                i += 2;
            }
            "--out" => {
                if i + 1 >= args.len() {
                    return Err("--out requires a path".to_string());
                }
                dump.out = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--tool" => {
                if i + 1 >= args.len() {
                    return Err("--tool requires an executable name".to_string());
                }
                dump.tool = Some(args[i + 1].clone());
                i += 2;
            }
            other => return Err(format!("Unknown dump argument: {other}")),
        }
    }

    Ok((common, dump))
}

fn build_sbf(project_root: &Path, workspace_root: &Path, package: Option<&str>) {
    let package_args: Vec<String> = package
        .map(|package| vec!["--package".to_string(), package.to_string()])
        .unwrap_or_default();
    let command_args = normalize_sbf_build_args(project_root, workspace_root, &package_args)
        .unwrap_or_else(|err| {
            eprintln!("hopper build failed: {err}");
            process::exit(1);
        });
    run_cargo_command(workspace_root, &command_args);
}

fn normalize_sbf_build_args(
    project_root: &Path,
    workspace_root: &Path,
    cargo_args: &[String],
) -> Result<Vec<String>, String> {
    let mut command_args = vec!["build-sbf".to_string()];
    let mut passthrough = Vec::new();
    let mut manifest_path: Option<PathBuf> = None;
    let mut i = 0;

    while i < cargo_args.len() {
        match cargo_args[i].as_str() {
            "-p" | "--package" => {
                if i + 1 >= cargo_args.len() {
                    return Err(format!("{} requires a package name", cargo_args[i]));
                }
                manifest_path = Some(workspace::resolve_workspace_member_manifest(
                    workspace_root,
                    &cargo_args[i + 1],
                )?);
                i += 2;
            }
            "--manifest-path" => {
                if i + 1 >= cargo_args.len() {
                    return Err("--manifest-path requires a path".to_string());
                }
                manifest_path = Some(PathBuf::from(&cargo_args[i + 1]));
                passthrough.push(cargo_args[i].clone());
                passthrough.push(cargo_args[i + 1].clone());
                i += 2;
            }
            other if other.starts_with("--manifest-path=") => {
                let value = other
                    .split_once('=')
                    .map(|(_, value)| value)
                    .unwrap_or_default();
                manifest_path = Some(PathBuf::from(value));
                passthrough.push(other.to_string());
                i += 1;
            }
            other => {
                passthrough.push(other.to_string());
                i += 1;
            }
        }
    }

    if manifest_path.is_none() {
        manifest_path = Some(project_root.join("Cargo.toml"));
    }

    if !passthrough.iter().any(|arg| arg == "--manifest-path" || arg.starts_with("--manifest-path=")) {
        command_args.push("--manifest-path".to_string());
        command_args.push(
            manifest_path
                .as_ref()
                .expect("manifest path was populated above")
                .display()
                .to_string(),
        );
    }

    command_args.extend(passthrough);
    Ok(command_args)
}

fn resolve_sbf_artifact(
    project_root: &Path,
    workspace_root: &Path,
    package_hint: Option<&str>,
) -> Result<PathBuf, String> {
    let crate_name = resolve_package_name(project_root, package_hint)?;
    let artifact_name = crate_name.replace('-', "_") + ".so";
    let candidates = [
        workspace_root.join("target").join("deploy").join(&artifact_name),
        project_root.join("target").join("deploy").join(&artifact_name),
    ];

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not find {} under target/deploy. Run `hopper build` first or pass -p/--package when running from a workspace root.",
        artifact_name
    ))
}

fn resolve_package_name(project_root: &Path, package_hint: Option<&str>) -> Result<String, String> {
    if let Some(package) = package_hint {
        return Ok(package.to_string());
    }

    let cargo_toml_path = project_root.join("Cargo.toml");
    let cargo_toml = fs::read_to_string(&cargo_toml_path)
        .map_err(|err| format!("Failed to read {}: {err}", cargo_toml_path.display()))?;
    let value: Value = cargo_toml.parse()
        .map_err(|err| format!("Failed to parse {}: {err}", cargo_toml_path.display()))?;
    value
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "{} does not declare [package].name; rerun with -p/--package <crate>",
                cargo_toml_path.display()
            )
        })
}

fn run_objdump(workspace_root: &Path, artifact: &Path, explicit_tool: Option<&str>) -> Result<String, String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(tool) = explicit_tool {
        candidates.push(tool.to_string());
    } else if let Ok(tool) = std::env::var("HOPPER_OBJDUMP") {
        if !tool.trim().is_empty() {
            candidates.push(tool);
        }
    }
    candidates.extend([
        "llvm-objdump".to_string(),
        "solana-llvm-objdump".to_string(),
        "rust-objdump".to_string(),
    ]);

    let args = vec!["-d".to_string(), artifact.display().to_string()];
    let mut last_error = None;

    for tool in candidates {
        match workspace::run_output(&tool, &args, workspace_root) {
            Ok(output) if output.status.success() => {
                return String::from_utf8(output.stdout)
                    .map_err(|err| format!("objdump output was not valid UTF-8: {err}"));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                last_error = Some(format!("{} failed: {}", tool, stderr));
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        "No usable objdump tool found. Set HOPPER_OBJDUMP or pass --tool <executable>.".to_string()
    }))
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
        Some(path) => format!(
            "hopper = {{ path = \"{}\", default-features = false, features = [\"hopper-native-backend\", \"proc-macros\"] }}",
            path.replace('\\', "/")
        ),
        None => "hopper = { version = \"0.1.0\", default-features = false, features = [\"hopper-native-backend\", \"proc-macros\"] }".to_string(),
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

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Config {
    pub authority: TypedAddress<Authority>,
    pub bump: u8,
}

#[hopper::context]
pub struct Initialize {
    #[account(mut(authority, bump))]
    pub config: Config,

    #[signer]
    pub authority: AccountView,
}

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    app::process_instruction(&mut ctx)
}

#[hopper::program]
mod app {
    use super::*;

    #[hopper::pipeline]
    #[instruction(0)]
    pub fn initialize(ctx: Context<Initialize>) -> ProgramResult {
        let authority = TypedAddress::from_account(ctx.account(1)?);
        *ctx.config_authority_mut()? = authority;
        *ctx.config_bump_mut()? = 0;
        Ok(())
    }
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

fn print_deploy_usage() {
    eprintln!("Usage: hopper deploy [--no-build] [-p|--package <crate>] [solana program deploy args]");
    eprintln!();
    eprintln!("Build the current Hopper SBF program if needed, then run `solana program deploy`." );
}

fn print_dump_usage() {
    eprintln!("Usage: hopper dump [--no-build] [-p|--package <crate>] [--tool <objdump>] [--out <path>]");
    eprintln!();
    eprintln!("Disassemble the built SBF `.so` using llvm-objdump, solana-llvm-objdump, or rust-objdump.");
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

    #[test]
    fn package_name_normalization_prefers_cli_hint() {
        let parsed = parse_lifecycle_args(&[
            "--no-build".to_string(),
            "-p".to_string(),
            "hopper-vault".to_string(),
            "--url".to_string(),
            "http://localhost:8899".to_string(),
        ]).unwrap();
        assert!(parsed.0.no_build);
        assert_eq!(parsed.0.package.as_deref(), Some("hopper-vault"));
        assert_eq!(parsed.1, vec!["--url", "http://localhost:8899"]);
    }
}
