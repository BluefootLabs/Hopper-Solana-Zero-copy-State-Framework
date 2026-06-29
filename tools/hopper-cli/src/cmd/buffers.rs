//! `hopper buffers` — buffer hygiene for stranded BPF Loader buffers.
//!
//! A failed `solana program deploy` strands rent in a dangling program buffer
//! owned by the deploy keypair. This command group is the safe, scriptable
//! recovery path on top of the Solana CLI:
//!
//! - `hopper buffers list` -> `solana program show --buffers`
//! - `hopper buffers close <BUFFER>` -> `solana program close <BUFFER>`
//! - `hopper buffers close --all` -> `solana program close --buffers`
//!
//! Closing is irreversible, so the close paths confirm unless `-y/--yes`. RPC
//! URLs are shown redacted (the confirmation prompt uses
//! [`ClusterArgs::display_url`]) so a custom endpoint's API key never lands in
//! a terminal log.

use std::path::PathBuf;

use crate::cmd::cluster::{parse_cluster_args, ClusterArgs};
use crate::cmd::lifecycle::{run_external_command, take_bare_flags, take_flag_values};
use crate::workspace;

/// Which buffers a `close` targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuffersClose {
    /// Close one specific buffer account.
    One(String),
    /// Close every dangling buffer owned by the configured keypair.
    All,
}

/// `hopper buffers <list|close>` dispatch.
pub fn cmd_buffers(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("list") => cmd_buffers_list(&args[1..]),
        Some("close") => cmd_buffers_close(&args[1..]),
        Some("--help") | Some("-h") | None => print_buffers_usage(),
        Some(other) => {
            eprintln!("hopper buffers: unknown subcommand `{other}`");
            print_buffers_usage();
            std::process::exit(1);
        }
    }
}

fn cmd_buffers_list(args: &[String]) {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_buffers_usage();
        return;
    }
    let cluster = parse_cluster_args(args).unwrap_or_else(|err| {
        eprintln!("hopper buffers list failed: {err}");
        std::process::exit(1);
    });
    let command = build_list_command(&cluster);
    run_external_command("solana", &workspace_root(), &command);
}

fn cmd_buffers_close(args: &[String]) {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_buffers_usage();
        return;
    }

    let mut rest = args.to_vec();
    let target = parse_buffers_close(&mut rest).unwrap_or_else(|err| {
        eprintln!("hopper buffers close failed: {err}");
        std::process::exit(1);
    });
    let cluster = parse_cluster_args(&rest).unwrap_or_else(|err| {
        eprintln!("hopper buffers close failed: {err}");
        std::process::exit(1);
    });

    // Closing a buffer reclaims its rent and is irreversible. Confirm unless -y.
    if !cluster.yes {
        let label = match &target {
            BuffersClose::One(buffer) => buffer.clone(),
            BuffersClose::All => "all dangling buffers".to_string(),
        };
        eprint!(
            "About to close {label} on {} ({}). This is irreversible. Type 'yes' to continue: ",
            cluster.label,
            cluster.display_url()
        );
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() || line.trim() != "yes" {
            eprintln!("aborted.");
            std::process::exit(1);
        }
    }

    let command = build_close_command(&target, &cluster);
    run_external_command("solana", &workspace_root(), &command);
}

fn workspace_root() -> PathBuf {
    let cwd = workspace::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    workspace::find_workspace_root(&cwd).unwrap_or(cwd)
}

/// Parse the close target: exactly one of `--all` (alias `--buffers`), a leading
/// positional buffer pubkey, or `--buffer <pubkey>`. The positional form must
/// precede any flags so it is never confused with a flag's value.
fn parse_buffers_close(args: &mut Vec<String>) -> Result<BuffersClose, String> {
    let all = take_bare_flags(args, &["--all", "--buffers"]);
    let mut buffers = take_flag_values(args, "--buffer")?;

    // A leading positional (no `-` prefix) also names a buffer, so
    // `hopper buffers close <PUBKEY>` works without `--buffer`.
    if let Some(first) = args.first() {
        if !first.starts_with('-') {
            buffers.push(args.remove(0));
        }
    }

    if all > 1 {
        return Err("--all may only be passed once".to_string());
    }
    if buffers.len() > 1 {
        return Err("pass only one buffer (or --all)".to_string());
    }

    let target_count = usize::from(all >= 1) + buffers.len();
    if target_count == 0 {
        return Err(
            "pass a buffer pubkey, --buffer <pubkey>, or --all (positional pubkey must precede flags)"
                .to_string(),
        );
    }
    if target_count > 1 {
        return Err("pass exactly one of a buffer pubkey, --buffer <pubkey>, or --all".to_string());
    }

    if all >= 1 {
        Ok(BuffersClose::All)
    } else {
        Ok(BuffersClose::One(
            buffers.into_iter().next().expect("one buffer proven above"),
        ))
    }
}

/// Build the `solana program show --buffers ...` command for `buffers list`.
pub(crate) fn build_list_command(cluster: &ClusterArgs) -> Vec<String> {
    let mut command = vec![
        "program".to_string(),
        "show".to_string(),
        "--buffers".to_string(),
    ];
    command.extend(cluster.solana_flags());
    command.extend(cluster.passthrough.iter().cloned());
    command
}

/// Build the `solana program close ...` command for the given target.
pub(crate) fn build_close_command(target: &BuffersClose, cluster: &ClusterArgs) -> Vec<String> {
    let mut command = vec!["program".to_string(), "close".to_string()];
    match target {
        BuffersClose::One(buffer) => command.push(buffer.clone()),
        BuffersClose::All => command.push("--buffers".to_string()),
    }
    command.extend(cluster.solana_flags());
    command.extend(cluster.passthrough.iter().cloned());
    command
}

fn print_buffers_usage() {
    eprintln!("hopper buffers — recover rent stranded in dangling BPF Loader buffers");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("  hopper buffers list [--cluster <name>] [--keypair <path>]");
    eprintln!("  hopper buffers close <BUFFER> [--cluster <name>] [--keypair <path>] [-y|--yes]");
    eprintln!("  hopper buffers close --all   [--cluster <name>] [--keypair <path>] [-y|--yes]");
    eprintln!();
    eprintln!("Notes:");
    eprintln!("  - `list` maps to `solana program show --buffers`.");
    eprintln!("  - `close` maps to `solana program close`. It is irreversible; pass -y to skip the prompt.");
    eprintln!("  - A positional buffer pubkey must come before any flags.");
    eprintln!("  - Custom RPC URLs are shown redacted so API keys never reach the terminal log.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    fn cluster(args: &[&str]) -> ClusterArgs {
        parse_cluster_args(&argv(args)).expect("cluster args parse")
    }

    #[test]
    fn parse_close_all_via_flag() {
        let mut a = argv(&["--all"]);
        assert_eq!(parse_buffers_close(&mut a).unwrap(), BuffersClose::All);
    }

    #[test]
    fn parse_close_all_via_buffers_alias() {
        let mut a = argv(&["--buffers"]);
        assert_eq!(parse_buffers_close(&mut a).unwrap(), BuffersClose::All);
    }

    #[test]
    fn parse_close_leading_positional_buffer_keeps_cluster_flags() {
        let mut a = argv(&["BuF111", "--cluster", "devnet"]);
        assert_eq!(
            parse_buffers_close(&mut a).unwrap(),
            BuffersClose::One("BuF111".to_string())
        );
        // Cluster flags survive for parse_cluster_args.
        assert_eq!(a, argv(&["--cluster", "devnet"]));
    }

    #[test]
    fn parse_close_buffer_flag() {
        let mut a = argv(&["--buffer", "BuF222"]);
        assert_eq!(
            parse_buffers_close(&mut a).unwrap(),
            BuffersClose::One("BuF222".to_string())
        );
    }

    #[test]
    fn parse_close_rejects_no_target() {
        // `--cluster devnet` is a flag + value, not a positional buffer.
        let mut a = argv(&["--cluster", "devnet"]);
        assert!(parse_buffers_close(&mut a).is_err());
    }

    #[test]
    fn parse_close_rejects_two_targets() {
        let mut a = argv(&["--all", "--buffer", "BuF"]);
        assert!(parse_buffers_close(&mut a).is_err());
    }

    #[test]
    fn parse_close_rejects_two_buffers() {
        let mut a = argv(&["--buffer", "A", "--buffer", "B"]);
        assert!(parse_buffers_close(&mut a).is_err());
    }

    #[test]
    fn build_list_command_is_program_show_buffers() {
        let c = cluster(&["--cluster", "devnet"]);
        let cmd = build_list_command(&c);
        assert_eq!(cmd[0], "program");
        assert_eq!(cmd[1], "show");
        assert_eq!(cmd[2], "--buffers");
        // Cluster target threads through as `--url`.
        assert!(cmd.iter().any(|a| a == "--url"));
    }

    #[test]
    fn build_close_one_is_program_close_buffer() {
        let c = cluster(&["--cluster", "devnet"]);
        let cmd = build_close_command(&BuffersClose::One("BuF333".to_string()), &c);
        assert_eq!(
            cmd[0..3],
            ["program".to_string(), "close".to_string(), "BuF333".to_string()]
        );
    }

    #[test]
    fn build_close_all_is_program_close_buffers() {
        let c = cluster(&["--cluster", "devnet"]);
        let cmd = build_close_command(&BuffersClose::All, &c);
        assert_eq!(
            cmd[0..3],
            [
                "program".to_string(),
                "close".to_string(),
                "--buffers".to_string()
            ]
        );
    }
}
