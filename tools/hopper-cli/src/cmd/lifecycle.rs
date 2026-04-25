use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use crate::config::{GlobalConfig, HopperToml};
use crate::workspace;
use toml::Value;

/// Project template. Picked interactively or via `--template <name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Minimal,
    NftMint,
    Token2022Vault,
    DefiVault,
}

impl Template {
    fn name(&self) -> &'static str {
        match self {
            Template::Minimal => "minimal",
            Template::NftMint => "nft-mint",
            Template::Token2022Vault => "token-2022-vault",
            Template::DefiVault => "defi-vault",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Template::Minimal => "Minimal — single Config layout, one initialize handler",
            Template::NftMint => "NFT mint — Metaplex CreateMetadataAccountV3 + CreateMasterEditionV3 (1-of-1)",
            Template::Token2022Vault => "Token-2022 vault — extension-aware mint validation + vault state",
            Template::DefiVault => "DeFi vault — segment-safe authority + balance pattern with PDA verification",
        }
    }

    fn from_name(s: &str) -> Option<Self> {
        match s {
            "minimal" => Some(Template::Minimal),
            "nft-mint" | "nft" => Some(Template::NftMint),
            "token-2022-vault" | "t22-vault" | "t22" => Some(Template::Token2022Vault),
            "defi-vault" | "vault" => Some(Template::DefiVault),
            _ => None,
        }
    }

    /// Cargo features to enable on the `hopper` dependency for this template.
    fn cargo_features(&self) -> &'static str {
        match self {
            Template::NftMint => "\"hopper-native-backend\", \"proc-macros\", \"metaplex\"",
            _ => "\"hopper-native-backend\", \"proc-macros\"",
        }
    }
}

/// Git policy after scaffolding. Mirrors Quasar's `init / commit / skip`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitPolicy {
    Commit,
    Init,
    Skip,
}

impl GitPolicy {
    fn from_name(s: &str) -> Self {
        match s {
            "commit" => GitPolicy::Commit,
            "init" => GitPolicy::Init,
            "skip" => GitPolicy::Skip,
            _ => GitPolicy::Commit,
        }
    }
    fn name(&self) -> &'static str {
        match self {
            GitPolicy::Commit => "commit",
            GitPolicy::Init => "init",
            GitPolicy::Skip => "skip",
        }
    }
}

/// Plan resolved from CLI flags, the wizard, or a mix of both.
#[derive(Debug, Clone)]
struct ScaffoldPlan {
    destination: PathBuf,
    crate_name: String,
    template: Template,
    toolchain: String,
    testing: String,
    backend: String,
    local_path: Option<String>,
    git: GitPolicy,
    force: bool,
}

pub fn cmd_init(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_init_usage();
        return;
    }

    let mut destination = None;
    let mut crate_name = None;
    let mut local_path = None;
    let mut template_flag: Option<Template> = None;
    let mut force = false;
    let mut yes = false;
    let mut interactive_flag = false;
    let mut no_git = false;

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
            "--template" | "-t" => {
                if i + 1 >= args.len() {
                    eprintln!("--template requires a value (minimal | nft-mint | token-2022-vault | defi-vault)");
                    process::exit(1);
                }
                let value = &args[i + 1];
                template_flag = Some(Template::from_name(value).unwrap_or_else(|| {
                    eprintln!("Unknown template `{value}`. Try: minimal, nft-mint, token-2022-vault, defi-vault");
                    process::exit(1);
                }));
                i += 2;
            }
            "--force" => {
                force = true;
                i += 1;
            }
            "--yes" | "-y" => {
                yes = true;
                i += 1;
            }
            "--interactive" => {
                interactive_flag = true;
                i += 1;
            }
            "--no-git" => {
                no_git = true;
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

    // Decide whether to run the wizard. Match Quasar's contract: bare
    // `hopper init` (no path, no -y) drops into prompts. `hopper init
    // <path>` skips them and uses saved defaults. `--interactive`
    // forces the wizard even when a path is supplied.
    let wizard_mode = interactive_flag || (destination.is_none() && !yes);

    let plan = if wizard_mode {
        match run_init_wizard(destination.clone(), crate_name.clone(), template_flag, no_git) {
            Ok(plan) => plan,
            Err(err) => {
                eprintln!("hopper init wizard cancelled: {err}");
                process::exit(1);
            }
        }
    } else {
        let destination = destination.unwrap_or_else(|| {
            eprintln!("Missing required <path> for `hopper init <path>`. Run `hopper init` (no path) for the interactive wizard.");
            process::exit(1);
        });
        let inferred_name = crate_name.unwrap_or_else(|| infer_crate_name(&destination));
        let crate_name = normalize_crate_name(&inferred_name);
        if crate_name.is_empty() {
            eprintln!("Could not infer a valid Rust crate name from {}", destination.display());
            process::exit(1);
        }
        let global = GlobalConfig::load();
        let template = template_flag
            .or_else(|| Template::from_name(&global.defaults.template))
            .unwrap_or(Template::Minimal);
        ScaffoldPlan {
            destination,
            crate_name,
            template,
            toolchain: global.defaults.toolchain.clone(),
            testing: global.defaults.testing.clone(),
            backend: global.defaults.backend.clone(),
            local_path,
            git: if no_git {
                GitPolicy::Skip
            } else {
                GitPolicy::from_name(&global.defaults.git)
            },
            force,
        }
    };

    if let Err(err) = execute_scaffold(&plan) {
        eprintln!("hopper init failed: {err}");
        process::exit(1);
    }

    // Persist the wizard's choices as the next-run default. Disable
    // the opening animation on the saved defaults so the second run
    // is silent — power users running `hopper init` repeatedly during
    // plugin development don't get the bounce every time. They can
    // re-enable with `hopper config set ui.animation true` (or by
    // editing `~/.hopper/wizard.toml`).
    if wizard_mode {
        let mut global = GlobalConfig::load();
        global.defaults.template = plan.template.name().to_string();
        global.defaults.toolchain = plan.toolchain.clone();
        global.defaults.testing = plan.testing.clone();
        global.defaults.backend = plan.backend.clone();
        global.defaults.git = plan.git.name().to_string();
        global.ui.animation = false;
        if let Err(err) = global.save() {
            eprintln!("warning: could not save wizard defaults: {err}");
        }
    }

    println!();
    println!(
        "{}  {} {}",
        crate::style::success("Initialized"),
        crate::style::bold(&plan.crate_name),
        crate::style::dim(&format!("at {}", plan.destination.display()))
    );
    println!();
    println!("  {} {}", crate::style::dim("Template:"), plan.template.label());
    println!("  {} {}", crate::style::dim("Backend: "), plan.backend);
    println!("  {} {}", crate::style::dim("Testing: "), plan.testing);
    println!();
    println!("  {}", crate::style::dim("Next steps:"));
    if plan.destination != Path::new(".") {
        println!(
            "    {} {}",
            crate::style::step(""),
            crate::style::bold(&format!("cd {}", plan.destination.display()))
        );
    }
    println!(
        "    {} {}  {}",
        crate::style::step(""),
        crate::style::bold("hopper build --host"),
        crate::style::dim("# host typecheck")
    );
    println!(
        "    {} {}",
        crate::style::step(""),
        crate::style::bold("hopper test")
    );
    println!(
        "    {} {}  {}",
        crate::style::step(""),
        crate::style::bold("hopper build"),
        crate::style::dim("# SBF build")
    );
    println!();
}

fn run_init_wizard(
    destination_hint: Option<PathBuf>,
    name_hint: Option<String>,
    template_hint: Option<Template>,
    no_git_flag: bool,
) -> Result<ScaffoldPlan, String> {
    use dialoguer::{Confirm, Input, Select};

    let theme = dialoguer::theme::ColorfulTheme::default();
    let global = GlobalConfig::load();

    // Animated leap-reveal opens the wizard on the first interactive
    // run; falls through to a plain header on subsequent runs (the
    // wizard flips `ui.animation` to false in the saved defaults
    // after a successful init) or when stdout isn't a TTY. Both
    // behaviours are owned inside `cmd::banner::print_banner`.
    crate::cmd::banner::print_banner(global.ui.animation);

    println!(
        "  {} {}",
        crate::style::dim("Run with"),
        crate::style::bold("--yes")
    );
    println!(
        "  {} {}",
        crate::style::dim("next time to skip these prompts. Saved at"),
        crate::style::dim(&GlobalConfig::path().display().to_string()),
    );
    println!();

    // 1. Project name + destination.
    let default_name = name_hint
        .clone()
        .or_else(|| {
            destination_hint
                .as_ref()
                .map(|p| infer_crate_name(p))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "my-program".to_string());

    let project_name: String = Input::with_theme(&theme)
        .with_prompt("Project name")
        .default(default_name.clone())
        .interact_text()
        .map_err(|e| e.to_string())?;

    let crate_name = normalize_crate_name(&project_name);
    if crate_name.is_empty() {
        return Err("project name does not produce a valid Rust crate name".into());
    }

    let destination = destination_hint.unwrap_or_else(|| PathBuf::from(&project_name));

    // 2. Template.
    let templates = [
        Template::Minimal,
        Template::NftMint,
        Template::Token2022Vault,
        Template::DefiVault,
    ];
    let labels: Vec<&str> = templates.iter().map(|t| t.label()).collect();
    let default_template_idx = template_hint
        .or_else(|| Template::from_name(&global.defaults.template))
        .and_then(|t| templates.iter().position(|x| x == &t))
        .unwrap_or(0);
    let template_idx = Select::with_theme(&theme)
        .with_prompt("Template")
        .items(&labels)
        .default(default_template_idx)
        .interact()
        .map_err(|e| e.to_string())?;
    let template = templates[template_idx];

    // 3. Testing framework.
    let testing_options = ["mollusk", "quasarsvm", "solana-test-validator", "none"];
    let default_testing_idx = testing_options
        .iter()
        .position(|x| *x == global.defaults.testing.as_str())
        .unwrap_or(0);
    let testing_idx = Select::with_theme(&theme)
        .with_prompt("Testing framework")
        .items(&testing_options)
        .default(default_testing_idx)
        .interact()
        .map_err(|e| e.to_string())?;
    let testing = testing_options[testing_idx].to_string();

    // 4. Git policy.
    let git_options = [
        "commit — git init + initial commit",
        "init — git init only, no commit",
        "skip — no git",
    ];
    let default_git_idx = match global.defaults.git.as_str() {
        "init" => 1,
        "skip" => 2,
        _ => 0,
    };
    let git = if no_git_flag {
        GitPolicy::Skip
    } else {
        let git_idx = Select::with_theme(&theme)
            .with_prompt("Git setup")
            .items(&git_options)
            .default(default_git_idx)
            .interact()
            .map_err(|e| e.to_string())?;
        match git_idx {
            0 => GitPolicy::Commit,
            1 => GitPolicy::Init,
            _ => GitPolicy::Skip,
        }
    };

    // 5. Confirm.
    println!();
    println!(" Path:     {}", destination.display());
    println!(" Crate:    {crate_name}");
    println!(" Template: {}", template.label());
    println!(" Testing:  {testing}");
    println!(" Git:      {}", git.name());
    println!();
    let confirmed = Confirm::with_theme(&theme)
        .with_prompt("Scaffold?")
        .default(true)
        .interact()
        .map_err(|e| e.to_string())?;
    if !confirmed {
        return Err("user declined".into());
    }

    Ok(ScaffoldPlan {
        destination,
        crate_name,
        template,
        toolchain: global.defaults.toolchain.clone(),
        testing,
        backend: global.defaults.backend.clone(),
        local_path: None,
        git,
        force: false,
    })
}

fn execute_scaffold(plan: &ScaffoldPlan) -> Result<(), String> {
    scaffold_project(plan)?;
    if !matches!(plan.git, GitPolicy::Skip) {
        if let Err(err) = run_git_init(&plan.destination, plan.git) {
            eprintln!("warning: git setup skipped: {err}");
        }
    }
    Ok(())
}

fn run_git_init(destination: &Path, policy: GitPolicy) -> Result<(), String> {
    let init_status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(destination)
        .status()
        .map_err(|err| format!("failed to launch `git init`: {err}"))?;
    if !init_status.success() {
        return Err("`git init` exited with a non-zero status".into());
    }
    if matches!(policy, GitPolicy::Commit) {
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(destination)
            .status();
        let _ = Command::new("git")
            .args(["commit", "-q", "-m", "Initial Hopper scaffold"])
            .current_dir(destination)
            .status();
    }
    Ok(())
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
                // Snapshot pre-build artefact sizes so we can report a
                // delta on the SBF path. Quasar prints something like
                // "✔ Build complete in 1.2s (56.6 KB, -1.2 KB)";
                // Hopper does the same (size only, since we don't
                // reach inside cargo's wall-clock yet).
                let deploy_dir = workspace_root.join("target").join("deploy");
                let before = snapshot_so_sizes(&deploy_dir);
                match normalize_sbf_build_args(&project_root, &workspace_root, &cargo_args) {
                    Ok(command_args) => run_cargo_command(&workspace_root, &command_args),
                    Err(err) => {
                        eprintln!("hopper build failed: {err}");
                        return;
                    }
                }
                let after = snapshot_so_sizes(&deploy_dir);
                report_size_delta(&before, &after);
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

fn scaffold_project(plan: &ScaffoldPlan) -> Result<(), String> {
    let dependency = render_hopper_dependency(plan.local_path.as_deref(), plan.template);
    let cargo_toml = render_cargo_toml(&plan.crate_name, &dependency);
    let source = render_template_lib_rs(plan.template);
    let readme = render_readme(&plan.crate_name, plan.template);
    let bench_readme = render_bench_readme();
    let gitignore = "/target\n";

    workspace::write_text_file(&plan.destination.join("Cargo.toml"), &cargo_toml, plan.force)?;
    workspace::write_text_file(&plan.destination.join("src").join("lib.rs"), &source, plan.force)?;
    workspace::write_text_file(&plan.destination.join("README.md"), &readme, plan.force)?;
    workspace::write_text_file(
        &plan.destination.join("bench").join("README.md"),
        &bench_readme,
        plan.force,
    )?;
    workspace::write_text_file(&plan.destination.join(".gitignore"), gitignore, plan.force)?;

    // Hopper.toml — declarative project config the rest of the CLI
    // (build, test, deploy, doctor) reads to know toolchain choice,
    // testing framework, and backend.
    let project_config =
        HopperToml::new(plan.crate_name.clone(), plan.template.name().to_string());
    let project_config = HopperToml {
        toolchain: crate::config::ToolchainSection {
            kind: plan.toolchain.clone(),
        },
        testing: crate::config::TestingSection {
            framework: plan.testing.clone(),
        },
        backend: crate::config::BackendSection {
            default: plan.backend.clone(),
        },
        ..project_config
    };
    project_config.save(&plan.destination)?;

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

fn render_hopper_dependency(local_path: Option<&str>, template: Template) -> String {
    let features = template.cargo_features();
    match local_path {
        Some(path) => format!(
            "hopper = {{ path = \"{}\", default-features = false, features = [{features}] }}",
            path.replace('\\', "/")
        ),
        None => format!(
            "hopper = {{ version = \"0.1.0\", default-features = false, features = [{features}] }}"
        ),
    }
}

fn render_template_lib_rs(template: Template) -> String {
    match template {
        Template::Minimal => render_lib_rs(),
        Template::NftMint => render_lib_rs_nft_mint(),
        Template::Token2022Vault => render_lib_rs_token_2022_vault(),
        Template::DefiVault => render_lib_rs_defi_vault(),
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

fn render_readme(crate_name: &str, template: Template) -> String {
    format!(
        "# {crate_name}\n\nGenerated with `hopper init` (template: `{}`). Hopper-native by default, proc-macro authoring path enabled.\n\nDocs: <https://hopperzero.dev>\n\n## Verify\n\n```bash\nhopper build --host    # host typecheck\nhopper test\nhopper build           # SBF build\n```\n\n## Project config\n\nSee `Hopper.toml` for the declarative project configuration\n(toolchain, testing framework, default backend).\n\n## Benchmark stub\n\n`hopper profile bench` runs the framework primitive lab. Add scenario-specific benchmarks under `bench/`.\n",
        template.name()
    )
}

fn render_bench_readme() -> String {
    "# Benchmark Stub\n\nThis directory is reserved for scenario-specific benchmarks once the program has real instruction flows worth profiling. Hopper's framework-wide primitive lab is available through `hopper profile bench`.\n".to_string()
}

fn render_lib_rs_nft_mint() -> String {
    r##"//! Hopper NFT mint scaffold. Uses the `hopper-metaplex` crate to
//! create an NFT metadata + master-edition pair (1-of-1) on top of an
//! existing SPL mint that the caller has already initialised and minted
//! one token to. Two instructions: `create_metadata` and
//! `create_master_edition`.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;
    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();
    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
fast_entrypoint!(process_instruction, 8);

fn process_instruction(
    _program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (disc, rest) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
    match *disc {
        0 => create_metadata(accounts, rest),
        1 => create_master_edition(accounts, rest),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn create_metadata(accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    hopper_load!(accounts => [authority, mint, metadata, system_program, mpl]);
    authority.require_signer()?;
    metadata.require_writable()?;
    if mpl.address().as_array() != MPL_TOKEN_METADATA_PROGRAM_ID.as_array() {
        return Err(ProgramError::IncorrectProgramId);
    }
    // Wire format: [name_len:u8][name][sym_len:u8][sym][uri_len:u8][uri][sfbp:u16][is_mutable:u8]
    let (name, rest) = read_short_string(data)?;
    let (symbol, rest) = read_short_string(rest)?;
    let (uri, rest) = read_short_string(rest)?;
    if rest.len() < 3 { return Err(ProgramError::InvalidInstructionData); }
    let sfbp = u16::from_le_bytes([rest[0], rest[1]]);
    let is_mutable = rest[2] != 0;
    CreateMetadataAccountV3 {
        metadata, mint,
        mint_authority: authority, payer: authority, update_authority: authority,
        system_program, rent: None,
        data: DataV2::simple(name, symbol, uri, sfbp),
        is_mutable,
    }.invoke()
}

fn create_master_edition(accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    hopper_load!(accounts => [authority, mint, metadata, master_edition, token_program, system_program, mpl]);
    authority.require_signer()?;
    if mpl.address().as_array() != MPL_TOKEN_METADATA_PROGRAM_ID.as_array() {
        return Err(ProgramError::IncorrectProgramId);
    }
    if data.len() < 8 { return Err(ProgramError::InvalidInstructionData); }
    let max_supply = u64::from_le_bytes([data[0],data[1],data[2],data[3],data[4],data[5],data[6],data[7]]);
    CreateMasterEditionV3 {
        edition: master_edition, mint, update_authority: authority, mint_authority: authority,
        payer: authority, metadata, token_program, system_program, rent: None,
        max_supply: Some(max_supply),
    }.invoke()
}

fn read_short_string(data: &[u8]) -> Result<(&str, &[u8]), ProgramError> {
    let (&n, rest) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
    if rest.len() < n as usize { return Err(ProgramError::InvalidInstructionData); }
    let (s, tail) = rest.split_at(n as usize);
    let s = core::str::from_utf8(s).map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok((s, tail))
}
"##
        .to_string()
}

fn render_lib_rs_token_2022_vault() -> String {
    r##"//! Hopper Token-2022 vault scaffold. Validates that an incoming
//! Token-2022 mint has none of the unsafe extensions (transfer fee,
//! permanent delegate, confidential transfer, non-transferable,
//! transfer hook) before accepting deposits. Pattern: extension
//! screening as a first-class gate.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;
use hopper::hopper_token_2022::check_safe_token_2022_mint;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;
    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();
    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

#[derive(Clone, Copy)]
pub struct Authority;
#[derive(Clone, Copy)]
pub struct Mint;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    pub authority: TypedAddress<Authority>,
    pub mint: TypedAddress<Mint>,
    pub bump: u8,
}

#[cfg(target_os = "solana")]
fast_entrypoint!(process_instruction, 4);

fn process_instruction(
    _program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (disc, _) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
    match *disc {
        0 => screen_mint(accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn screen_mint(accounts: &[AccountView]) -> ProgramResult {
    hopper_load!(accounts => [mint]);
    let mint_data = mint.try_borrow()?;
    check_safe_token_2022_mint(&mint_data)?;
    Ok(())
}
"##
        .to_string()
}

// ---------------------------------------------------------------------------
// Build-time helpers
// ---------------------------------------------------------------------------

/// Snapshot the size in bytes of every `.so` file inside a `target/deploy/`
/// directory. Returns an empty map if the directory does not exist
/// (first build of a fresh project).
fn snapshot_so_sizes(deploy_dir: &Path) -> std::collections::HashMap<PathBuf, u64> {
    let mut map = std::collections::HashMap::new();
    let entries = match fs::read_dir(deploy_dir) {
        Ok(it) => it,
        Err(_) => return map,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("so") {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            map.insert(path, meta.len());
        }
    }
    map
}

/// Compare a before/after snapshot of deploy artefact sizes and print
/// a human-readable line per binary that changed. Format:
///
/// ```text
///   ✔ my_program.so   56.6 KiB (-1.2 KiB)
/// ```
///
/// New binaries (present in `after` but not `before`) print with `(new)`.
/// Removed binaries are silent — `cargo build-sbf` doesn't usually
/// remove artefacts and we'd rather not draw attention if it does.
fn report_size_delta(
    before: &std::collections::HashMap<PathBuf, u64>,
    after: &std::collections::HashMap<PathBuf, u64>,
) {
    let mut printed_any = false;
    let mut paths: Vec<&PathBuf> = after.keys().collect();
    paths.sort();
    for path in paths {
        let new = after[path];
        let prev = before.get(path).copied();
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("<unknown>");
        let new_size = crate::style::human_size(new);
        let line = match prev {
            None => format!(
                "  {} {}  {}  {}",
                crate::style::success(""),
                crate::style::bold(name),
                crate::style::dim(&new_size),
                crate::style::dim("(new)")
            ),
            Some(p) if p == new => continue,
            Some(p) => {
                let delta = new as i64 - p as i64;
                let sign = if delta >= 0 { "+" } else { "" };
                let delta_kib = delta as f64 / 1024.0;
                let delta_str = format!("({sign}{delta_kib:.2} KiB)");
                // Colour-cue: green when shrinking, yellow when
                // growing, dim for an unchanged-size rebuild (which
                // we already filter out above).
                let coloured_delta = if delta < 0 {
                    crate::style::color(83, &delta_str)
                } else if delta > 0 {
                    crate::style::color(208, &delta_str)
                } else {
                    crate::style::dim(&delta_str)
                };
                format!(
                    "  {} {}  {}  {}",
                    crate::style::success(""),
                    crate::style::bold(name),
                    crate::style::dim(&new_size),
                    coloured_delta
                )
            }
        };
        if !printed_any {
            println!();
        }
        println!("{line}");
        printed_any = true;
    }
}

fn render_lib_rs_defi_vault() -> String {
    r##"//! Hopper DeFi vault scaffold. Authority + balance state with
//! segment-level borrow tracking, PDA-bound vault, and the canonical
//! verify-only PDA path that saves ~350 CU per instruction over
//! `find_program_address`.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

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
pub struct Authority;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    pub authority: TypedAddress<Authority>,
    pub balance: WireU64,
    pub bump: u8,
}

#[cfg(target_os = "solana")]
fast_entrypoint!(process_instruction, 3);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (disc, rest) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
    match *disc {
        0 => deposit(program_id, accounts, rest),
        1 => withdraw(program_id, accounts, rest),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn deposit(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    hopper_load!(accounts => [user, vault]);
    user.require_signer()?;
    vault.require_writable()?;
    find_and_verify_pda(vault, &[b"vault", user.address().as_ref()], program_id)?;

    if data.len() < 8 { return Err(ProgramError::InvalidInstructionData); }
    let amount = u64::from_le_bytes([data[0],data[1],data[2],data[3],data[4],data[5],data[6],data[7]]);

    // Segment-safe balance bump: locks just the 8 bytes of `balance`.
    let mut borrows = SegmentBorrowRegistry::new();
    let mut balance = vault.segment_mut::<WireU64>(&mut borrows, Vault::BALANCE_ABS_OFFSET, 8)?;
    let next = balance.get().checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;
    *balance = WireU64::new(next);
    Ok(())
}

fn withdraw(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    hopper_load!(accounts => [user, vault]);
    user.require_signer()?;
    vault.require_writable()?;
    find_and_verify_pda(vault, &[b"vault", user.address().as_ref()], program_id)?;

    if data.len() < 8 { return Err(ProgramError::InvalidInstructionData); }
    let amount = u64::from_le_bytes([data[0],data[1],data[2],data[3],data[4],data[5],data[6],data[7]]);

    let mut borrows = SegmentBorrowRegistry::new();
    let mut balance = vault.segment_mut::<WireU64>(&mut borrows, Vault::BALANCE_ABS_OFFSET, 8)?;
    let current = balance.get();
    if amount > current { return Err(ProgramError::InsufficientFunds); }
    *balance = WireU64::new(current - amount);
    Ok(())
}
"##
        .to_string()
}

fn print_init_usage() {
    eprintln!("Usage:");
    eprintln!("  hopper init                                  Interactive wizard");
    eprintln!("  hopper init <path> [flags]                   Use saved defaults");
    eprintln!();
    eprintln!("Flags:");
    eprintln!("  --template, -t <name>     minimal | nft-mint | token-2022-vault | defi-vault");
    eprintln!("  --name <crate-name>       Override the inferred crate name");
    eprintln!("  --local-path <path>       Path-dep on a local Hopper checkout (development)");
    eprintln!("  --yes, -y                 Skip prompts (use saved defaults from ~/.hopper/wizard.toml)");
    eprintln!("  --interactive             Force the wizard even when <path> is supplied");
    eprintln!("  --no-git                  Skip git init / initial commit");
    eprintln!("  --force                   Overwrite existing files at <path>");
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
        // Function signature is `(local_path, template)` now —
        // template determines which feature flags get stamped onto
        // the dependency line.
        let dep = render_hopper_dependency(Some("../hopper"), Template::Minimal);
        assert!(dep.contains("path"));
        assert!(dep.contains("default-features = false"));
        assert!(dep.contains("hopper-native-backend"));

        // The NFT template adds the `metaplex` feature.
        let dep_nft = render_hopper_dependency(Some("../hopper"), Template::NftMint);
        assert!(dep_nft.contains("metaplex"));
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
