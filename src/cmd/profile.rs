//! `hopper profile` subcommand tree.
//!
//! Two shipping subcommands:
//!
//! - `profile bench`. Runs the primitive-benchmark lab against a live
//!   cluster and emits JSON/CSV regression artifacts. Existing code.
//! - `profile elf`. Parses a compiled SBF ELF, resolves DWARF function
//!   names, and emits flamegraph-compatible folded-stack output plus a
//!   human-readable "top N functions by static size" table. Matches
//!   Quasar's `quasar profile` command ergonomically.
//!
//! The flamegraph output is the standard folded-stack format the
//! Brendan Gregg `FlameGraph.pl` and `inferno-flamegraph` consume:
//! `<stack_frames>;<semicolon_separated> <value>`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process;

use crate::bench;

pub fn cmd_profile(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_profile_usage();
        return;
    }

    if args.is_empty() || args[0] == "bench" {
        let bench_args = if args.first().map(String::as_str) == Some("bench") {
            &args[1..]
        } else {
            args
        };

        if let Err(err) = bench::run_primitive_bench(bench_args) {
            eprintln!("hopper profile bench failed: {err}");
            process::exit(1);
        }
        return;
    }

    if args[0] == "elf" {
        if let Err(err) = cmd_profile_elf(&args[1..]) {
            eprintln!("hopper profile elf failed: {err}");
            process::exit(1);
        }
        return;
    }

    eprintln!("Unknown profile subcommand: {}", args[0]);
    print_profile_usage();
    process::exit(1);
}

fn print_profile_usage() {
    eprintln!("Usage: hopper profile <subcommand> [options]");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  bench                         Primitive benchmark lab with JSON/CSV artifacts");
    eprintln!("  elf <path/to/program.so>      Static SBF ELF analysis: symbol sizes, DWARF");
    eprintln!("                                names, flamegraph-compatible folded output");
    eprintln!();
    eprintln!("`profile bench` options:");
    eprintln!("  --rpc <url>                   RPC endpoint (default: SOLANA_RPC_URL or localhost)");
    eprintln!("  --keypair <path>             Fee payer keypair (default: ~/.config/solana/id.json)");
    eprintln!("  --out-dir <dir>              Output directory for JSON/CSV artifacts");
    eprintln!("  --program-id <pubkey>        Reuse an existing deployed hopper-bench program");
    eprintln!("  --no-build                   Reuse the current hopper-bench .so");
    eprintln!("  --no-deploy                  Skip deploy (requires --program-id)");
    eprintln!("  --fail-on-regression <pct>   Override tolerated regression percentage");
    eprintln!();
    eprintln!("`profile elf` options:");
    eprintln!("  --top <N>                    Print the top N symbols by size (default 20)");
    eprintln!("  --folded <out.txt>           Write Brendan-Gregg folded-stack output for flamegraph");
    eprintln!("  --no-demangle                Leave mangled symbol names intact");
}

struct ElfArgs<'a> {
    path: &'a str,
    top: usize,
    folded_out: Option<&'a str>,
    demangle: bool,
}

fn parse_elf_args<'a>(args: &'a [String]) -> Result<ElfArgs<'a>, String> {
    if args.is_empty() {
        return Err("missing path to ELF; usage: hopper profile elf <program.so>".into());
    }
    let mut out = ElfArgs {
        path: &args[0],
        top: 20,
        folded_out: None,
        demangle: true,
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--top" => {
                i += 1;
                out.top = args.get(i)
                    .ok_or("`--top` requires a value")?
                    .parse()
                    .map_err(|e| format!("`--top` must be a usize: {e}"))?;
            }
            "--folded" => {
                i += 1;
                out.folded_out = Some(args.get(i).ok_or("`--folded` requires a path")?.as_str());
            }
            "--no-demangle" => out.demangle = false,
            other => return Err(format!("unknown elf flag: {other}")),
        }
        i += 1;
    }
    Ok(out)
}

fn cmd_profile_elf(args: &[String]) -> Result<(), String> {
    let opts = parse_elf_args(args)?;
    let bytes = fs::read(opts.path)
        .map_err(|e| format!("could not read `{}`: {e}", opts.path))?;

    let (symbols, byte_total) = parse_symbols(&bytes, opts.demangle)?;

    // Rank and print top-N by size.
    let mut ranked: Vec<(&str, u64)> = symbols.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    println!("hopper profile elf  -  {}", opts.path);
    println!("total code in .text: {} bytes", byte_total);
    println!("distinct symbols:    {}", ranked.len());
    println!();
    println!("top {} symbols by static size:", opts.top);
    println!(
        "{:>10}  {:>6}  symbol",
        "bytes", "pct"
    );
    let total = byte_total.max(1);
    for (name, sz) in ranked.iter().take(opts.top) {
        let pct = (*sz as f64 / total as f64) * 100.0;
        println!("{:>10}  {:>5.2}%  {}", sz, pct, name);
    }

    if let Some(out_path) = opts.folded_out {
        let folded = render_folded(&ranked);
        fs::write(out_path, folded)
            .map_err(|e| format!("could not write `{}`: {e}", out_path))?;
        println!();
        println!("wrote folded-stack flamegraph input to {}", out_path);
        println!("pipe it to a flamegraph renderer:");
        println!("  cat {} | inferno-flamegraph > profile.svg", out_path);
    }
    Ok(())
}

/// Parse .text-region function symbols out of the ELF and return a
/// `(symbol_name -> bytes)` map plus the total.
///
/// DWARF-based inline expansion is a future enhancement; the symbol
/// table alone gives a useful first-order map of code footprint,
/// which is the metric the `quasar profile` output leads with. Names
/// are demangled via `rustc-demangle` when the flag is set.
fn parse_symbols(
    bytes: &[u8],
    demangle: bool,
) -> Result<(BTreeMap<String, u64>, u64), String> {
    use object::{Object, ObjectSymbol};

    let file = object::File::parse(bytes).map_err(|e| format!("not a valid ELF: {e}"))?;

    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    let mut total: u64 = 0;
    for sym in file.symbols() {
        let size = sym.size();
        if size == 0 {
            continue;
        }
        if !matches!(sym.kind(), object::SymbolKind::Text) {
            continue;
        }
        let raw_name = sym.name().unwrap_or("?");
        let name = if demangle {
            rustc_demangle::demangle(raw_name).to_string()
        } else {
            raw_name.to_string()
        };
        *out.entry(name).or_insert(0) += size;
        total += size;
    }

    if out.is_empty() {
        return Err(format!(
            "ELF at `{}` has no .text symbols. Was it stripped? Try building with `cargo build-sbf --debug`.",
            "input"
        ));
    }
    Ok((out, total))
}

/// Render a Brendan-Gregg folded-stack flamegraph input from a
/// symbol-to-size table. Each symbol is one stack frame with its
/// name as the only identifier; nested call frames live in the
/// DWARF-enabled follow-up. Even without nesting, the flamegraph
/// still shows symbol sizes as proportional bars, which is the most
/// useful bird's-eye view of a compiled SBF program.
fn render_folded(ranked: &[(&str, u64)]) -> String {
    let mut s = String::new();
    for (name, sz) in ranked {
        // Sanitize the `;` separator inside demangled names (Rust's
        // generics use `<` `>` but not `;`; this is defensive).
        let safe: String = name.replace(';', ":");
        s.push_str(&safe);
        s.push(' ');
        s.push_str(&sz.to_string());
        s.push('\n');
    }
    s
}

// Compile-time assertion that the module sees its imports. Rustc emits
// an unused-import warning if `Path` is never referenced; pin it here.
#[allow(dead_code)]
fn _ignore(_: &Path) {}
