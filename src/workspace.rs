use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use toml::Value;

pub fn current_dir() -> Result<PathBuf, String> {
    env::current_dir().map_err(|err| format!("Failed to determine current directory: {err}"))
}

pub fn find_project_root(start: &Path) -> Result<PathBuf, String> {
    for dir in start.ancestors() {
        if dir.join("Cargo.toml").exists() {
            return Ok(dir.to_path_buf());
        }
    }
    Err(format!(
        "No Cargo.toml found while searching upward from {}",
        start.display()
    ))
}

pub fn find_workspace_root(start: &Path) -> Result<PathBuf, String> {
    let mut workspace = None;
    for dir in start.ancestors() {
        let cargo_toml = dir.join("Cargo.toml");
        if !cargo_toml.exists() {
            continue;
        }

        let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
        if content.contains("[workspace]") {
            workspace = Some(dir.to_path_buf());
        }
    }

    workspace.or_else(|| find_project_root(start).ok()).ok_or_else(|| {
        format!(
            "No Cargo workspace root found while searching upward from {}",
            start.display()
        )
    })
}

pub fn run_status(program: &str, args: &[String], cwd: &Path) -> Result<ExitStatus, String> {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .map_err(|err| format!("Failed to run {}: {err}", display_command(program, args)))
}

pub fn run_output(program: &str, args: &[String], cwd: &Path) -> Result<Output, String> {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|err| format!("Failed to run {}: {err}", display_command(program, args)))
}

pub fn display_command(program: &str, args: &[String]) -> String {
    let mut rendered = String::from(program);
    for arg in args {
        rendered.push(' ');
        if arg.contains(' ') {
            rendered.push('"');
            rendered.push_str(arg);
            rendered.push('"');
        } else {
            rendered.push_str(arg);
        }
    }
    rendered
}

pub fn default_solana_keypair_path() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("SOLANA_KEYPAIR") {
        if !explicit.trim().is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }

    home_dir().map(|home| home.join(".config").join("solana").join("id.json"))
}

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
}

pub fn resolve_workspace_member_manifest(workspace_root: &Path, package: &str) -> Result<PathBuf, String> {
    let workspace_manifest_path = workspace_root.join("Cargo.toml");
    let workspace_manifest = fs::read_to_string(&workspace_manifest_path).map_err(|err| {
        format!(
            "Failed to read {}: {err}",
            workspace_manifest_path.display()
        )
    })?;
    let workspace_value: Value = workspace_manifest.parse().map_err(|err| {
        format!(
            "Failed to parse {}: {err}",
            workspace_manifest_path.display()
        )
    })?;

    let members = workspace_value
        .get("workspace")
        .and_then(Value::as_table)
        .and_then(|workspace| workspace.get("members"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "{} does not declare workspace members",
                workspace_manifest_path.display()
            )
        })?;

    for member in members {
        let Some(member_path) = member.as_str() else {
            continue;
        };
        let manifest_path = workspace_root.join(member_path).join("Cargo.toml");
        if !manifest_path.exists() {
            continue;
        }

        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|err| format!("Failed to read {}: {err}", manifest_path.display()))?;
        let manifest_value: Value = manifest
            .parse()
            .map_err(|err| format!("Failed to parse {}: {err}", manifest_path.display()))?;

        let package_name = manifest_value
            .get("package")
            .and_then(Value::as_table)
            .and_then(|package_table| package_table.get("name"))
            .and_then(Value::as_str);

        if package_name == Some(package) {
            return Ok(manifest_path);
        }
    }

    Err(format!(
        "Could not find a workspace member named {} under {}",
        package,
        workspace_root.display()
    ))
}

pub fn infer_program_manifest_for_project(start: &Path) -> Result<PathBuf, String> {
    let project_root = find_project_root(start)?;
    infer_program_manifest_in_dir(&project_root)
}

pub fn infer_program_manifest_for_package(workspace_root: &Path, package: &str) -> Result<PathBuf, String> {
    let manifest_path = resolve_workspace_member_manifest(workspace_root, package)?;
    let project_root = manifest_path.parent().ok_or_else(|| {
        format!(
            "Resolved manifest {} has no parent directory",
            manifest_path.display()
        )
    })?;
    infer_program_manifest_in_dir(project_root)
}

fn infer_program_manifest_in_dir(project_root: &Path) -> Result<PathBuf, String> {
    let cargo_manifest = project_root.join("Cargo.toml");
    let package_name = if cargo_manifest.exists() {
        read_package_name(&cargo_manifest).ok()
    } else {
        None
    };

    let mut candidates = vec![project_root.join("hopper.manifest.json")];
    if let Some(name) = package_name {
        candidates.push(project_root.join(format!("{name}.manifest.json")));
    }
    candidates.push(project_root.join("manifest.json"));

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not find a generated Hopper program manifest under {}. Looked for hopper.manifest.json, <package>.manifest.json, and manifest.json.",
        project_root.display()
    ))
}

fn read_package_name(manifest_path: &Path) -> Result<String, String> {
    let manifest = fs::read_to_string(manifest_path)
        .map_err(|err| format!("Failed to read {}: {err}", manifest_path.display()))?;
    let manifest_value: Value = manifest
        .parse()
        .map_err(|err| format!("Failed to parse {}: {err}", manifest_path.display()))?;

    manifest_value
        .get("package")
        .and_then(Value::as_table)
        .and_then(|package_table| package_table.get("name"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{} does not declare package.name", manifest_path.display()))
}

pub fn write_text_file(path: &Path, contents: &str, force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!(
            "Refusing to overwrite existing file {} without --force",
            path.display()
        ));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!("Failed to create directory {}: {err}", parent.display())
        })?;
    }

    fs::write(path, contents)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}
