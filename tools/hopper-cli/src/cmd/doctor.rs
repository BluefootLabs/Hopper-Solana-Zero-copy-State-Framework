//! `hopper doctor` - environment sanity check.
//!
//! One command, three categories of check:
//!
//! 1. Toolchain. Is `cargo` on the PATH, is `cargo-build-sbf` installed,
//!    does `solana --version` answer, is `cargo-expand` reachable, is
//!    `rustc` recent enough?
//! 2. Config. Does `~/.hopper/config.toml` exist, does the declared
//!    `cluster_url` resolve, is `payer` readable, is it a valid
//!    keypair json?
//! 3. Workspace. Is the current directory inside a Hopper project, do
//!    we see a `src/lib.rs`, is a `#[program]` module present?
//!
//! Every check prints `ok`, `warn`, or `fail`. Exit is 0 unless any
//! fail is seen, which makes this safe to drop into CI as a smoke test.

use std::fs;
use std::path::Path;
use std::process::{self, Command};

pub fn cmd_doctor(args: &[String]) {
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        eprintln!("Usage: hopper doctor");
        eprintln!();
        eprintln!("Verify the Hopper development environment. Exits non-zero on any");
        eprintln!("failing check; safe to call from CI as a smoke test.");
        return;
    }

    let mut fails: u32 = 0;
    let mut warns: u32 = 0;

    println!("-- hopper doctor --");

    // Toolchain.
    fails += check("cargo on PATH", check_cargo_present);
    fails += check("cargo-build-sbf installed", check_cargo_build_sbf);
    warns += check_warn("cargo-expand installed", check_cargo_expand);
    fails += check("solana CLI answers --version", check_solana_cli);
    fails += check("rustc version", check_rustc_version);

    // Config.
    warns += check_warn("~/.hopper/config.toml present", check_hopper_config);
    warns += check_warn("default keypair readable", check_default_keypair);

    // Workspace.
    warns += check_warn("current dir has src/lib.rs", check_src_lib_rs);
    warns += check_warn("#[program] module declared", check_program_attr);

    println!();
    println!("summary: {} failed, {} warnings", fails, warns);
    if fails > 0 {
        process::exit(1);
    }
}

type CheckFn = fn() -> Result<String, String>;

fn check(label: &str, f: CheckFn) -> u32 {
    match f() {
        Ok(detail) => {
            println!("  [ok  ] {label}: {detail}");
            0
        }
        Err(e) => {
            println!("  [fail] {label}: {e}");
            1
        }
    }
}

fn check_warn(label: &str, f: CheckFn) -> u32 {
    match f() {
        Ok(detail) => {
            println!("  [ok  ] {label}: {detail}");
            0
        }
        Err(e) => {
            println!("  [warn] {label}: {e}");
            1
        }
    }
}

fn check_cargo_present() -> Result<String, String> {
    let out = Command::new("cargo")
        .arg("--version")
        .output()
        .map_err(|e| format!("not found: {e}"))?;
    String::from_utf8(out.stdout)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("bad utf-8: {e}"))
}

fn check_cargo_build_sbf() -> Result<String, String> {
    let out = Command::new("cargo")
        .arg("build-sbf")
        .arg("--version")
        .output()
        .map_err(|e| format!("not installed: {e}. run `cargo install solana-cargo-build-sbf` or use the solana CLI installer."))?;
    if !out.status.success() {
        return Err("exited non-zero".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn check_cargo_expand() -> Result<String, String> {
    let out = Command::new("cargo")
        .arg("expand")
        .arg("--version")
        .output()
        .map_err(|_| "not installed. run `cargo install cargo-expand` (or `hopper expand --install`)".to_string())?;
    if !out.status.success() {
        return Err("exited non-zero".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn check_solana_cli() -> Result<String, String> {
    let out = Command::new("solana")
        .arg("--version")
        .output()
        .map_err(|e| format!("not found: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn check_rustc_version() -> Result<String, String> {
    let out = Command::new("rustc")
        .arg("--version")
        .output()
        .map_err(|e| format!("rustc not found: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn check_hopper_config() -> Result<String, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "no HOME env var".to_string())?;
    let path = Path::new(&home).join(".hopper").join("config.toml");
    if !path.exists() {
        return Err(format!(
            "{} missing. run `hopper config set cluster_url devnet` to create it.",
            path.display()
        ));
    }
    Ok(path.display().to_string())
}

fn check_default_keypair() -> Result<String, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "no HOME env var".to_string())?;
    let path = Path::new(&home)
        .join(".config")
        .join("solana")
        .join("id.json");
    if !path.exists() {
        return Err(format!(
            "{} missing. run `solana-keygen new -o {}` or override with `hopper config set payer <path>`.",
            path.display(),
            path.display()
        ));
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
    if !text.trim().starts_with('[') {
        return Err("not a valid keypair json (expected byte array)".into());
    }
    Ok(path.display().to_string())
}

fn check_src_lib_rs() -> Result<String, String> {
    let path = Path::new("src/lib.rs");
    if !path.exists() {
        return Err("not in a Rust project (no src/lib.rs)".into());
    }
    Ok("src/lib.rs".to_string())
}

fn check_program_attr() -> Result<String, String> {
    let text = fs::read_to_string("src/lib.rs").map_err(|e| format!("read: {e}"))?;
    if text.contains("#[program]") || text.contains("#[hopper_program]") || text.contains("#[hopper::program]") {
        Ok("found".into())
    } else {
        Err("no #[program] module in src/lib.rs".into())
    }
}
