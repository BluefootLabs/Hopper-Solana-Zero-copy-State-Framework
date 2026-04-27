//! `hopper clean` — delete build artefacts.
//!
//! Two modes:
//!
//! - **Default** (no flags): walk the workspace `target/` and clear
//!   the directories `hopper` actually produces — `target/deploy`,
//!   `target/idl`, `target/client`, `target/profile`, `target/hopper`.
//!   Inside `target/deploy/` we **preserve `*-keypair.json`** because
//!   losing a program keypair means losing the on-chain program
//!   address. Quasar makes the same exception; we follow it.
//! - **`--all` / `-a`**: above, plus `cargo clean` from the project
//!   root for a full target wipe (LLVM build cache, intermediate
//!   `.rmeta`, dep-graph, the lot).
//!
//! Why keep these as separate dirs and not just nuke `target/`? Most
//! Hopper users keep a Cargo workspace with non-SBF crates inside it
//! (the schema crates, derive crates, internal harnesses). A blunt
//! `rm -rf target/` would force a full rebuild of those host-side
//! crates every time someone wants a fresh `.so`. Selective deletion
//! of the SBF artefacts and keeping host build cache is the right
//! default.
//!
//! Errors during deletion are reported but non-fatal — we report each
//! removed directory and end with `clean` so users can see the
//! summary even if one subdirectory was locked by another process.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{self, Command};

use crate::style;
use crate::workspace;

/// Directories under `target/` that Hopper writes to. Must be kept in
/// sync with the producers — `hopper build`, `hopper compile --emit
/// idl/codama/ts/kt`, `hopper profile bench`, etc.
const HOPPER_OUTPUT_DIRS: &[&str] = &[
    "target/deploy",
    "target/idl",
    "target/client",
    "target/profile",
    "target/hopper",
];

pub fn cmd_clean(args: &[String]) {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_clean_usage();
        return;
    }

    let mut all = false;
    for arg in args {
        match arg.as_str() {
            "--all" | "-a" => all = true,
            other => {
                eprintln!("Unknown clean flag: {other}");
                print_clean_usage();
                process::exit(1);
            }
        }
    }

    let cwd = workspace::current_dir().unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    let workspace_root = workspace::find_workspace_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    let project_root = workspace::find_project_root(&cwd).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    // Find which directories actually exist before we go touching them
    // — otherwise `target/idl` not existing in a fresh project would
    // print as if we tried to delete it. Quasar gates on `.exists()`
    // for the same reason; we copy that behaviour.
    let existing: Vec<&str> = HOPPER_OUTPUT_DIRS
        .iter()
        .copied()
        .filter(|rel| workspace_root.join(rel).exists())
        .collect();

    if existing.is_empty() && !all {
        println!("  {}", style::dim("nothing to clean"));
        return;
    }

    let mut removed_count = 0usize;
    let mut error_count = 0usize;

    for rel in &existing {
        let dir = workspace_root.join(rel);

        let result = if *rel == "target/deploy" {
            clean_deploy_dir(&dir)
        } else {
            fs::remove_dir_all(&dir)
        };

        match result {
            Ok(_) => {
                println!("  {} {}", style::success("removed"), style::dim(rel));
                removed_count += 1;
            }
            Err(err) => {
                eprintln!(
                    "  {} {} {}",
                    style::fail("failed"),
                    style::dim(rel),
                    err
                );
                error_count += 1;
            }
        }
    }

    if all {
        println!("  {} cargo clean", style::step("running"));
        let status = Command::new("cargo")
            .arg("clean")
            .current_dir(&project_root)
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("  {} {}", style::success("cleared"), style::dim("target/ (cargo clean)"));
                removed_count += 1;
            }
            Ok(s) => {
                eprintln!(
                    "  {} cargo clean exited with {}",
                    style::fail("failed"),
                    s.code().unwrap_or(1)
                );
                error_count += 1;
            }
            Err(err) => {
                eprintln!("  {} cargo clean: {err}", style::fail("failed"));
                error_count += 1;
            }
        }
    }

    println!();
    if error_count == 0 {
        println!(
            "{}",
            style::success(&format!("clean ({} cleared)", removed_count))
        );
    } else {
        println!(
            "{}",
            style::warn(&format!(
                "clean ({} cleared, {} failed)",
                removed_count, error_count
            ))
        );
        process::exit(1);
    }
}

/// Delete every entry in `target/deploy/` except `*-keypair.json`.
///
/// We can't just `remove_dir_all` because keypairs live alongside the
/// `.so` files. Walk one level deep and skip anything whose filename
/// ends with `-keypair.json`. We don't recurse — `cargo build-sbf`
/// doesn't put nested directories in there, and recursing would
/// introduce surprising "where did my files go" failure modes.
fn clean_deploy_dir(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let is_keypair = path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.ends_with("-keypair.json"));
        if is_keypair {
            continue;
        }
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn print_clean_usage() {
    eprintln!("Usage: hopper clean [--all|-a]");
    eprintln!();
    eprintln!("Clear Hopper build artefacts under the workspace target/.");
    eprintln!();
    eprintln!("  Default:  delete target/{{deploy,idl,client,profile,hopper}}/*");
    eprintln!("            (preserves *-keypair.json under target/deploy)");
    eprintln!("  --all:    above, plus `cargo clean` for a full target wipe");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn unique_tempdir(label: &str) -> std::path::PathBuf {
        // Hand-rolled tmpdir: avoids pulling in `tempfile` just for
        // these tests. Uses the process id + a per-test label so two
        // tests can't collide on the same machine.
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("hopper-clean-{label}-{pid}-{nanos}"));
        fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    #[test]
    fn deploy_clean_preserves_keypair_files() {
        let dir = unique_tempdir("preserve");
        let so = dir.join("my_program.so");
        let kp = dir.join("my_program-keypair.json");
        File::create(&so).unwrap().write_all(b"\x7fELF").unwrap();
        File::create(&kp).unwrap().write_all(b"[1,2,3]").unwrap();

        clean_deploy_dir(&dir).expect("clean deploy");

        assert!(!so.exists(), "expected .so to be deleted");
        assert!(kp.exists(), "expected keypair to be preserved");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deploy_clean_recurses_into_subdirs() {
        // cargo build-sbf can drop intermediate dirs (e.g. release/);
        // the cleaner should remove them when they're not keypair
        // files.
        let dir = unique_tempdir("subdirs");
        let nested = dir.join("release");
        fs::create_dir_all(&nested).unwrap();
        File::create(nested.join("blob.o")).unwrap();
        let kp = dir.join("p-keypair.json");
        File::create(&kp).unwrap();

        clean_deploy_dir(&dir).expect("clean deploy");

        assert!(!nested.exists(), "expected nested dir to be removed");
        assert!(kp.exists(), "expected keypair to be preserved");

        let _ = fs::remove_dir_all(&dir);
    }
}
