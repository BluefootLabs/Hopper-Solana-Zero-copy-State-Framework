//! `hopper version`, `hopper completions`, `hopper tx simulate`,
//! `hopper tx submit`, `hopper manager accounts read`.
//!
//! Small-but-load-bearing polish commands. Grouped in one file so
//! they can share the RPC helper set without a new module each.

use std::fs;
use std::process;

/// `hopper version` - print the CLI build info.
pub fn cmd_version(_args: &[String]) {
    println!("hopper {}", env!("CARGO_PKG_VERSION"));
    println!("hopper-schema linked: {}", hopper_schema::MANIFEST_VERSION);
    if let Some(sha) = option_env!("HOPPER_GIT_SHA") {
        println!("git sha: {sha}");
    }
    // Rust toolchain + target triple, so bug reports carry enough
    // context. env!("TARGET") is not stable; we fall back to the
    // host triple baked by rustc.
    if let Some(t) = option_env!("TARGET") {
        println!("target : {t}");
    }
}

/// `hopper completions <shell>` - emit shell-completion script.
///
/// Three shells: bash, zsh, fish. We hand-generate instead of
/// pulling clap-complete because the CLI is already hand-rolled and
/// the completion set is short enough to maintain here.
pub fn cmd_completions(args: &[String]) {
    let Some(shell) = args.first() else {
        print_completions_usage();
        process::exit(1);
    };
    match shell.as_str() {
        "bash" => print!("{}", BASH_COMPLETION),
        "zsh" => print!("{}", ZSH_COMPLETION),
        "fish" => print!("{}", FISH_COMPLETION),
        other => {
            eprintln!("unsupported shell: {other}");
            print_completions_usage();
            process::exit(1);
        }
    }
}

fn print_completions_usage() {
    eprintln!("Usage: hopper completions <bash | zsh | fish>");
    eprintln!();
    eprintln!("Emit a shell-completion script. Save it to your rc file or sourcing dir:");
    eprintln!("  bash  ->  hopper completions bash > /etc/bash_completion.d/hopper");
    eprintln!("  zsh   ->  hopper completions zsh  > \"${{fpath[1]}}/_hopper\"");
    eprintln!("  fish  ->  hopper completions fish > ~/.config/fish/completions/hopper.fish");
}

const TOP_LEVEL: &[&str] = &[
    "schema",
    "compile",
    "inspect",
    "explain",
    "client",
    "profile",
    "fetch",
    "init",
    "build",
    "test",
    "deploy",
    "dump",
    "verify",
    "keys",
    "config",
    "lint",
    "expand",
    "tx",
    "manager",
    "doctor",
    "completions",
    "version",
    "help",
];

const BASH_COMPLETION: &str = r#"_hopper() {
    local cur prev words cword
    _init_completion || return
    if [ "$cword" -eq 1 ]; then
        COMPREPLY=($(compgen -W "schema compile inspect explain client profile fetch init build test deploy dump verify keys config lint expand tx manager doctor completions version help" -- "$cur"))
        return
    fi
    case "${words[1]}" in
        keys) COMPREPLY=($(compgen -W "new list print pda" -- "$cur")) ;;
        config) COMPREPLY=($(compgen -W "get set list reset path" -- "$cur")) ;;
        tx) COMPREPLY=($(compgen -W "explain simulate submit" -- "$cur")) ;;
        manager) COMPREPLY=($(compgen -W "fetch summary identify decode instruction layouts policies events fingerprints compat receipt explain diff simulate invoke crank accounts interactive" -- "$cur")) ;;
        profile) COMPREPLY=($(compgen -W "bench elf" -- "$cur")) ;;
        schema) COMPREPLY=($(compgen -W "export validate diff" -- "$cur")) ;;
        completions) COMPREPLY=($(compgen -W "bash zsh fish" -- "$cur")) ;;
    esac
}
complete -F _hopper hopper
"#;

const ZSH_COMPLETION: &str = r#"#compdef hopper
_hopper() {
    local -a commands
    commands=(
        'schema:manifest export/validate/diff'
        'compile:emit lowered Rust'
        'inspect:decode account bytes'
        'explain:narrate account/receipt/compat'
        'client:generate TypeScript/Kotlin/Python clients'
        'profile:bench or ELF flamegraph'
        'fetch:pull on-chain manifest'
        'init:scaffold a Hopper project'
        'build:compile (optionally --watch)'
        'test:run tests (optionally --watch)'
        'deploy:deploy an SBF artifact'
        'dump:disassemble .so'
        'verify:ABI fingerprint check'
        'keys:key + PDA helpers'
        'config:global config store'
        'lint:account-relationship checker'
        'expand:macro expansion'
        'tx:on-chain transaction helpers'
        'manager:on-chain introspection + invoke + crank'
        'doctor:environment sanity check'
        'completions:emit shell completions'
        'version:print CLI version info'
        'help:print top-level usage'
    )
    _describe -t commands 'hopper command' commands
}
_hopper
"#;

const FISH_COMPLETION: &str = r#"complete -c hopper -f
complete -c hopper -n '__fish_use_subcommand' -a 'schema' -d 'manifest export/validate/diff'
complete -c hopper -n '__fish_use_subcommand' -a 'compile' -d 'emit lowered Rust'
complete -c hopper -n '__fish_use_subcommand' -a 'inspect' -d 'decode account bytes'
complete -c hopper -n '__fish_use_subcommand' -a 'explain' -d 'narrate account/receipt/compat'
complete -c hopper -n '__fish_use_subcommand' -a 'client' -d 'generate TS/KT/PY clients'
complete -c hopper -n '__fish_use_subcommand' -a 'profile' -d 'bench or ELF flamegraph'
complete -c hopper -n '__fish_use_subcommand' -a 'fetch' -d 'pull on-chain manifest'
complete -c hopper -n '__fish_use_subcommand' -a 'init' -d 'scaffold a Hopper project'
complete -c hopper -n '__fish_use_subcommand' -a 'build' -d 'compile (optionally --watch)'
complete -c hopper -n '__fish_use_subcommand' -a 'test' -d 'run tests (optionally --watch)'
complete -c hopper -n '__fish_use_subcommand' -a 'deploy' -d 'deploy an SBF artifact'
complete -c hopper -n '__fish_use_subcommand' -a 'dump' -d 'disassemble .so'
complete -c hopper -n '__fish_use_subcommand' -a 'verify' -d 'ABI fingerprint check'
complete -c hopper -n '__fish_use_subcommand' -a 'keys' -d 'key + PDA helpers'
complete -c hopper -n '__fish_use_subcommand' -a 'config' -d 'global config store'
complete -c hopper -n '__fish_use_subcommand' -a 'lint' -d 'account-relationship checker'
complete -c hopper -n '__fish_use_subcommand' -a 'expand' -d 'macro expansion'
complete -c hopper -n '__fish_use_subcommand' -a 'tx' -d 'on-chain transaction helpers'
complete -c hopper -n '__fish_use_subcommand' -a 'manager' -d 'on-chain introspection + invoke + crank'
complete -c hopper -n '__fish_use_subcommand' -a 'doctor' -d 'environment sanity check'
complete -c hopper -n '__fish_use_subcommand' -a 'completions' -d 'emit shell completions'
complete -c hopper -n '__fish_use_subcommand' -a 'version' -d 'print CLI version info'
"#;

/// `hopper tx simulate <tx-base64>` - simulate a pre-built tx.
///
/// Useful for tooling pipelines that build transactions elsewhere
/// and want Hopper's simulation output (CU + logs + err).
pub fn cmd_tx_simulate(args: &[String]) {
    tx_simulate_or_submit(args, false);
}

/// `hopper tx submit <tx-base64>` - submit a pre-built tx.
pub fn cmd_tx_submit(args: &[String]) {
    tx_simulate_or_submit(args, true);
}

fn tx_simulate_or_submit(args: &[String], send: bool) {
    use solana_client::rpc_client::RpcClient;
    use solana_client::rpc_config::RpcSimulateTransactionConfig;
    use solana_sdk::commitment_config::CommitmentConfig;
    use solana_sdk::transaction::Transaction;

    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        let verb = if send { "submit" } else { "simulate" };
        eprintln!("Usage: hopper tx {verb} <tx-base64> [--rpc <url>]");
        return;
    }
    let tx_b64 = &args[0];
    let mut rpc: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" => {
                i += 1;
                rpc = args.get(i).cloned();
            }
            other => {
                eprintln!("unknown flag: {other}");
                process::exit(1);
            }
        }
        i += 1;
    }
    let rpc_url = rpc.unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));

    let bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        tx_b64,
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("base64 decode: {e}");
            process::exit(1);
        }
    };
    let tx: Transaction = match bincode::deserialize(&bytes) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("bincode decode (transaction): {e}");
            process::exit(1);
        }
    };
    let client =
        RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    if send {
        match client.send_and_confirm_transaction(&tx) {
            Ok(sig) => {
                println!("signature: {sig}");
                println!("status   : confirmed");
            }
            Err(e) => {
                eprintln!("send_and_confirm: {e}");
                process::exit(1);
            }
        }
    } else {
        let cfg = RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            commitment: Some(CommitmentConfig::processed()),
            ..Default::default()
        };
        match client.simulate_transaction_with_config(&tx, cfg) {
            Ok(res) => {
                if let Some(err) = res.value.err {
                    println!("simulation failed: {err:?}");
                } else {
                    println!("simulation: ok");
                }
                if let Some(cu) = res.value.units_consumed {
                    println!("units     : {cu}");
                }
                if let Some(logs) = res.value.logs {
                    println!("logs:");
                    for log in logs {
                        println!("  {log}");
                    }
                }
            }
            Err(e) => {
                eprintln!("simulate: {e}");
                process::exit(1);
            }
        }
    }
}

/// `hopper manager accounts read <pubkey>` - fetch, decode, print one
/// account against a manifest.
pub fn cmd_manager_accounts_read(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        eprintln!("Usage: hopper manager accounts read <pubkey> [--rpc <url>] [--manifest <path> | --program-id <id>]");
        return;
    }
    let pubkey = &args[0];
    let mut rpc: Option<String> = None;
    let mut manifest_path: Option<String> = None;
    let mut program_id: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" => {
                i += 1;
                rpc = args.get(i).cloned();
            }
            "--manifest" => {
                i += 1;
                manifest_path = args.get(i).cloned();
            }
            "--program-id" => {
                i += 1;
                program_id = args.get(i).cloned();
            }
            other => {
                eprintln!("unknown flag: {other}");
                process::exit(1);
            }
        }
        i += 1;
    }
    let rpc_url = rpc.unwrap_or_else(|| crate::rpc::resolve_rpc_url(None));
    let manifest_json = match manifest_path {
        Some(p) => match fs::read_to_string(&p) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("read {p}: {e}");
                process::exit(1);
            }
        },
        None => match program_id.as_deref() {
            Some(pid) => match super::manager_invoke::try_fetch_manifest(&rpc_url, pid) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("fetch manifest: {e}");
                    process::exit(1);
                }
            },
            None => {
                eprintln!("supply either --manifest <path> or --program-id <id>");
                process::exit(1);
            }
        },
    };
    let info = match crate::rpc::get_account_info(&rpc_url, pubkey) {
        Ok(Some(info)) => info,
        Ok(None) => {
            eprintln!("account {pubkey} does not exist");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("get_account_info: {e}");
            process::exit(1);
        }
    };
    println!("pubkey   : {pubkey}");
    println!("owner    : {}", info.owner);
    println!("lamports : {}", info.lamports);
    println!("data len : {} bytes", info.data.len());
    if info.data.is_empty() {
        return;
    }
    let disc = info.data[0];
    println!("disc     : 0x{:02x}", disc);
    match layout_name_by_disc(&manifest_json, disc) {
        Some(name) => println!("layout   : {name}"),
        None => println!("layout   : (no match in manifest for disc 0x{:02x})", disc),
    }
    let (version, layout_id) = if info.data.len() >= 16 {
        let ver = info.data[1];
        let mut id = [0u8; 8];
        id.copy_from_slice(&info.data[8..16]);
        (Some(ver), Some(id))
    } else {
        (None, None)
    };
    if let Some(v) = version {
        println!("version  : {v}");
    }
    if let Some(id) = layout_id {
        println!(
            "layout_id: {}",
            id.iter().map(|b| format!("{:02x}", b)).collect::<String>()
        );
    }
}

fn layout_name_by_disc(manifest_json: &str, disc: u8) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(manifest_json).ok()?;
    let layouts = v.get("layouts")?.as_array()?;
    for l in layouts {
        let d = l.get("disc").and_then(|x| x.as_u64())? as u8;
        if d == disc {
            return l
                .get("name")
                .and_then(|x| x.as_str())
                .map(String::from);
        }
    }
    None
}
