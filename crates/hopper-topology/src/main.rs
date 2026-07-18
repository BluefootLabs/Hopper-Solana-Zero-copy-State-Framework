use std::env;
use std::fs;
use std::io::{self, Read as _};
use std::path::PathBuf;
use std::process;

use hopper_topology::{commit_profile, solve, WorkloadProfile};

fn usage() {
    eprintln!("Usage:");
    eprintln!("  hopper-topology solve <profile.json|-> [--out <plan.json>] [--compact]");
    eprintln!("  hopper-topology validate <profile.json|->");
}

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|error| format!("read stdin: {error}"))?;
        Ok(input)
    } else {
        fs::read_to_string(path).map_err(|error| format!("read {path}: {error}"))
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage();
        return Ok(());
    }
    let command = args[0].as_str();
    let Some(path) = args.get(1) else {
        usage();
        return Err("missing profile JSON path".to_string());
    };
    let json = read_input(path)?;
    let profile: WorkloadProfile = serde_json::from_str(&json)
        .map_err(|error| format!("strict profile JSON decode failed: {error}"))?;

    match command {
        "validate" => {
            if args.len() != 2 {
                return Err("validate accepts exactly one profile path".to_string());
            }
            let commitment = commit_profile(&profile).map_err(|error| error.to_string())?;
            println!("{commitment}");
            Ok(())
        }
        "solve" => {
            let mut out: Option<PathBuf> = None;
            let mut compact = false;
            let mut index = 2usize;
            while index < args.len() {
                match args[index].as_str() {
                    "--compact" => {
                        compact = true;
                        index += 1;
                    }
                    "--out" => {
                        let value = args
                            .get(index + 1)
                            .ok_or("--out requires a path")?;
                        out = Some(PathBuf::from(value));
                        index += 2;
                    }
                    other => return Err(format!("unknown solve argument `{other}`")),
                }
            }
            let plan = solve(profile).map_err(|error| error.to_string())?;
            let rendered = if compact {
                serde_json::to_string(&plan)
            } else {
                serde_json::to_string_pretty(&plan)
            }
            .map_err(|error| format!("serialize plan JSON: {error}"))?;
            if let Some(path) = out {
                fs::write(&path, rendered)
                    .map_err(|error| format!("write {}: {error}", path.display()))?;
            } else {
                println!("{rendered}");
            }
            Ok(())
        }
        other => {
            usage();
            Err(format!("unknown command `{other}`"))
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hopper-topology: {error}");
        process::exit(2);
    }
}
