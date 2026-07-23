//! `grillo` — the independent effect verifier, as a command.
//!
//! Everything is offline and reproducible: a manifest, an evidence
//! bundle, byte arithmetic, a verdict. No RPC, no trust in the producer.
//!
//! ```text
//! grillo commit <hopper.manifest.json>
//! grillo verify <hopper.manifest.json> <bundle.json>
//! ```
//!
//! Exit codes: 0 scoped PASS, 2 VIOLATION, 3 INCONCLUSIVE,
//! 1 usage / malformed input.

use std::process::ExitCode;

use grillo_verifier::{parse_bundle, verify_bundle, MutationManifest, Verdict};

fn usage() {
    eprintln!("grillo — independent byte-effect verifier for Hopper mutation contracts");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  grillo commit <hopper.manifest.json>");
    eprintln!("      Print the SHA-256 mutation-contract commitment for every");
    eprintln!("      instruction (domain grillo.mutation-contract.v2).");
    eprintln!();
    eprintln!("  grillo verify <hopper.manifest.json> <bundle.json>");
    eprintln!("      Verify an offline evidence bundle (pre/post account hex,");
    eprintln!("      touch-map blob, optional argument payload) against the");
    eprintln!("      published contract: changed ⊆ acquired ⊆ authorized.");
    eprintln!();
    eprintln!("Exit codes: 0 PASS, 2 VIOLATION, 3 INCONCLUSIVE, 1 input error.");
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))
}

fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
        return Ok(ExitCode::from(1));
    }
    match args[0].as_str() {
        "commit" => {
            let [_, manifest_path] = args.as_slice() else {
                usage();
                return Err("commit takes exactly one manifest path".to_string());
            };
            let manifest = MutationManifest::from_json(&read(manifest_path)?)
                .map_err(|e| format!("manifest rejected: {e}"))?;
            println!(
                "program: {} v{}",
                manifest.program_name, manifest.program_version
            );
            for instruction in &manifest.instructions {
                println!(
                    "  {:<28} tag {:>3}  {}",
                    instruction.name,
                    instruction.tag,
                    hex32(&instruction.commitment()),
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        "verify" => {
            let [_, manifest_path, bundle_path] = args.as_slice() else {
                usage();
                return Err("verify takes a manifest path and a bundle path".to_string());
            };
            let manifest = MutationManifest::from_json(&read(manifest_path)?)
                .map_err(|e| format!("manifest rejected: {e}"))?;
            let bundle =
                parse_bundle(&read(bundle_path)?).map_err(|e| format!("bundle rejected: {e}"))?;
            let verdict = verify_bundle(&manifest, &bundle)
                .map_err(|e| format!("verification aborted: {e}"))?;
            print!("{}", verdict.render());
            Ok(match verdict {
                Verdict::Pass(_) => ExitCode::SUCCESS,
                Verdict::Violation(_) => ExitCode::from(2),
                Verdict::Inconclusive(_) => ExitCode::from(3),
            })
        }
        other => {
            usage();
            Err(format!("unknown command `{other}`"))
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("grillo: {message}");
            ExitCode::from(1)
        }
    }
}
