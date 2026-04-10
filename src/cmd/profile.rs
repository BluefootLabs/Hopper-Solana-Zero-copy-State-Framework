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

    eprintln!("Unknown profile subcommand: {}", args[0]);
    print_profile_usage();
    process::exit(1);
}

fn print_profile_usage() {
    eprintln!("Usage: hopper profile bench [options]");
    eprintln!();
    eprintln!("Run the Hopper primitive benchmark lab and emit JSON/CSV artifacts.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --rpc <url>                   RPC endpoint (default: SOLANA_RPC_URL or localhost)");
    eprintln!("  --keypair <path>             Fee payer keypair (default: ~/.config/solana/id.json)");
    eprintln!("  --out-dir <dir>              Output directory for JSON/CSV artifacts");
    eprintln!("  --program-id <pubkey>        Reuse an existing deployed hopper-bench program");
    eprintln!("  --no-build                   Reuse the current hopper-bench .so");
    eprintln!("  --no-deploy                  Skip deploy (requires --program-id)");
    eprintln!("  --fail-on-regression <pct>   Override tolerated regression percentage");
}
