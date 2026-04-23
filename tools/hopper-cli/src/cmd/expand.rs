//! `hopper expand` - wrap `cargo expand` with Hopper-aware defaults.
//!
//! `cargo expand` lives upstream. Hopper's value-add is:
//!
//! 1. A single, well-documented entry point next to every other
//!    `hopper <verb>` command so users do not need to remember the
//!    upstream name.
//! 2. Auto-install. When `cargo expand` is missing, we offer to
//!    `cargo install cargo-expand` so the first-time experience does
//!    not dead-end.
//! 3. Default target selection. For a workspace that ships a Hopper
//!    program, we expand the current crate with standard Hopper
//!    features enabled (`proc-macros` and the native backend) so the
//!    output shows the macros users actually care about.
//! 4. Filter by symbol. `--filter vault` walks the expanded output
//!    and prints just the functions, structs, and impls whose name
//!    matches the substring. Mirrors Quasar's `quasar profile --expand`.
//!
//! Every unknown flag passes straight to `cargo expand`, so the
//! upstream flag surface is reachable.

use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::workspace;

pub fn cmd_expand(args: &[String]) {
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        print_usage();
        return;
    }

    let Some(cwd) = workspace::current_dir().ok() else {
        eprintln!("could not resolve current directory");
        std::process::exit(1);
    };
    let project_root: PathBuf = workspace::find_project_root(&cwd).unwrap_or(cwd);

    if !is_cargo_expand_installed() {
        eprintln!("cargo-expand is not installed.");
        eprintln!("run `cargo install cargo-expand` and re-run `hopper expand`,");
        eprintln!("or pass `--install` to have hopper install it for you.");
        if !args.iter().any(|a| a == "--install") {
            std::process::exit(1);
        }
        if !install_cargo_expand() {
            std::process::exit(1);
        }
    }

    // Pull out Hopper-specific flags before delegating.
    let mut passthrough: Vec<String> = Vec::with_capacity(args.len());
    let mut filter: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--install" => {
                // Already handled above.
            }
            "--filter" => {
                i += 1;
                filter = args.get(i).cloned();
            }
            other => passthrough.push(other.to_string()),
        }
        i += 1;
    }

    // Run `cargo expand` with a sensible default feature set when the
    // user did not already pass one. The `--features` flag being
    // absent is a hint that the caller wants the default Hopper DX.
    let want_default_features = !passthrough
        .iter()
        .any(|a| a == "--features" || a.starts_with("--features="));
    let mut cmd = Command::new("cargo");
    cmd.arg("expand");
    cmd.current_dir(&project_root);
    if want_default_features {
        cmd.arg("--features").arg("proc-macros");
    }
    for a in &passthrough {
        cmd.arg(a);
    }

    if let Some(substring) = filter {
        // Capture stdout so we can filter it, pipe stderr through
        // verbatim so the user still sees cargo's progress bar.
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::inherit());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("failed to spawn cargo expand: {e}");
                std::process::exit(1);
            }
        };
        let stdout = child.stdout.take().expect("cargo stdout piped above");
        filter_stream(stdout, &substring);
        let status = child.wait().expect("wait on cargo expand");
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    } else {
        // No filter: stream stdout/stderr straight through.
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());
        let status = match cmd.status() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("failed to run cargo expand: {e}");
                std::process::exit(1);
            }
        };
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

fn print_usage() {
    eprintln!("Usage: hopper expand [cargo-expand args...] [--filter <substring>]");
    eprintln!();
    eprintln!("Expand `#[hopper::*]` (and every other proc-macro) in the current");
    eprintln!("crate and print the result. A thin Hopper-flavoured wrapper over");
    eprintln!("`cargo expand` with sensible defaults.");
    eprintln!();
    eprintln!("Hopper-specific flags:");
    eprintln!("  --filter <substring>   Only emit items whose spelling contains");
    eprintln!("                         <substring>. Matches struct, fn, and impl");
    eprintln!("                         headers.");
    eprintln!("  --install              Install cargo-expand if it is missing.");
    eprintln!();
    eprintln!("All other flags forward to `cargo expand` unchanged.");
}

fn is_cargo_expand_installed() -> bool {
    let Ok(output) = Command::new("cargo").arg("expand").arg("--version").output() else {
        return false;
    };
    output.status.success()
}

fn install_cargo_expand() -> bool {
    eprintln!("running: cargo install cargo-expand");
    let status = Command::new("cargo")
        .arg("install")
        .arg("cargo-expand")
        .status();
    match status {
        Ok(s) if s.success() => true,
        Ok(s) => {
            eprintln!("cargo install cargo-expand exited {s}");
            false
        }
        Err(e) => {
            eprintln!("failed to spawn cargo install: {e}");
            false
        }
    }
}

/// Forward every line whose containing block matches `substring`.
///
/// "Block" is defined by brace-depth: we keep buffering a chunk until
/// we reach depth zero, then emit the buffer iff it contains the
/// substring. Works without a Rust parser because cargo-expand output
/// is already pretty-printed.
fn filter_stream<R: io::Read>(stream: R, substring: &str) {
    let reader = BufReader::new(stream);
    let mut buf: Vec<String> = Vec::new();
    let mut depth: i32 = 0;

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        buf.push(line.clone());
        for c in line.chars() {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
            }
        }
        if depth <= 0 {
            let chunk = buf.join("\n");
            if chunk.contains(substring) {
                println!("{chunk}");
            }
            buf.clear();
            if depth < 0 {
                depth = 0;
            }
        }
    }
    // Trailing fragment (should not happen, but flush for safety).
    if !buf.is_empty() {
        let chunk = buf.join("\n");
        if chunk.contains(substring) {
            println!("{chunk}");
        }
    }
}
