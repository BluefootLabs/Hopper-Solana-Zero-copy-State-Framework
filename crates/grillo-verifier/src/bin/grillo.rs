//! Command-line interface for the offline Grillo effect verifier.
//!
//! Verification uses a manifest and caller-supplied evidence bundle without
//! RPC access. Verdicts are reproducible from those inputs, but the command
//! does not authenticate their producer or on-chain provenance.
//!
//! ```text
//! grillo commit <hopper.manifest.json>
//! grillo verify <hopper.manifest.json> <bundle.json>
//! grillo authority-diff <old.manifest.json> <new.manifest.json> [--json] [--out <report.json>] [--approve <report.json>]
//! ```
//!
//! Exit codes: 0 scoped PASS, 2 VIOLATION, 3 INCONCLUSIVE,
//! 1 usage / malformed input. `authority-diff` uses 0 NOT WIDENED,
//! 2 WIDENED, 3 REVIEW, each after any `--approve` file is applied.

use std::process::ExitCode;

use grillo_verifier::authority::{AuthorityDiff, AuthorityReport, AuthorityVerdict};
use grillo_verifier::{parse_bundle, verify_bundle, MutationManifest, Verdict};

fn usage() {
    eprintln!("grillo: offline byte-effect verifier for Hopper mutation contracts");
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
    eprintln!("  grillo authority-diff <old.manifest.json> <new.manifest.json>");
    eprintln!("               [--json] [--out <report.json>] [--approve <report.json>]");
    eprintln!("      Upgrade review: report every instruction that gains authority");
    eprintln!("      (dropped signer, new writable, wider byte ranges, removed PDA or");
    eprintln!("      has_one binding, new CPI program, lamport permission). An");
    eprintln!("      --approve file is a reviewed report for exactly this manifest pair.");
    eprintln!();
    eprintln!("Exit codes: 0 PASS, 2 VIOLATION, 3 INCONCLUSIVE, 1 input error.");
    eprintln!("authority-diff: 0 NOT WIDENED, 2 WIDENED, 3 REVIEW, 1 input error.");
}

fn authority_diff(args: &[String]) -> Result<ExitCode, String> {
    let mut paths = Vec::new();
    let mut json = false;
    let mut out = None;
    let mut approve = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--out" | "--approve" => {
                let flag = args[i].clone();
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("{flag} requires a path"))?;
                if flag == "--out" {
                    out = Some(value);
                } else {
                    approve = Some(value);
                }
            }
            other if other.starts_with("--") => return Err(format!("unknown flag `{other}`")),
            other => paths.push(other.to_string()),
        }
        i += 1;
    }
    let [old_path, new_path] = paths.as_slice() else {
        usage();
        return Err("authority-diff takes an old and a new manifest path".to_string());
    };
    let report = AuthorityDiff::between_json(&read(old_path)?, &read(new_path)?)
        .map_err(|e| format!("manifest rejected: {e}"))?;
    if let Some(out) = &out {
        std::fs::write(out, report.to_json()).map_err(|e| format!("write {out}: {e}"))?;
    }
    if json {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.render());
    }

    let verdict = report.verdict();
    if verdict == AuthorityVerdict::NotWidened {
        return Ok(ExitCode::SUCCESS);
    }
    let failing = ExitCode::from(if verdict == AuthorityVerdict::Widened {
        2
    } else {
        3
    });
    let Some(approve) = approve else {
        return Ok(failing);
    };
    let approval = AuthorityReport::from_json(&read(&approve)?)
        .map_err(|e| format!("approval rejected: {e}"))?;
    match report.check_approval(&approval) {
        Ok(()) => {
            eprintln!("approved: every widening is listed in {approve}");
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            eprintln!("not approved: {err}");
            Ok(failing)
        }
    }
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
        "authority-diff" => authority_diff(&args[1..]),
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
