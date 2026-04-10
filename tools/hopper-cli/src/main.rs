//! # Hopper CLI
//!
//! Command-line tool for inspecting Hopper accounts, comparing layout
//! schemas, generating migration plans, and exporting schema metadata.
//!
//! ## Command Families
//!
//! ```text
//! hopper schema export [--manifest|--idl|--codama]  Export schema in various formats
//! hopper schema validate <manifest-json>            Validate a manifest
//! hopper schema diff <old> <new>                    Field-level diff
//!
//! hopper inspect <hex-data>                         Decode account header
//! hopper inspect segments <hex-data>                Decode segment map
//! hopper inspect receipt <hex-data>                 Decode a state receipt
//!
//! hopper explain <hex-data>                         Human-readable account explanation
//! hopper explain account <hex-data>                  Explicit account explanation
//! hopper explain receipt <hex-data>                  Explain a receipt in plain English
//! hopper explain compat <old> <new>                  Explain compatibility
//! hopper explain policy <policy-pack>                Explain a named policy pack
//! hopper explain layout <manifest>                   Explain layout fields and fingerprint
//! hopper explain program <manifest>                  Explain entire program pipeline
//!
//! hopper compat <old> <new>                         Compatibility report
//! hopper compat --why <old> <new>                   Compatibility with explanations
//!
//! hopper plan <old> <new>                           Generate migration plan
//!
//! hopper receipt <hex-data>                         Decode and display receipt
//!
//! hopper manager <subcommand> ...                   Program management
//!
//! hopper fetch <program-id>                          Fetch on-chain manifest
//!
//! hopper init <path>                                 Create a Hopper-native project scaffold
//! hopper build [--host|--sbf]                        Build the current project
//! hopper test                                        Run host-side tests for the current project
//! hopper profile bench                               Run the primitive benchmark lab
//!
//! hopper interactive <manifest>                      Interactive terminal explorer
//!
//! hopper client gen --ts <manifest>                 Generate TypeScript client
//! hopper client gen --kt <manifest>                 Generate Kotlin client
//! ```
//!
//! Hex data is passed as a hex string (no 0x prefix).
//! Manifest arguments accept inline JSON or `@path/to/file.json`.

use hopper_schema::{
    DecodedHeader, FieldCompat, FieldDescriptor, FieldIntent, LayoutFingerprint,
    LayoutManifest,
    MigrationAction, MigrationPlan, MigrationPolicy,
    SegmentMigrationReport, SegmentRoleHint,
    CompatibilityVerdict,
    compare_fields, decode_header, decode_segments,
    // Manager types
    AccountEntry, ArgDescriptor, EventDescriptor,
    InstructionDescriptor, PolicyDescriptor, ProgramManifest,
    decode_account_fields,
    // Receipt types (re-exported from hopper-core)
    CompatImpact, DecodedReceipt, Phase,
};
use hopper_schema::clientgen::{TsClientGen, KtClientGen};
use std::env;
use std::process;

mod bench;
mod cmd;
mod rpc;
mod interactive;
mod workspace;

/// Decode a Hopper header or exit with a diagnostic message.
fn require_header(data: &[u8]) -> DecodedHeader {
    match decode_header(data) {
        Some(h) => h,
        None => {
            eprintln!("Failed to decode Hopper header (data too short: {} bytes).", data.len());
            process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        // Command families
        "schema" => cmd_schema_family(&args[2..]),
        "inspect" => cmd_inspect_family(&args[2..]),
        "explain" => cmd_explain_family(&args[2..]),
        "client" => cmd_client_family(&args[2..]),
        "profile" => cmd::profile::cmd_profile(&args[2..]),

        // On-chain fetch
        "fetch" => cmd_fetch(&args[2..]),

        // Lifecycle
        "init" => cmd::lifecycle::cmd_init(&args[2..]),
        "build" => cmd::lifecycle::cmd_build(&args[2..]),
        "test" => cmd::lifecycle::cmd_test(&args[2..]),

        // Direct commands (backward compatible)
        "decode" => cmd_inspect(&args[2..]),
        "segments" => cmd_segments(&args[2..]),
        "receipt" => cmd_receipt(&args[2..]),
        "compat" => cmd_compat(&args[2..]),
        "diff" => cmd_diff(&args[2..]),
        "plan" => cmd_plan(&args[2..]),
        "schema-export" => cmd_schema_export(),
        "manager" => cmd_manager(&args[2..]),
        "interactive" | "ui" => cmd_interactive(&args[2..]),
        "help" | "--help" | "-h" => print_usage(),
        other => {
            eprintln!("Unknown command: {}", other);
            print_usage();
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Command Family Routers
// ---------------------------------------------------------------------------

fn cmd_schema_family(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper schema <subcommand>");
        eprintln!();
        eprintln!("Subcommands:");
        eprintln!("  export [--manifest|--idl|--codama]  Export schema format reference");
        eprintln!("  validate <manifest-json>            Validate a program manifest");
        eprintln!("  diff <old-json> <new-json>          Field-level diff between versions");
        process::exit(1);
    }
    match args[0].as_str() {
        "export" => cmd_schema_export_family(&args[1..]),
        "validate" => cmd_schema_validate(&args[1..]),
        "diff" => cmd_diff(&args[1..]),
        other => {
            eprintln!("Unknown schema subcommand: {}", other);
            process::exit(1);
        }
    }
}

fn cmd_inspect_family(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper inspect <hex-data|subcommand>");
        eprintln!();
        eprintln!("  hopper inspect <hex-data>            Decode account header");
        eprintln!("  hopper inspect segments <hex-data>   Decode segment map");
        eprintln!("  hopper inspect receipt <hex-data>    Decode a state receipt");
        process::exit(1);
    }
    match args[0].as_str() {
        "segments" => cmd_segments(&args[1..]),
        "receipt" => cmd_receipt(&args[1..]),
        _ => cmd_inspect(args), // treat first arg as hex data
    }
}

fn cmd_explain_family(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper explain <hex-data|subcommand>");
        eprintln!();
        eprintln!("  hopper explain <hex-data>            Human-readable account explanation");
        eprintln!("  hopper explain account <hex-data>    Explicit account explanation");
        eprintln!("  hopper explain receipt <hex-data>    Explain a receipt in plain English");
        eprintln!("  hopper explain compat <old> <new>    Explain compatibility report");
        eprintln!("  hopper explain policy <pack-name>    Explain a named policy pack");
        eprintln!("  hopper explain layout <manifest>     Explain layout fields, intents, and fingerprint");
        eprintln!("  hopper explain program <manifest>    Explain an entire program from its manifest");
        eprintln!("  hopper explain context <manifest>    Explain instruction contexts (accounts, roles, policies)");
        process::exit(1);
    }
    match args[0].as_str() {
        "account" => cmd_explain(&args[1..]),
        "receipt" => cmd_explain_receipt(&args[1..]),
        "compat" => cmd_explain_compat(&args[1..]),
        "policy" => cmd_explain_policy(&args[1..]),
        "layout" => cmd_explain_layout(&args[1..]),
        "program" => cmd_explain_program(&args[1..]),
        "context" => cmd_explain_context(&args[1..]),
        _ => cmd_explain(args), // treat first arg as hex data
    }
}

fn cmd_client_family(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper client gen [--ts|--kt] <manifest-json>");
        eprintln!();
        eprintln!("Subcommands:");
        eprintln!("  gen --ts <manifest>   Generate TypeScript client SDK");
        eprintln!("  gen --kt <manifest>   Generate Kotlin client SDK");
        process::exit(1);
    }
    match args[0].as_str() {
        "gen" => cmd_client_gen(&args[1..]),
        other => {
            eprintln!("Unknown client subcommand: {}", other);
            process::exit(1);
        }
    }
}

fn cmd_client_gen(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper client gen [--ts|--kt] <manifest-json>");
        process::exit(1);
    }

    let (lang, manifest_arg) = if args[0].starts_with("--") {
        if args.len() < 2 {
            eprintln!("Usage: hopper client gen [--ts|--kt] <manifest-json>");
            process::exit(1);
        }
        (args[0].as_str(), &args[1])
    } else {
        // Default to TypeScript
        ("--ts", &args[0])
    };

    let manifest = load_program_manifest(manifest_arg);

    match lang {
        "--ts" => {
            println!("{}", TsClientGen(&manifest));
        }
        "--kt" => {
            println!("{}", KtClientGen(&manifest));
        }
        other => {
            eprintln!("Unknown language flag: {}. Use --ts or --kt.", other);
            process::exit(1);
        }
    }
}

fn cmd_schema_validate(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper schema validate <manifest-json>");
        process::exit(1);
    }
    let manifest = load_program_manifest(&args[0]);
    println!("=== Manifest Validation ===");
    println!();
    println!("  Program: {} v{}", manifest.name, manifest.version);
    println!("  Layouts: {}", manifest.layouts.len());
    println!("  Instructions: {}", manifest.instructions.len());
    println!("  Events: {}", manifest.events.len());
    println!("  Policies: {}", manifest.policies.len());
    println!();

    let mut errors = 0u32;
    // Check layouts have unique discriminators
    for (i, l1) in manifest.layouts.iter().enumerate() {
        for l2 in manifest.layouts[i+1..].iter() {
            if l1.disc == l2.disc {
                println!("  ERROR: Duplicate discriminator {} for {} and {}", l1.disc, l1.name, l2.name);
                errors += 1;
            }
        }
    }
    // Check instructions have unique tags
    for (i, ix1) in manifest.instructions.iter().enumerate() {
        for ix2 in manifest.instructions[i+1..].iter() {
            if ix1.tag == ix2.tag {
                println!("  ERROR: Duplicate instruction tag {} for {} and {}", ix1.tag, ix1.name, ix2.name);
                errors += 1;
            }
        }
    }
    // Check events have unique tags
    for (i, e1) in manifest.events.iter().enumerate() {
        for e2 in manifest.events[i+1..].iter() {
            if e1.tag == e2.tag {
                println!("  ERROR: Duplicate event tag {} for {} and {}", e1.tag, e1.name, e2.name);
                errors += 1;
            }
        }
    }

    if errors == 0 {
        println!("  VALID: No errors found.");
    } else {
        println!();
        println!("  {} error(s) found.", errors);
        process::exit(1);
    }
}

fn cmd_explain_receipt(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper explain receipt <hex-data>");
        process::exit(1);
    }
    let data = match hex_decode(&args[0]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Hex decode error: {}", e);
            process::exit(1);
        }
    };
    if data.len() < RECEIPT_WIRE_SIZE {
        eprintln!("Receipt data too short.");
        process::exit(1);
    }

    let changed_bytes = u32::from_le_bytes(data[16..20].try_into().expect("slice length mismatch"));
    let flags = data[32];
    let was_resized = flags & (1 << 0) != 0;
    let invariants_passed = flags & (1 << 1) != 0;
    let cpi_invoked = flags & (1 << 2) != 0;
    let committed = flags & (1 << 3) != 0;
    let before_fp = &data[33..41];
    let after_fp = &data[41..49];
    let phase = data[58];
    let compat_impact = data[61];
    let migration_flags = data[62];

    let phase_name = match phase {
        1 => "initialization",
        2 => "close",
        3 => "migration",
        4 => "read-only inspection",
        _ => "update",
    };

    println!("=== Receipt Explanation ===");
    println!();
    if !committed {
        println!("  This receipt was NOT committed. The mutation was started but not finalized.");
        return;
    }
    println!("  This receipt records a state {} operation.", phase_name);
    if before_fp == after_fp {
        println!("  The account data was NOT changed (fingerprints match).");
    } else {
        println!("  The account data WAS changed ({} bytes modified).", changed_bytes);
    }
    if was_resized {
        let old_size = u32::from_le_bytes(data[22..26].try_into().expect("slice length mismatch"));
        let new_size = u32::from_le_bytes(data[26..30].try_into().expect("slice length mismatch"));
        println!("  The account was RESIZED from {} to {} bytes.", old_size, new_size);
    }
    if invariants_passed {
        println!("  All invariants PASSED.");
    } else {
        let inv_checked = u16::from_le_bytes(data[30..32].try_into().expect("slice length mismatch"));
        if inv_checked > 0 {
            println!("  WARNING: Invariants were checked but DID NOT PASS.");
        }
    }
    if cpi_invoked {
        let cpi_count = data[57];
        println!("  CPI was invoked ({} call(s)).", cpi_count);
    }
    if compat_impact != 0 {
        let impact_name = match compat_impact {
            1 => "append-only (backward readable)",
            2 => "requires migration",
            3 => "BREAKING",
            _ => "unknown",
        };
        println!("  Compatibility impact: {}.", impact_name);
    }
    if migration_flags != 0 {
        let mut mig = Vec::new();
        if migration_flags & 1 != 0 { mig.push("triggered"); }
        if migration_flags & 2 != 0 { mig.push("realloc"); }
        if migration_flags & 4 != 0 { mig.push("schema bump"); }
        println!("  Migration: {}.", mig.join(", "));
    }
}

fn cmd_explain_compat(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper explain compat <v1-json> <v2-json>");
        process::exit(1);
    }
    let v1 = parse_or_exit(&args[0]);
    let v2 = parse_or_exit(&args[1]);
    let (m1, _f1) = to_manifest(&v1);
    let (m2, _f2) = to_manifest(&v2);

    let verdict = CompatibilityVerdict::between(&m1, &m2);

    println!("=== Compatibility Explanation ===");
    println!();
    println!("  Comparing '{}' v{} → v{}", m1.name, m1.version, m2.version);
    println!("  Verdict: {}", verdict.name());
    println!();

    match verdict {
        CompatibilityVerdict::Identical => {
            println!("  No structural changes detected.");
        }
        CompatibilityVerdict::WireCompatible => {
            println!("  WIRE-COMPATIBLE: Byte layout is identical but semantic metadata differs.");
            println!("  Readers can parse both versions with the same wire code.");
            println!("  Review field intents and update tooling if semantics changed.");
        }
        CompatibilityVerdict::AppendSafe => {
            println!("  SAFE upgrade: New version preserves the old field prefix.");
            println!("  Old readers can still parse new accounts (they ignore new fields).");
        }
        CompatibilityVerdict::MigrationRequired => {
            println!("  MIGRATION required: Field layout has changed.");
            println!("  Old data is NOT directly backward-readable.");
            println!("  You need a migration instruction to move accounts to the new layout.");
            println!("  Use `hopper plan` to generate a step-by-step migration plan.");
        }
        CompatibilityVerdict::Incompatible => {
            println!("  BREAKING change: Field layout has changed and is NOT backward-readable.");
            println!("  You MUST migrate all accounts before deploying the new version.");
            println!("  Use `hopper plan` to generate a step-by-step migration plan.");
        }
    }

    // Explain field-level changes
    let report = compare_fields::<64>(&m1, &m2);
    let mut changes = Vec::new();
    for i in 0..report.len() {
        if let Some(entry) = report.get(i) {
            match entry.status {
                FieldCompat::Identical => {},
                FieldCompat::Added => changes.push(format!("  + Added field '{}'", entry.name)),
                FieldCompat::Removed => changes.push(format!("  - Removed field '{}'", entry.name)),
                FieldCompat::Changed => changes.push(format!("  ~ Changed field '{}'", entry.name)),
            }
        }
    }
    if !changes.is_empty() {
        println!();
        println!("  Field changes:");
        for c in &changes {
            println!("{}", c);
        }
    }
}

fn cmd_explain_policy(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper explain policy <pack-name>");
        eprintln!();
        eprintln!("Available packs:");
        eprintln!("  TreasuryWrite       Vault/treasury balance mutations");
        eprintln!("  JournalTouch        Journal segment writes");
        eprintln!("  ExternalCall        CPI-invoking instructions");
        eprintln!("  ShardMutation       Shard data modifications");
        eprintln!("  MigrationSensitive  Account reallocation/migration");
        eprintln!("  AuthorityChange     Authority/permission modifications");
        eprintln!("  ReadOnlyAudit       Read-only inspection/audit");
        eprintln!("  AccountInit         Account creation");
        eprintln!("  AccountClose        Account closure");
        process::exit(1);
    }

    let policy_info: (&str, &[(&str, &str)], bool, &[&str]) = match args[0].as_str() {
        "TreasuryWrite" => ("Vault/treasury balance mutations", &[
            ("MutatesState", "Authority, StateSnapshot"),
            ("MutatesTreasury", "LamportConservation, InvariantCheck"),
        ], true, &["lamport_balance_conserved", "no_phantom_credit"]),
        "JournalTouch" => ("Journal segment writes", &[
            ("MutatesState", "Authority"),
            ("TouchesJournal", "JournalCapacity, StateSnapshot"),
        ], true, &["journal_append_only", "segment_bounds_checked"]),
        "ExternalCall" => ("CPI-invoking instructions", &[
            ("ExternalCall", "CpiGuard, PostMutationCheck, StateSnapshot"),
        ], true, &["cpi_target_allowlisted", "no_reentrant_mutation"]),
        "ShardMutation" => ("Shard data modifications", &[
            ("MutatesState", "Authority, StateSnapshot, InvariantCheck"),
        ], true, &["shard_index_bounds_checked", "discriminator_preserved"]),
        "MigrationSensitive" => ("Account reallocation/migration", &[
            ("ReallocatesAccount", "Authority, RentExemption, StateSnapshot, InvariantCheck"),
        ], true, &["layout_id_updated", "old_data_preserved_or_migrated"]),
        "AuthorityChange" => ("Authority/permission modifications", &[
            ("ModifiesAuthority", "Authority, CpiGuard, PostMutationCheck, InvariantCheck"),
        ], true, &["old_authority_signed", "no_authority_escalation"]),
        "ReadOnlyAudit" => ("Read-only inspection", &[
            ("ReadsState", "StateSnapshot"),
        ], false, &["no_state_mutation"]),
        "AccountInit" => ("Account creation", &[
            ("CreatesAccount", "Authority, RentExemption, InvariantCheck"),
        ], true, &["header_initialized_correctly", "discriminator_set"]),
        "AccountClose" => ("Account closure", &[
            ("ClosesAccount", "Authority, StateSnapshot, LamportConservation"),
        ], true, &["lamports_drained", "data_zeroed"]),
        other => {
            eprintln!("Unknown policy pack: {}", other);
            process::exit(1);
        }
    };

    println!("=== Policy Pack: {} ===", args[0]);
    println!();
    println!("  Purpose: {}", policy_info.0);
    println!();
    println!("  Rules:");
    for &(cap, reqs) in policy_info.1 {
        println!("    When {}  → require {}", cap, reqs);
    }
    println!();
    println!("  Receipt expected: {}", if policy_info.2 { "YES" } else { "NO" });
    println!();
    if !policy_info.3.is_empty() {
        println!("  Invariant hints:");
        for hint in policy_info.3 {
            println!("    • {}", hint);
        }
        println!();
    }
    println!("  When this policy pack is active, the listed requirements");
    println!("  are automatically enforced for any instruction declaring");
    println!("  the corresponding capability.");
}

fn cmd_explain_layout(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper explain layout <manifest-json>");
        process::exit(1);
    }
    let pm = load_program_manifest(&args[0]);

    println!("=== Layout Explanation: {} v{} ===", pm.name, pm.version);
    println!();

    if pm.layouts.is_empty() {
        println!("  No layouts defined.");
        return;
    }

    for layout in pm.layouts {
        println!("  Layout: {} (disc={}, version={})", layout.name, layout.disc, layout.version);
        println!("    Wire layout_id: {}", hex_encode(&layout.layout_id));

        let fp = LayoutFingerprint::from_manifest(layout);
        println!("    Semantic fingerprint: {}", hex_encode(&fp.semantic_hash));
        println!("    Total size: {} bytes ({} fields)", layout.total_size, layout.field_count);
        println!();

        if layout.fields.is_empty() {
            println!("    (no field descriptors)");
        } else {
            println!("    Fields:");
            let mut monetary_count = 0u32;
            let mut identity_count = 0u32;
            for field in layout.fields {
                let intent_tag = if field.intent as u8 != 255 {
                    field.intent.name()
                } else {
                    "custom"
                };
                println!(
                    "      {:16} {:12} {:>3}B  @{:<4}  intent={}",
                    field.name, field.canonical_type, field.size, field.offset, intent_tag
                );
                if field.intent.is_monetary() { monetary_count += 1; }
                if field.intent.is_identity() { identity_count += 1; }
            }
            println!();
            if monetary_count > 0 {
                println!("    {} monetary field(s): lamport conservation checks recommended.", monetary_count);
            }
            if identity_count > 0 {
                println!("    {} identity field(s): authority validation required.", identity_count);
            }
        }
        println!();
    }
}

fn cmd_explain_program(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper explain program <manifest>");
        eprintln!("  Human-readable explanation of the entire program: layouts, instructions,");
        eprintln!("  policies, events, compatibility, and the Hopper pipeline steps it uses.");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);

    println!("=== Program: {} v{} ===", prog.name, prog.version);
    println!();
    println!("  {}", prog.description);
    println!();

    // Pipeline coverage
    println!("  Pipeline Coverage:");
    println!("    1. Define     {} layout(s) defined", prog.layouts.len());
    println!("    2. Resolve    {} instruction(s) with account resolution", prog.instructions.len());
    let policy_count = prog.policies.len();
    if policy_count > 0 {
        println!("    3. Validate   {} policy pack(s) enforced", policy_count);
    } else {
        println!("    3. Validate   (no named policies; consider adding policy packs)");
    }
    println!("    4. Execute    Mutations guarded by capabilities");
    let receipt_count = prog.instructions.iter().filter(|ix| ix.receipt_expected).count();
    if receipt_count > 0 {
        println!("    5. Record     {} instruction(s) emit receipts", receipt_count);
    } else {
        println!("    5. Record     (no receipt expectations; consider adding receipt tracking)");
    }
    let compat_count = prog.compatibility_pairs.len();
    if compat_count > 0 {
        println!("    6. Verify     {} compatibility rule(s)", compat_count);
    } else {
        println!("    6. Verify     (no compat rules; safe for single-version programs)");
    }
    let event_count = prog.events.len();
    println!("    7. Inspect    {} event(s) for off-chain observability", event_count);
    println!();

    // Layouts
    println!("  Layouts:");
    for l in prog.layouts.iter() {
        let fp = LayoutFingerprint::from_manifest(l);
        println!("    {} v{} | disc {} | {} bytes | {} fields",
            l.name, l.version, l.disc, l.total_size, l.field_count);
        println!("      wire={}  semantic={}", hex_encode(&fp.wire_hash), hex_encode(&fp.semantic_hash));
        let monetary: Vec<&str> = l.fields.iter()
            .filter(|f| f.intent.is_monetary())
            .map(|f| f.name)
            .collect();
        let identity: Vec<&str> = l.fields.iter()
            .filter(|f| f.intent.is_identity())
            .map(|f| f.name)
            .collect();
        if !monetary.is_empty() {
            println!("      monetary fields: {}", monetary.join(", "));
        }
        if !identity.is_empty() {
            println!("      identity fields: {}", identity.join(", "));
        }
    }
    println!();

    // Instructions
    println!("  Instructions:");
    for ix in prog.instructions.iter() {
        print!("    [{}] {} | {} args | {} accounts",
            ix.tag, ix.name, ix.args.len(), ix.accounts.len());
        if ix.receipt_expected { print!(" | receipt"); }
        if !ix.policy_pack.is_empty() { print!(" | policy={}", ix.policy_pack); }
        println!();
    }
    println!();

    // Policies
    if !prog.policies.is_empty() {
        println!("  Policies:");
        for p in prog.policies.iter() {
            println!("    {} | {} capabilities, {} requirements, {} invariants | receipt={}",
                p.name, p.capabilities.len(), p.requirements.len(),
                p.invariants.len(), p.receipt_profile);
        }
        println!();
    }

    // Events
    if !prog.events.is_empty() {
        println!("  Events:");
        for ev in prog.events.iter() {
            println!("    [{}] {} | {} fields", ev.tag, ev.name, ev.fields.len());
        }
        println!();
    }

    // Contexts
    if !prog.contexts.is_empty() {
        println!("  Contexts:");
        for ctx in prog.contexts.iter() {
            let signers = ctx.accounts.iter().filter(|a| a.signer).count();
            let writables = ctx.accounts.iter().filter(|a| a.writable).count();
            print!("    {} | {} accounts ({} signer, {} writable)",
                ctx.name, ctx.accounts.len(), signers, writables);
            if !ctx.policies.is_empty() {
                print!(" | policies: {}", ctx.policies.join(", "));
            }
            if ctx.receipts_expected {
                print!(" | receipt");
            }
            println!();
        }
        println!();
    }

    // Assessment
    println!("  Assessment:");
    if policy_count > 0 && receipt_count > 0 && compat_count > 0 {
        println!("    This program uses the full Hopper pipeline: layouts, policies,");
        println!("    receipts, and compatibility rules. It is production-ready for");
        println!("    schema-aware tooling and version evolution.");
    } else {
        let mut missing = Vec::new();
        if policy_count == 0 { missing.push("named policies"); }
        if receipt_count == 0 { missing.push("receipt tracking"); }
        if compat_count == 0 { missing.push("compatibility rules"); }
        println!("    The program is functional but could benefit from adding: {}.",
            missing.join(", "));
        println!("    These are optional for simple programs but recommended for");
        println!("    protocols planning version evolution or operator dashboards.");
    }
}

fn cmd_explain_context(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper explain context <manifest> [--type <ContextName>]");
        eprintln!("  Show instruction contexts with account roles, mutability, signer status,");
        eprintln!("  layout bindings, policy bindings, seeds, and optionality.");
        eprintln!();
        eprintln!("  Without --type, shows all contexts in the manifest.");
        eprintln!("  With --type, filters to a single named context.");
        process::exit(1);
    }

    let mut manifest_path: Option<&str> = None;
    let mut filter_type: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--type" {
            if i + 1 < args.len() {
                filter_type = Some(&args[i + 1]);
                i += 2;
                continue;
            } else {
                eprintln!("--type requires a context name argument");
                process::exit(1);
            }
        }
        if manifest_path.is_none() {
            manifest_path = Some(&args[i]);
        }
        i += 1;
    }

    let manifest_path = match manifest_path {
        Some(p) => p,
        None => {
            eprintln!("Missing manifest path");
            process::exit(1);
        }
    };

    let manifest = load_program_manifest(manifest_path);

    if manifest.contexts.is_empty() {
        println!("=== Contexts: {} v{} ===", manifest.name, manifest.version);
        println!();
        println!("  No contexts defined in this manifest.");
        println!("  Use hopper_accounts! or #[derive(HopperAccounts)] to add typed contexts.");
        return;
    }

    let contexts: Vec<_> = if let Some(name) = filter_type {
        manifest.contexts.iter().filter(|c| c.name == name).collect()
    } else {
        manifest.contexts.iter().collect()
    };

    if contexts.is_empty() {
        if let Some(name) = filter_type {
            eprintln!("No context named '{}' found in manifest.", name);
            eprintln!("Available contexts:");
            for c in manifest.contexts.iter() {
                eprintln!("  {}", c.name);
            }
            process::exit(1);
        }
    }

    println!("=== Contexts: {} v{} ===", manifest.name, manifest.version);
    println!();

    for ctx in &contexts {
        println!("  Context: {}", ctx.name);
        println!("    Accounts: {} total, {} signer(s), {} writable",
            ctx.accounts.len(),
            ctx.accounts.iter().filter(|a| a.signer).count(),
            ctx.accounts.iter().filter(|a| a.writable).count(),
        );
        println!();

        for acct in ctx.accounts.iter() {
            print!("    {:16} {:16}", acct.name, acct.kind);
            let mut flags = Vec::new();
            if acct.writable { flags.push("mut"); }
            if acct.signer { flags.push("signer"); }
            if acct.optional { flags.push("optional"); }
            if !flags.is_empty() {
                print!("  [{}]", flags.join(", "));
            }
            println!();

            if !acct.layout_ref.is_empty() {
                println!("      layout = {}", acct.layout_ref);
            }
            if !acct.policy_ref.is_empty() {
                println!("      policy = {}", acct.policy_ref);
            }
            if !acct.seeds.is_empty() {
                println!("      seeds  = [{}]", acct.seeds.join(", "));
            }
        }
        println!();

        if !ctx.policies.is_empty() {
            println!("    Policies: {}", ctx.policies.join(", "));
        }
        if ctx.receipts_expected {
            println!("    Receipts: expected");
        }
        if !ctx.mutation_classes.is_empty() {
            println!("    Mutations: {}", ctx.mutation_classes.join(", "));
        }
        if !ctx.policies.is_empty() || ctx.receipts_expected || !ctx.mutation_classes.is_empty() {
            println!();
        }
    }
}

fn print_usage() {
    println!("hopper-cli: Hopper account inspection, schema tooling, and program management");
    println!();
    println!("COMMAND FAMILIES:");
    println!();
    println!("  Schema:");
    println!("    hopper schema export               Schema format reference");
    println!("    hopper schema validate <manifest>   Validate a program manifest");
    println!("    hopper schema diff <old> <new>      Field-level diff");
    println!();
    println!("  Inspect:");
    println!("    hopper inspect <hex-data>           Decode account header");
    println!("    hopper inspect segments <hex-data>  Decode segment map");
    println!("    hopper inspect receipt <hex-data>   Decode a state receipt");
    println!();
    println!("  Explain:");
    println!("    hopper explain <hex-data>           Human-readable account explanation");
    println!("    hopper explain account <hex-data>   Explicit account explanation");
    println!("    hopper explain receipt <hex-data>   Explain a receipt in plain English");
    println!("    hopper explain compat <old> <new>   Explain compatibility report");
    println!("    hopper explain policy <pack-name>   Explain a named policy pack");
    println!("    hopper explain layout <manifest>    Explain layout fields, intents, fingerprint");
    println!("    hopper explain program <manifest>   Explain entire program pipeline");
    println!("    hopper explain context <manifest>   Explain instruction contexts and account roles");
    println!();
    println!("  Compatibility:");
    println!("    hopper compat <v1-json> <v2-json>   Compatibility report");
    println!("    hopper plan <v1-json> <v2-json>     Migration plan with steps");
    println!();
    println!("  Receipts:");
    println!("    hopper receipt <hex-data>           Decode and display receipt");
    println!();
    println!("  Manager:");
    println!("    hopper manager summary <manifest>                      Program overview");
    println!("    hopper manager identify <manifest> <hex>               Identify account type");
    println!("    hopper manager decode <manifest> <hex>                 Decode all fields");
    println!("    hopper manager instruction <manifest> <tag|name>       Instruction details");
    println!("    hopper manager layouts <manifest>                      List all layouts");
    println!("    hopper manager policies <manifest>                     List policy packs");
    println!("    hopper manager events <manifest>                       List events with fields");
    println!("    hopper manager fingerprints <manifest>                 Show all fingerprints");
    println!("    hopper manager compat <manifest> <hex-old> <hex-new>   Compare two accounts");
    println!("    hopper manager receipt <hex-64-bytes>                  Decode a state receipt");
    println!("    hopper manager explain <manifest>                      Aggregated summary");
    println!("    hopper manager diff <manifest> <hex-old> <hex-new>    Semantic field diff");
    println!("    hopper manager simulate <manifest> <instruction>      Preview requirements");
    println!();
    println!("  Fetch (on-chain):");
    println!("    hopper fetch <program-id> [--rpc <url>]          Fetch on-chain manifest");
    println!("    hopper fetch <program-id> --json [--rpc <url>]   Fetch manifest as raw JSON");
    println!("    hopper manager fetch <program-id> [--rpc <url>]  Fetch + show program summary");
    println!();
    println!("  Lifecycle:");
    println!("    hopper init <path>                 Create a Hopper-native project scaffold");
    println!("    hopper build [--host|--sbf]        Build the current project (default: SBF)");
    println!("    hopper test                        Run the current project's host-side tests");
    println!();
    println!("  Profiling:");
    println!("    hopper profile bench               Run the primitive benchmark lab");
    println!();
    println!("  Interactive:");
    println!("    hopper interactive <manifest>        Launch interactive explorer");
    println!("    hopper ui <manifest>                 Alias for interactive");
    println!("    hopper manager interactive <manifest> Interactive from manager context");
    println!();
    println!("  Client:");
    println!("    hopper client gen --ts <manifest>  Generate TypeScript client SDK");
    println!("    hopper client gen --kt <manifest>  Generate Kotlin client SDK");
    println!();
    println!("Hex data: hex-encoded account bytes (no 0x prefix).");
    println!("Manifest arguments accept inline JSON or @path/to/file.json.");
    println!("Program IDs: base58-encoded Solana public keys.");
    println!("RPC URL: set via --rpc flag, SOLANA_RPC_URL env, or defaults to mainnet.");
}

// -- Hex decode (inline, no external dependency) --

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("Hex string must have even length".to_string());
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    let chars: Vec<u8> = s.bytes().collect();
    for pair in chars.chunks(2) {
        let hi = hex_nibble(pair[0]).ok_or_else(|| format!("Invalid hex char: {}", pair[0] as char))?;
        let lo = hex_nibble(pair[1]).ok_or_else(|| format!("Invalid hex char: {}", pair[1] as char))?;
        bytes.push((hi << 4) | lo);
    }
    Ok(bytes)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

// -- Minimal JSON manifest parser (no serde dependency) --

/// Parsed manifest from JSON input (simplified).
struct ParsedManifest {
    name: String,
    disc: u8,
    version: u8,
    layout_id: [u8; 8],
    total_size: usize,
    fields: Vec<ParsedField>,
}

struct ParsedField {
    name: String,
    canonical_type: String,
    size: u16,
    offset: u16,
}

fn parse_manifest_json(json: &str) -> Result<ParsedManifest, String> {
    // Minimal JSON parser for manifest objects.
    // Expects: {"name":"...","disc":N,"version":N,"layout_id":[...],"total_size":N,
    //           "fields":[{"name":"...","type":"...","size":N,"offset":N},...]}
    let json = json.trim();
    if !json.starts_with('{') || !json.ends_with('}') {
        return Err("Expected JSON object".to_string());
    }

    let name = extract_string(json, "name")?;
    let disc = extract_number(json, "disc")? as u8;
    let version = extract_number(json, "version")? as u8;
    let total_size = extract_number(json, "total_size")? as usize;
    let layout_id = extract_array_u8(json, "layout_id")?;

    let mut lid = [0u8; 8];
    if layout_id.len() != 8 {
        return Err("layout_id must be exactly 8 bytes".to_string());
    }
    lid.copy_from_slice(&layout_id);

    let fields = extract_fields(json)?;

    Ok(ParsedManifest {
        name,
        disc,
        version,
        layout_id: lid,
        total_size,
        fields,
    })
}

fn extract_string(json: &str, key: &str) -> Result<String, String> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern).ok_or_else(|| format!("Missing key: {}", key))?;
    let after = &json[pos + pattern.len()..];
    // Skip : and whitespace
    let after = after.trim_start().strip_prefix(':').ok_or("Expected :")?;
    let after = after.trim_start();
    if !after.starts_with('"') {
        return Err(format!("Expected string value for {}", key));
    }
    let after = &after[1..]; // skip opening quote
    let end = after.find('"').ok_or("Unterminated string")?;
    Ok(after[..end].to_string())
}

fn extract_number(json: &str, key: &str) -> Result<u64, String> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern).ok_or_else(|| format!("Missing key: {}", key))?;
    let after = &json[pos + pattern.len()..];
    let after = after.trim_start().strip_prefix(':').ok_or("Expected :")?;
    let after = after.trim_start();
    let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    after[..end].parse().map_err(|e| format!("Invalid number for {}: {}", key, e))
}

fn extract_array_u8(json: &str, key: &str) -> Result<Vec<u8>, String> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern).ok_or_else(|| format!("Missing key: {}", key))?;
    let after = &json[pos + pattern.len()..];
    let after = after.trim_start().strip_prefix(':').ok_or("Expected :")?;
    let after = after.trim_start();
    let start = after.find('[').ok_or("Expected [")?;
    let end = after.find(']').ok_or("Expected ]")?;
    let inner = &after[start + 1..end];
    inner
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<u8>()
                .map_err(|e| format!("Invalid byte: {}", e))
        })
        .collect()
}

fn extract_fields(json: &str) -> Result<Vec<ParsedField>, String> {
    let key = "\"fields\"";
    let pos = json.find(key).ok_or("Missing fields array")?;
    let after = &json[pos + key.len()..];
    let after = after.trim_start().strip_prefix(':').ok_or("Expected :")?;
    let after = after.trim_start();

    // Find the outer array brackets
    let start = after.find('[').ok_or("Expected [")?;
    let after = &after[start + 1..];

    // Parse individual field objects
    let mut fields = Vec::new();
    let mut remaining = after;

    loop {
        remaining = remaining.trim_start();
        if remaining.starts_with(']') {
            break;
        }
        if remaining.starts_with(',') {
            remaining = &remaining[1..];
            continue;
        }
        if !remaining.starts_with('{') {
            break;
        }

        let end = remaining.find('}').ok_or("Unterminated field object")?;
        let obj = &remaining[..=end];

        let name = extract_string(obj, "name")?;
        let canonical_type = extract_string(obj, "type")?;
        let size = extract_number(obj, "size")? as u16;
        let offset = extract_number(obj, "offset")? as u16;

        fields.push(ParsedField {
            name,
            canonical_type,
            size,
            offset,
        });

        remaining = &remaining[end + 1..];
    }

    Ok(fields)
}

// -- Convert ParsedManifest to hopper_schema types --

// We need to hold the field descriptors in a Vec so the LayoutManifest can
// borrow them. This helper struct owns the data.
struct OwnedManifest {
    name: String,
    disc: u8,
    version: u8,
    layout_id: [u8; 8],
    total_size: usize,
    fields: Vec<OwnedField>,
}

struct OwnedField {
    name: String,
    canonical_type: String,
    size: u16,
    offset: u16,
}

impl From<ParsedManifest> for OwnedManifest {
    fn from(p: ParsedManifest) -> Self {
        Self {
            name: p.name,
            disc: p.disc,
            version: p.version,
            layout_id: p.layout_id,
            total_size: p.total_size,
            fields: p
                .fields
                .into_iter()
                .map(|f| OwnedField {
                    name: f.name,
                    canonical_type: f.canonical_type,
                    size: f.size,
                    offset: f.offset,
                })
                .collect(),
        }
    }
}

// -- Commands --

fn cmd_explain(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper explain <hex-data>");
        process::exit(1);
    }
    let data = match hex_decode(&args[0]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Hex decode error: {}", e);
            process::exit(1);
        }
    };

    if data.len() < 16 {
        println!("This data is {} bytes, which is too short for a Hopper account.", data.len());
        println!("Every Hopper account starts with a 16-byte header.");
        process::exit(1);
    }

    let header = require_header(&data);

    println!("This is a Hopper account ({} bytes total).", data.len());
    println!();

    // Header narrative
    println!("Header:");
    println!(
        "  Discriminator {} identifies the account type.",
        header.disc
    );
    println!(
        "  Layout version {}, fingerprint {}.",
        header.version,
        hex_encode(&header.layout_id)
    );
    let flag_str = format_flags(header.flags);
    if header.flags == 0 {
        println!("  No flags set.");
    } else {
        println!("  Flags: {} (0x{:04x}).", flag_str, header.flags);
    }
    println!();

    // Check if it looks segmented
    let is_segmented = header.flags & 0x0004 != 0;
    let has_registry = data.len() >= 20;
    let seg_result = if is_segmented || has_registry {
        decode_segments::<16>(&data)
    } else {
        None
    };

    match seg_result {
        Some((count, segments)) if count > 0 => {
            println!("Account structure: segmented ({} segments).", count);
            println!();
            let reg_end = 16 + 4 + count * 16;
            println!(
                "  Bytes 0..16 are the header, 16..{} is the segment registry.",
                reg_end
            );
            for (i, seg) in segments[..count].iter().enumerate() {
                let end = seg.offset as usize + seg.size as usize;
                let role_name = decode_segment_role(seg.flags);
                println!(
                    "  Segment {} (id {}): bytes {}..{} ({} bytes, role: {}).",
                    i,
                    hex_encode(&seg.id),
                    seg.offset,
                    end,
                    seg.size,
                    role_name,
                );
                println!("    {}", describe_segment_role(role_name));
            }

            // Migration advice summary
            let advice = SegmentMigrationReport::<16>::analyze(&segments, count);
            println!();
            println!("Migration readiness:");
            println!(
                "  {} must-preserve, {} clearable, {} rebuildable.",
                advice.must_preserve_count(),
                advice.clearable_count(),
                advice.rebuildable_bytes,
            );
            println!(
                "  preserve={} bytes, clearable={} bytes.",
                advice.preserve_bytes, advice.clearable_bytes,
            );
        }
        _ => {
            let body_size = data.len() - 16;
            println!("Account structure: fixed layout ({} byte body after header).", body_size);
        }
    }

    println!();

    // Close sentinel check
    if data[0] == 0xFF {
        println!("Warning: discriminator is 0xFF. This account may have been closed");
        println!("with a sentinel byte to prevent revival.");
        println!();
    }

    // Zero-fill check
    let zero_body = data[16..].iter().all(|&b| b == 0);
    if zero_body && data.len() > 16 {
        println!("Note: the body is entirely zeroed. This account may be freshly");
        println!("initialized or not yet written to.");
        println!();
    }

    // Policy context hint based on account structure
    if is_segmented || seg_result.is_some() {
        println!("Policy context:");
        println!("  Segmented accounts typically use named policy packs:");
        println!("    TREASURY_WRITE  -- balance mutations (authority + snapshot + conservation)");
        println!("    JOURNAL_TOUCH   -- journal appends (authority + capacity + snapshot)");
        println!("    AUTHORITY_CHANGE -- permission changes (authority + CPI guard + invariants)");
        println!("  Use 'hopper receipt <hex>' to decode which capabilities were declared");
        println!("  in a specific transaction.");
        println!();
    }

    println!("Next steps:");
    println!("  hopper inspect <hex>     -- raw header fields");
    println!("  hopper segments <hex>    -- segment map with roles");
    println!("  hopper receipt <hex>     -- decode a state receipt from transaction logs");
    println!("  hopper compat <v1> <v2>  -- compare against another version");
    println!("  hopper plan <v1> <v2>    -- generate migration plan");
}

fn decode_segment_role(flags: u16) -> &'static str {
    SegmentRoleHint::from_flags(flags).name()
}

fn describe_segment_role(role: &str) -> &'static str {
    match role {
        "Core" | "core" => "Primary state, must be preserved across migrations.",
        "Extension" | "extension" => "Optional extension data, safe to append new fields.",
        "Journal" | "journal" => "Append-only log, may wrap if circular. Clearable on migration.",
        "Index" | "index" => "Derived lookup data. Can be rebuilt from core state.",
        "Cache" | "cache" => "Computed cache. Can be cleared and recomputed on migration.",
        "Audit" | "audit" => "Immutable audit trail. Must be preserved.",
        "Shard" | "shard" => "Partitioned data. May be split or merged across accounts.",
        _ => "No defined migration or runtime semantics.",
    }
}

fn cmd_inspect(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper inspect <hex-data>");
        process::exit(1);
    }
    let data = match hex_decode(&args[0]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Hex decode error: {}", e);
            process::exit(1);
        }
    };

    match decode_header(&data) {
        Some(h) => print_header(&h),
        None => {
            eprintln!("Account data too short for header (need 16 bytes, got {})", data.len());
            process::exit(1);
        }
    }
}

fn print_header(h: &DecodedHeader) {
    println!("=== Account Header (16 bytes) ===");
    println!("  Discriminator : {}", h.disc);
    println!("  Version       : {}", h.version);
    println!("  Flags         : 0x{:04x} ({})", h.flags, format_flags(h.flags));
    println!("  Layout ID     : {}", hex_encode(&h.layout_id));
    println!("  Reserved      : {}", hex_encode(&h.reserved));
}

fn format_flags(flags: u16) -> String {
    if flags == 0 {
        return "none".to_string();
    }
    let mut parts = Vec::new();
    if flags & 0x0001 != 0 {
        parts.push("INITIALIZED");
    }
    if flags & 0x0002 != 0 {
        parts.push("FROZEN");
    }
    if flags & 0x0004 != 0 {
        parts.push("SEGMENTED");
    }
    if flags & 0x0008 != 0 {
        parts.push("CLOSING");
    }
    if parts.is_empty() {
        format!("0x{:04x}", flags)
    } else {
        parts.join(" | ")
    }
}

fn cmd_segments(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper segments <hex-data>");
        process::exit(1);
    }
    let data = match hex_decode(&args[0]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Hex decode error: {}", e);
            process::exit(1);
        }
    };

    // Print header first
    if let Some(h) = decode_header(&data) {
        print_header(&h);
        println!();
    }

    match decode_segments::<16>(&data) {
        Some((count, segments)) => {
            println!("=== Segment Registry ({} segments) ===", count);
            println!(
                "  {:>4}  {:>10}  {:>10}  {:>8}  {:>6}  Ver",
                "#", "ID", "Offset", "Size", "Flags"
            );
            println!("  {}", "-".repeat(56));
            for (i, seg) in segments[..count].iter().enumerate() {
                println!(
                    "  {:>4}  {:>10}  {:>10}  {:>8}  0x{:04x}  {:>4}",
                    i,
                    hex_encode(&seg.id),
                    seg.offset,
                    seg.size,
                    seg.flags,
                    seg.version,
                );
            }

            // Visual segment map
            println!();
            println!("=== Segment Map ===");
            println!("  [Header: 0..16]");
            // Registry header + entries
            let reg_end = 16 + 4 + count * 16;
            println!("  [Registry: 16..{}]", reg_end);
            for (i, seg) in segments[..count].iter().enumerate() {
                let end = seg.offset as usize + seg.size as usize;
                println!(
                    "  [Segment {}: {}..{} ({} bytes, id={})]",
                    i,
                    seg.offset,
                    end,
                    seg.size,
                    hex_encode(&seg.id),
                );
            }
        }
        None => {
            eprintln!("Could not decode segment registry (data too short or invalid)");
            process::exit(1);
        }
    }
}

// -- Receipt decoding (matches hopper-core receipt wire format v2, 64 bytes) --

const RECEIPT_WIRE_SIZE: usize = 64;

fn cmd_receipt(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper receipt <hex-data>");
        process::exit(1);
    }
    let data = match hex_decode(&args[0]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Hex decode error: {}", e);
            process::exit(1);
        }
    };

    if data.len() < RECEIPT_WIRE_SIZE {
        eprintln!(
            "Receipt data too short (got {} bytes, need {} bytes).",
            data.len(),
            RECEIPT_WIRE_SIZE,
        );
        process::exit(1);
    }

    let layout_id = &data[0..8];
    let changed_fields = u64::from_le_bytes(data[8..16].try_into().expect("slice length mismatch"));
    let changed_bytes = u32::from_le_bytes(data[16..20].try_into().expect("slice length mismatch"));
    let changed_regions = u16::from_le_bytes(data[20..22].try_into().expect("slice length mismatch"));
    let old_size = u32::from_le_bytes(data[22..26].try_into().expect("slice length mismatch"));
    let new_size = u32::from_le_bytes(data[26..30].try_into().expect("slice length mismatch"));
    let invariants_checked = u16::from_le_bytes(data[30..32].try_into().expect("slice length mismatch"));
    let flags = data[32];
    let was_resized = flags & (1 << 0) != 0;
    let invariants_passed = flags & (1 << 1) != 0;
    let cpi_invoked = flags & (1 << 2) != 0;
    let committed = flags & (1 << 3) != 0;
    let before_fp = &data[33..41];
    let after_fp = &data[41..49];
    let segment_mask = u16::from_le_bytes(data[49..51].try_into().expect("slice length mismatch"));
    let policy_flags = u32::from_le_bytes(data[51..55].try_into().expect("slice length mismatch"));
    let journal_appends = u16::from_le_bytes(data[55..57].try_into().expect("slice length mismatch"));
    let cpi_count = data[57];
    let phase = data[58];
    let validation_bundle_id = u16::from_le_bytes(data[59..61].try_into().expect("slice length mismatch"));
    let compat_impact = data[61];
    let migration_flags = data[62];

    let phase_name = match phase {
        1 => "init",
        2 => "close",
        3 => "migrate",
        4 => "read-only",
        _ => "update",
    };
    let compat_name = match compat_impact {
        1 => "append",
        2 => "migration",
        3 => "breaking",
        _ => "none",
    };

    println!("=== State Receipt ({} bytes) ===", data.len());
    println!();
    println!("  Layout ID           : {}", hex_encode(layout_id));
    println!("  Committed           : {}", if committed { "YES" } else { "NO" });
    println!("  Phase               : {}", phase_name);
    println!();
    println!("  Changed bytes       : {}", changed_bytes);
    println!("  Changed regions     : {}", changed_regions);
    println!("  Changed field mask  : 0x{:016x}", changed_fields);
    if changed_fields != 0 {
        let mut fields_list = Vec::new();
        for bit in 0..64u32 {
            if changed_fields & (1u64 << bit) != 0 {
                fields_list.push(format!("{}", bit));
            }
        }
        println!("    Fields touched    : [{}]", fields_list.join(", "));
    }
    println!();
    println!("  Old size            : {} bytes", old_size);
    println!("  New size            : {} bytes", new_size);
    println!("  Resized             : {}", if was_resized { "YES" } else { "NO" });
    println!();
    println!("  Before fingerprint  : {}", hex_encode(before_fp));
    println!("  After fingerprint   : {}", hex_encode(after_fp));
    let fp_changed = before_fp != after_fp;
    println!("  Data changed        : {}", if fp_changed { "YES" } else { "NO" });
    println!();
    println!("  Invariants checked  : {}", invariants_checked);
    println!("  Invariants passed   : {}", if invariants_passed { "YES" } else { "NO" });
    println!();

    if policy_flags != 0 {
        println!("  Policy flags        : 0x{:08x}", policy_flags);
        let cap_names = [
            "ReadsState", "MutatesState", "TouchesJournal", "ExternalCall",
            "MutatesTreasury", "ReallocatesAccount", "CreatesAccount", "ClosesAccount",
            "ModifiesAuthority", "TransitionsState",
        ];
        let mut active = Vec::new();
        for (i, name) in cap_names.iter().enumerate() {
            if policy_flags & (1 << i) != 0 {
                active.push(*name);
            }
        }
        if !active.is_empty() {
            println!("    Capabilities      : {}", active.join(", "));
        }
        println!();
    }

    if segment_mask != 0 {
        println!("  Segment change mask : 0x{:04x}", segment_mask);
        let mut segs = Vec::new();
        for bit in 0..16u32 {
            if segment_mask & (1 << bit) != 0 {
                segs.push(format!("{}", bit));
            }
        }
        println!("    Segments touched  : [{}]", segs.join(", "));
        println!();
    }

    if journal_appends > 0 {
        println!("  Journal appends     : {}", journal_appends);
    }
    if cpi_invoked {
        println!("  CPI invoked         : YES ({} calls)", cpi_count);
    }

    if validation_bundle_id != 0 {
        println!("  Validation bundle   : {}", validation_bundle_id);
    }
    if compat_impact != 0 {
        println!("  Compat impact       : {}", compat_name);
    }
    if migration_flags != 0 {
        let mut mig = Vec::new();
        if migration_flags & 1 != 0 { mig.push("triggered"); }
        if migration_flags & 2 != 0 { mig.push("realloc"); }
        if migration_flags & 4 != 0 { mig.push("schema-bump"); }
        println!("  Migration           : {}", mig.join(", "));
    }
}

fn cmd_compat(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper compat <v1-json> <v2-json>");
        process::exit(1);
    }
    let v1 = parse_or_exit(&args[0]);
    let v2 = parse_or_exit(&args[1]);

    let (m1, _f1) = to_manifest(&v1);
    let (m2, _f2) = to_manifest(&v2);

    let verdict = CompatibilityVerdict::between(&m1, &m2);

    println!("=== Compatibility Report ===");
    println!("  {} v{} -> {} v{}", v1.name, v1.version, v2.name, v2.version);
    println!("  Layout ID (old) : {}", hex_encode(&v1.layout_id));
    println!("  Layout ID (new) : {}", hex_encode(&v2.layout_id));
    println!("  Size (old)      : {} bytes", v1.total_size);
    println!("  Size (new)      : {} bytes", v2.total_size);
    println!("  Verdict         : {}", verdict.name());
    println!("  Safe            : {}", if verdict.is_safe() { "YES" } else { "NO" });
    println!("  Backward-read   : {}", if verdict.is_backward_readable() { "YES" } else { "NO" });
    println!("  Requires migration: {}", if verdict.requires_migration() { "YES" } else { "NO" });

    println!();
    match verdict {
        CompatibilityVerdict::Identical => {
            println!("  Result: No changes detected.");
        }
        CompatibilityVerdict::WireCompatible => {
            println!("  Result: Wire-compatible. Byte layout identical, semantic metadata differs.");
        }
        CompatibilityVerdict::AppendSafe => {
            println!("  Result: Safe upgrade. Old field prefix preserved, no migration needed.");
        }
        CompatibilityVerdict::MigrationRequired => {
            println!("  Result: Migration required. Use `hopper plan` for details.");
        }
        CompatibilityVerdict::Incompatible => {
            println!("  Result: Breaking change. Full migration required before upgrade.");
        }
    }
}

fn cmd_diff(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper diff <v1-json> <v2-json>");
        process::exit(1);
    }
    let v1 = parse_or_exit(&args[0]);
    let v2 = parse_or_exit(&args[1]);

    let (m1, _f1) = to_manifest(&v1);
    let (m2, _f2) = to_manifest(&v2);

    let report = compare_fields::<32>(&m1, &m2);

    println!("=== Field Diff: {} v{} -> {} v{} ===", v1.name, v1.version, v2.name, v2.version);
    println!(
        "  {:>20}  {:>12}  {:>8}",
        "Field", "Status", "Detail"
    );
    println!("  {}", "-".repeat(46));
    for i in 0..report.len() {
        if let Some(entry) = report.get(i) {
            let status_str = match entry.status {
                FieldCompat::Identical => "IDENTICAL",
                FieldCompat::Changed => "CHANGED",
                FieldCompat::Added => "ADDED",
                FieldCompat::Removed => "REMOVED",
            };
            let detail = match entry.status {
                FieldCompat::Added => {
                    // Find in v2 fields
                    let mut d = String::new();
                    for f in &v2.fields {
                        if f.name == entry.name {
                            d = format!("{} ({} bytes @ {})", f.canonical_type, f.size, f.offset);
                            break;
                        }
                    }
                    d
                }
                FieldCompat::Removed => "(deleted)".to_string(),
                FieldCompat::Changed => "(type or size changed)".to_string(),
                FieldCompat::Identical => "".to_string(),
            };
            println!("  {:>20}  {:>12}  {}", entry.name, status_str, detail);
        }
    }

    println!();
    let identical = report.count_status(FieldCompat::Identical);
    let added = report.count_status(FieldCompat::Added);
    let removed = report.count_status(FieldCompat::Removed);
    let changed = report.count_status(FieldCompat::Changed);
    println!("  Summary: {} identical, {} added, {} removed, {} changed", identical, added, removed, changed);
    println!("  Append-safe: {}", if report.is_append_safe { "YES" } else { "NO" });
}

fn cmd_plan(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper plan <v1-json> <v2-json>");
        process::exit(1);
    }
    let v1 = parse_or_exit(&args[0]);
    let v2 = parse_or_exit(&args[1]);

    let (m1, _f1) = to_manifest(&v1);
    let (m2, _f2) = to_manifest(&v2);

    let plan = MigrationPlan::<16>::generate(&m1, &m2);

    println!("=== Migration Plan: {} v{} -> {} v{} ===", v1.name, v1.version, v2.name, v2.version);
    println!();

    let policy_str = match plan.policy {
        MigrationPolicy::NoOp => "NO-OP (layouts identical)",
        MigrationPolicy::AppendOnly => "APPEND-ONLY (safe in-place upgrade)",
        MigrationPolicy::RequiresMigration => "FULL MIGRATION (data copy required)",
        MigrationPolicy::Incompatible => "INCOMPATIBLE (different discriminators)",
    };
    println!("  Policy     : {}", policy_str);
    println!("  Old size   : {} bytes", plan.old_size);
    println!("  New size   : {} bytes", plan.new_size);
    println!("  Copy bytes : {}", plan.copy_bytes);
    println!("  Zero bytes : {}", plan.zero_bytes);
    println!("  Backward   : {}", if plan.backward_readable { "YES (v1 code can read v2 accounts)" } else { "NO" });
    println!("  Steps      : {}", plan.len());

    if !plan.is_empty() {
        println!();
        println!("  {:>4}  {:>14}  {:>8}  {:>8}  Field", "#", "Action", "Offset", "Size");
        println!("  {}", "-".repeat(52));
        plan.for_each_step(|i, step| {
            let action_str = match step.action {
                MigrationAction::CopyPrefix => "CopyPrefix",
                MigrationAction::ZeroInit => "ZeroInit",
                MigrationAction::UpdateHeader => "UpdateHeader",
                MigrationAction::Realloc => "Realloc",
            };
            let field = if step.field.is_empty() {
                "-"
            } else {
                step.field
            };
            println!(
                "  {:>4}  {:>14}  {:>8}  {:>8}  {}",
                i, action_str, step.offset, step.size, field
            );
        });
    }
}

fn cmd_schema_export_family(args: &[String]) {
    if args.is_empty() {
        // No flag: show the reference document
        cmd_schema_export();
        return;
    }
    match args[0].as_str() {
        "--manifest" => {
            if args.len() < 2 {
                eprintln!("Usage: hopper schema export --manifest <manifest-json>");
                process::exit(1);
            }
            let prog = load_program_manifest(&args[1]);
            println!("{}", hopper_schema::codama::ManifestJson(&prog));
        }
        "--idl" => {
            if args.len() < 2 {
                eprintln!("Usage: hopper schema export --idl <manifest-json>");
                process::exit(1);
            }
            let prog = load_program_manifest(&args[1]);
            println!("{}", hopper_schema::codama::IdlJsonFromManifest(&prog));
        }
        "--codama" => {
            if args.len() < 2 {
                eprintln!("Usage: hopper schema export --codama <manifest-json>");
                process::exit(1);
            }
            let prog = load_program_manifest(&args[1]);
            println!("{}", hopper_schema::codama::CodamaJsonFromManifest(&prog));
        }
        _ => cmd_schema_export(),
    }
}

fn cmd_schema_export() {
    println!("=== Hopper Account Schema Format ===");
    println!();
    println!("Header (16 bytes, offset 0):");
    println!("  [0]      disc        u8       Account discriminator");
    println!("  [1]      version     u8       Layout version");
    println!("  [2..4]   flags       u16 LE   Status flags");
    println!("  [4..12]  layout_id   [u8;8]   SHA-256 fingerprint (first 8 bytes)");
    println!("  [12..16] reserved    [u8;4]   Reserved for future use");
    println!();
    println!("Flags (bits):");
    println!("  0x0001   INITIALIZED");
    println!("  0x0002   FROZEN");
    println!("  0x0004   SEGMENTED");
    println!("  0x0008   CLOSING");
    println!();
    println!("Layout ID computation:");
    println!("  sha256(\"hopper:v1:{{Name}}:{{version}}:{{field}}:{{type}}:{{size}},...\")[..8]");
    println!();
    println!("Segment Registry (for segmented accounts):");
    println!("  [+0..2]  count       u16 LE   Number of segments");
    println!("  [+2..4]  reserved    u16 LE   Reserved");
    println!("  For each segment (16 bytes):");
    println!("    [+0..4]   id        [u8;4]   FNV-1a hash of segment name");
    println!("    [+4..8]   offset    u32 LE   Byte offset in account data");
    println!("    [+8..12]  size      u32 LE   Segment size in bytes");
    println!("    [+12..14] flags     u16 LE   Segment flags (includes role in bits 12-15)");
    println!("    [+14]     version   u8       Segment version");
    println!("    [+15]     reserved  u8       Reserved");
    println!();
    println!("Segment Roles (upper 4 bits of segment flags):");
    println!("  0x0000   Unclassified");
    println!("  0x1000   Core       -- primary state, must preserve");
    println!("  0x2000   Extension  -- optional fields, must preserve");
    println!("  0x3000   Journal    -- append-only log, clearable on migration");
    println!("  0x4000   Index      -- derived lookup, rebuildable");
    println!("  0x5000   Cache      -- computed cache, rebuildable");
    println!("  0x6000   Audit      -- immutable trail, must preserve");
    println!("  0x7000   Shard      -- partitioned data, must preserve");
    println!();
    println!("State Receipt (64 bytes, emitted as event):");
    println!("  [0..8]   layout_id       [u8;8]    Source layout fingerprint");
    println!("  [8..12]  before_fp       u32 LE    FNV-1a fingerprint before mutation");
    println!("  [12..16] after_fp        u32 LE    FNV-1a fingerprint after mutation");
    println!("  [16..20] changed_bytes   u32 LE    Byte count of changes");
    println!("  [20..24] changed_regions u32 LE    Number of changed regions");
    println!("  [24..28] old_size        u32 LE    Size before (0 if no resize)");
    println!("  [28..32] new_size        u32 LE    Size after (0 if no resize)");
    println!("  [32..36] segment_mask    u32 LE    Bitmask of changed segments");
    println!("  [36..40] policy_flags    u32 LE    Capability bitmask");
    println!("  [40]     inv_passed      u8        Invariants passed count");
    println!("  [41]     inv_checked     u8        Invariants checked count");
    println!("  [42]     journal_appends u8        Journal append count");
    println!("  [43]     cpi_count       u8        CPI invocation count");
    println!("  [44]     flags           u8        Status (bit 0 = committed, bit 1 = resized)");
    println!("  [45..64] reserved        [u8;19]   Reserved");
    println!();
    println!("Policy Capability Bits (in receipt policy_flags):");
    println!("  bit 0    ReadsState");
    println!("  bit 1    MutatesState");
    println!("  bit 2    TouchesJournal");
    println!("  bit 3    ExternalCall");
    println!("  bit 4    MutatesTreasury");
    println!("  bit 5    ReallocatesAccount");
    println!("  bit 6    CreatesAccount");
    println!("  bit 7    ClosesAccount");
    println!("  bit 8    ModifiesAuthority");
    println!("  bit 9    TransitionsState");
    println!();
    println!("Named Policy Packs:");
    println!("  TREASURY_WRITE      MutatesState + MutatesTreasury");
    println!("  JOURNAL_TOUCH       MutatesState + TouchesJournal");
    println!("  EXTERNAL_CALL       ExternalCall");
    println!("  SHARD_MUTATION      MutatesState");
    println!("  MIGRATION_SENSITIVE MutatesState + ReallocatesAccount");
    println!("  AUTHORITY_CHANGE    MutatesState + ModifiesAuthority");
    println!();
    println!("--- Layout Manifest JSON ---");
    println!("  {{");
    println!("    \"name\": \"Vault\",");
    println!("    \"disc\": 1,");
    println!("    \"version\": 1,");
    println!("    \"layout_id\": [1,2,3,4,5,6,7,8],");
    println!("    \"total_size\": 57,");
    println!("    \"fields\": [");
    println!("      {{\"name\":\"authority\",\"type\":\"[u8;32]\",\"size\":32,\"offset\":16}},");
    println!("      {{\"name\":\"balance\",\"type\":\"WireU64\",\"size\":8,\"offset\":48}},");
    println!("      {{\"name\":\"bump\",\"type\":\"u8\",\"size\":1,\"offset\":56}}");
    println!("    ]");
    println!("  }}");
    println!();
    println!("--- Program Manifest JSON (for Hopper Manager) ---");
    println!("  {{");
    println!("    \"name\": \"my_program\",");
    println!("    \"version\": \"0.1.0\",");
    println!("    \"description\": \"Program description\",");
    println!("    \"layouts\": [");
    println!("      {{ <layout manifest as above> }}");
    println!("    ],");
    println!("    \"instructions\": [");
    println!("      {{");
    println!("        \"name\": \"deposit\",");
    println!("        \"tag\": 1,");
    println!("        \"args\": [{{\"name\":\"amount\",\"type\":\"WireU64\",\"size\":8}}],");
    println!("        \"accounts\": [");
    println!("          {{\"name\":\"vault\",\"writable\":true,\"signer\":false,\"layout_ref\":\"VaultState\"}}");
    println!("        ],");
    println!("        \"capabilities\": [\"MutatesState\",\"MutatesTreasury\"],");
    println!("        \"policy_pack\": \"TREASURY_WRITE\",");
    println!("        \"receipt_expected\": true");
    println!("      }}");
    println!("    ],");
    println!("    \"events\": [");
    println!("      {{");
    println!("        \"name\": \"DepositEvent\",");
    println!("        \"tag\": 1,");
    println!("        \"fields\": [{{\"name\":\"amount\",\"type\":\"WireU64\",\"size\":8,\"offset\":0}}]");
    println!("      }}");
    println!("    ],");
    println!("    \"policies\": [");
    println!("      {{");
    println!("        \"name\": \"TREASURY_WRITE\",");
    println!("        \"capabilities\": [\"MutatesState\",\"MutatesTreasury\"],");
    println!("        \"requirements\": [\"SignerAuthority\",\"SnapshotCommit\"]");
    println!("      }}");
    println!("    ]");
    println!("  }}");
    println!();
    println!("Use 'hopper manager summary @manifest.json' to inspect a program manifest.");
    println!("Use 'hopper manager decode @manifest.json <hex>' to decode account fields.");
}

// -- Helpers --

/// If the argument starts with `@`, read the file contents. Otherwise return as-is.
fn resolve_manifest_arg(arg: &str) -> String {
    if let Some(path) = arg.strip_prefix('@') {
        match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(e) => {
                eprintln!("Could not read manifest file '{}': {}", path, e);
                process::exit(1);
            }
        }
    } else {
        arg.to_string()
    }
}

// ---------------------------------------------------------------------------
// Program Manifest JSON Parser (for Manager commands)
// ---------------------------------------------------------------------------

struct OwnedProgramManifest {
    name: String,
    version: String,
    description: String,
    layouts: Vec<OwnedManifest>,
    instructions: Vec<OwnedInstruction>,
    events: Vec<OwnedEvent>,
    policies: Vec<OwnedPolicy>,
}

struct OwnedInstruction {
    name: String,
    tag: u8,
    args: Vec<OwnedArg>,
    accounts: Vec<OwnedAccount>,
    capabilities: Vec<String>,
    policy_pack: String,
    receipt_expected: bool,
}

struct OwnedArg {
    name: String,
    canonical_type: String,
    size: u16,
}

struct OwnedAccount {
    name: String,
    writable: bool,
    signer: bool,
    layout_ref: String,
}

struct OwnedEvent {
    name: String,
    tag: u8,
    fields: Vec<ParsedField>,
}

struct OwnedPolicy {
    name: String,
    capabilities: Vec<String>,
    requirements: Vec<String>,
    invariants: Vec<String>,
    receipt_profile: String,
}

/// Find the matching closing bracket, handling nesting.
fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && in_string {
            escape = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Extract an array of objects from JSON. Returns the raw JSON objects as strings.
fn extract_object_array(json: &str, key: &str) -> Result<Vec<String>, String> {
    let pattern = format!("\"{}\"", key);
    let pos = match json.find(&pattern) {
        Some(p) => p,
        None => return Ok(Vec::new()), // Key not present = empty array
    };
    let after = &json[pos + pattern.len()..];
    let after = after.trim_start().strip_prefix(':').ok_or("Expected :")?;
    let after = after.trim_start();
    if !after.starts_with('[') {
        return Err(format!("Expected array for {}", key));
    }
    let end = find_matching_bracket(after, '[', ']')
        .ok_or_else(|| format!("Unterminated array for {}", key))?;
    let inner = &after[1..end];

    let mut objects = Vec::new();
    let mut remaining = inner;
    loop {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }
        if remaining.starts_with(',') {
            remaining = &remaining[1..];
            continue;
        }
        if !remaining.starts_with('{') {
            break;
        }
        let obj_end = find_matching_bracket(remaining, '{', '}')
            .ok_or("Unterminated object in array")?;
        objects.push(remaining[..=obj_end].to_string());
        remaining = &remaining[obj_end + 1..];
    }
    Ok(objects)
}

/// Extract a string array from JSON (e.g. "capabilities":["A","B"]).
fn extract_string_array(json: &str, key: &str) -> Result<Vec<String>, String> {
    let pattern = format!("\"{}\"", key);
    let pos = match json.find(&pattern) {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };
    let after = &json[pos + pattern.len()..];
    let after = after.trim_start().strip_prefix(':').ok_or("Expected :")?;
    let after = after.trim_start();
    if !after.starts_with('[') {
        return Err(format!("Expected array for {}", key));
    }
    let end = find_matching_bracket(after, '[', ']')
        .ok_or_else(|| format!("Unterminated array for {}", key))?;
    let inner = &after[1..end];

    let mut values = Vec::new();
    let mut remaining = inner;
    loop {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }
        if remaining.starts_with(',') {
            remaining = &remaining[1..];
            continue;
        }
        if remaining.starts_with('"') {
            let s = &remaining[1..];
            let q_end = s.find('"').ok_or("Unterminated string in array")?;
            values.push(s[..q_end].to_string());
            remaining = &s[q_end + 1..];
        } else {
            break;
        }
    }
    Ok(values)
}

/// Extract a boolean value from JSON.
fn extract_bool(json: &str, key: &str) -> Result<bool, String> {
    let pattern = format!("\"{}\"", key);
    let pos = match json.find(&pattern) {
        Some(p) => p,
        None => return Ok(false),
    };
    let after = &json[pos + pattern.len()..];
    let after = after.trim_start().strip_prefix(':').ok_or("Expected :")?;
    let after = after.trim_start();
    if after.starts_with("true") {
        Ok(true)
    } else {
        Ok(false)
    }
}

fn parse_program_manifest_json(json: &str) -> Result<OwnedProgramManifest, String> {
    let json = json.trim();
    if !json.starts_with('{') || !json.ends_with('}') {
        return Err("Expected JSON object".to_string());
    }

    let name = extract_string(json, "name")?;
    let version = extract_string(json, "version").unwrap_or_else(|_| "0.1.0".to_string());
    let description = extract_string(json, "description").unwrap_or_default();

    // Parse layouts
    let layout_objects = extract_object_array(json, "layouts")?;
    let mut layouts = Vec::with_capacity(layout_objects.len());
    for obj in &layout_objects {
        let pm = parse_manifest_json(obj)?;
        layouts.push(OwnedManifest::from(pm));
    }

    // Parse instructions
    let ix_objects = extract_object_array(json, "instructions")?;
    let mut instructions = Vec::with_capacity(ix_objects.len());
    for obj in &ix_objects {
        let ix_name = extract_string(obj, "name")?;
        let tag = extract_number(obj, "tag")? as u8;
        let capabilities = extract_string_array(obj, "capabilities")?;
        let policy_pack = extract_string(obj, "policy_pack").unwrap_or_default();
        let receipt_expected = extract_bool(obj, "receipt_expected")?;

        // Parse args
        let arg_objects = extract_object_array(obj, "args")?;
        let mut args = Vec::with_capacity(arg_objects.len());
        for aobj in &arg_objects {
            args.push(OwnedArg {
                name: extract_string(aobj, "name")?,
                canonical_type: extract_string(aobj, "type")?,
                size: extract_number(aobj, "size")? as u16,
            });
        }

        // Parse accounts
        let acct_objects = extract_object_array(obj, "accounts")?;
        let mut accounts = Vec::with_capacity(acct_objects.len());
        for aobj in &acct_objects {
            accounts.push(OwnedAccount {
                name: extract_string(aobj, "name")?,
                writable: extract_bool(aobj, "writable")?,
                signer: extract_bool(aobj, "signer")?,
                layout_ref: extract_string(aobj, "layout_ref").unwrap_or_default(),
            });
        }

        instructions.push(OwnedInstruction {
            name: ix_name,
            tag,
            args,
            accounts,
            capabilities,
            policy_pack,
            receipt_expected,
        });
    }

    // Parse events
    let event_objects = extract_object_array(json, "events")?;
    let mut events = Vec::with_capacity(event_objects.len());
    for obj in &event_objects {
        let ev_name = extract_string(obj, "name")?;
        let tag = extract_number(obj, "tag")? as u8;
        let fields = extract_fields(obj).unwrap_or_default();
        events.push(OwnedEvent {
            name: ev_name,
            tag,
            fields,
        });
    }

    // Parse policies
    let policy_objects = extract_object_array(json, "policies")?;
    let mut policies = Vec::with_capacity(policy_objects.len());
    for obj in &policy_objects {
        policies.push(OwnedPolicy {
            name: extract_string(obj, "name")?,
            capabilities: extract_string_array(obj, "capabilities")?,
            requirements: extract_string_array(obj, "requirements")?,
            invariants: extract_string_array(obj, "invariants").unwrap_or_default(),
            receipt_profile: extract_string(obj, "receipt_profile").unwrap_or_default(),
        });
    }

    Ok(OwnedProgramManifest {
        name,
        version,
        description,
        layouts,
        instructions,
        events,
        policies,
    })
}

/// Convert an OwnedProgramManifest to a ProgramManifest by leaking into static refs.
fn to_program_manifest(m: &OwnedProgramManifest) -> ProgramManifest {
    let layouts: Vec<LayoutManifest> = m
        .layouts
        .iter()
        .map(|l| to_manifest(l).0)
        .collect();

    let instructions: Vec<InstructionDescriptor> = m
        .instructions
        .iter()
        .map(|ix| {
            let args: Vec<ArgDescriptor> = ix
                .args
                .iter()
                .map(|a| ArgDescriptor {
                    name: leak_str(&a.name),
                    canonical_type: leak_str(&a.canonical_type),
                    size: a.size,
                })
                .collect();
            let accounts: Vec<AccountEntry> = ix
                .accounts
                .iter()
                .map(|a| AccountEntry {
                    name: leak_str(&a.name),
                    writable: a.writable,
                    signer: a.signer,
                    layout_ref: leak_str(&a.layout_ref),
                })
                .collect();
            let capabilities: Vec<&'static str> = ix
                .capabilities
                .iter()
                .map(|c| leak_str(c))
                .collect();
            InstructionDescriptor {
                name: leak_str(&ix.name),
                tag: ix.tag,
                args: Box::leak(args.into_boxed_slice()),
                accounts: Box::leak(accounts.into_boxed_slice()),
                capabilities: Box::leak(capabilities.into_boxed_slice()),
                policy_pack: leak_str(&ix.policy_pack),
                receipt_expected: ix.receipt_expected,
            }
        })
        .collect();

    let events: Vec<EventDescriptor> = m
        .events
        .iter()
        .map(|e| {
            let fields: Vec<FieldDescriptor> = e
                .fields
                .iter()
                .map(|f| FieldDescriptor {
                    name: leak_str(&f.name),
                    canonical_type: leak_str(&f.canonical_type),
                    size: f.size,
                    offset: f.offset,
                    intent: FieldIntent::Custom,
                })
                .collect();
            EventDescriptor {
                name: leak_str(&e.name),
                tag: e.tag,
                fields: Box::leak(fields.into_boxed_slice()),
            }
        })
        .collect();

    let policies: Vec<PolicyDescriptor> = m
        .policies
        .iter()
        .map(|p| {
            let caps: Vec<&'static str> = p
                .capabilities
                .iter()
                .map(|c| leak_str(c))
                .collect();
            let reqs: Vec<&'static str> = p
                .requirements
                .iter()
                .map(|r| leak_str(r))
                .collect();
            let invs: Vec<&'static str> = p
                .invariants
                .iter()
                .map(|i| leak_str(i))
                .collect();
            PolicyDescriptor {
                name: leak_str(&p.name),
                capabilities: Box::leak(caps.into_boxed_slice()),
                requirements: Box::leak(reqs.into_boxed_slice()),
                invariants: Box::leak(invs.into_boxed_slice()),
                receipt_profile: leak_str(&p.receipt_profile),
            }
        })
        .collect();

    ProgramManifest {
        name: leak_str(&m.name),
        version: leak_str(&m.version),
        description: leak_str(&m.description),
        layouts: Box::leak(layouts.into_boxed_slice()),
        instructions: Box::leak(instructions.into_boxed_slice()),
        events: Box::leak(events.into_boxed_slice()),
        policies: Box::leak(policies.into_boxed_slice()),
        layout_metadata: &[],
        compatibility_pairs: &[],
        tooling_hints: &[],
        contexts: &[],
    }
}

// ---------------------------------------------------------------------------
// Manager Command
// ---------------------------------------------------------------------------

fn cmd_manager(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager <subcommand> [args]");
        eprintln!();
        eprintln!("Subcommands:");
        eprintln!("  summary <manifest>                      Program overview");
        eprintln!("  identify <manifest> <hex>               Identify account type");
        eprintln!("  decode <manifest> <hex>                 Decode all fields with values");
        eprintln!("  instruction <manifest> <tag|name>       Instruction details and policies");
        eprintln!("  layouts <manifest>                      List all layouts with fields");
        eprintln!("  policies <manifest>                     List policy packs with mappings");
        eprintln!("  events <manifest>                       List events with fields");
        eprintln!("  fingerprints <manifest>                 Show all layout fingerprints");
        eprintln!("  compat <manifest> <hex-old> <hex-new>   Compare two account versions");
        eprintln!("  receipt <hex-64-bytes>                  Decode a receipt from wire bytes");
        eprintln!("  explain <manifest>                      Aggregated human-readable summary");
        eprintln!("  diff <manifest> <hex-before> <hex-after>  Semantic field-level diff");
        eprintln!("  simulate <manifest> <instruction>        Preview instruction requirements");
        eprintln!("  fetch <program-id> [--rpc <url>]        Fetch manifest from on-chain");
        eprintln!("  interactive <manifest>                   Interactive terminal explorer");
        process::exit(1);
    }

    match args[0].as_str() {
        "summary" => cmd_manager_summary(&args[1..]),
        "identify" => cmd_manager_identify(&args[1..]),
        "decode" => cmd_manager_decode(&args[1..]),
        "instruction" => cmd_manager_instruction(&args[1..]),
        "layouts" => cmd_manager_layouts(&args[1..]),
        "policies" => cmd_manager_policies(&args[1..]),
        "events" => cmd_manager_events(&args[1..]),
        "fingerprints" => cmd_manager_fingerprints(&args[1..]),
        "compat" => cmd_manager_compat(&args[1..]),
        "receipt" => cmd_manager_receipt(&args[1..]),
        "explain" => cmd_manager_explain(&args[1..]),
        "diff" => cmd_manager_diff(&args[1..]),
        "fetch" => cmd_manager_fetch(&args[1..]),
        "simulate" => cmd_manager_simulate(&args[1..]),
        "interactive" | "ui" => cmd_interactive(&args[1..]),
        other => {
            eprintln!("Unknown manager subcommand: {}", other);
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// On-Chain Fetch Commands
// ---------------------------------------------------------------------------

/// Parse common fetch flags from args: (program_id, rpc_override, json_mode)
fn parse_fetch_args(args: &[String]) -> (String, Option<String>, bool) {
    let mut program_id = None;
    let mut rpc_override = None;
    let mut json_mode = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" => {
                if i + 1 >= args.len() {
                    eprintln!("--rpc requires a URL argument");
                    process::exit(1);
                }
                rpc_override = Some(args[i + 1].clone());
                i += 2;
            }
            "--json" => {
                json_mode = true;
                i += 1;
            }
            other => {
                if program_id.is_none() {
                    program_id = Some(other.to_string());
                } else {
                    eprintln!("Unexpected argument: {}", other);
                    process::exit(1);
                }
                i += 1;
            }
        }
    }
    let pid = match program_id {
        Some(p) => p,
        None => {
            eprintln!("Missing required <program-id> argument");
            process::exit(1);
        }
    };
    (pid, rpc_override, json_mode)
}

/// Fetch a Hopper manifest from on-chain, returning the raw JSON string.
fn fetch_manifest_json(program_id_str: &str, rpc_override: Option<&str>) -> String {
    let rpc_url = rpc::resolve_rpc_url(rpc_override);
    let program_id = match rpc::decode_pubkey(program_id_str) {
        Ok(pk) => pk,
        Err(e) => {
            eprintln!("Invalid program ID: {}", e);
            process::exit(1);
        }
    };

    let (pda, bump) = match rpc::find_program_address(
        &[hopper_schema::MANIFEST_SEED],
        &program_id,
    ) {
        Some(result) => result,
        None => {
            eprintln!("Failed to derive manifest PDA (no valid bump found)");
            process::exit(1);
        }
    };

    let pda_b58 = rpc::encode_pubkey(&pda);
    eprintln!("Manifest PDA: {} (bump {})", pda_b58, bump);
    eprintln!("RPC endpoint: {}", rpc_url);
    eprintln!();

    let account = match rpc::get_account_info(&rpc_url, &pda_b58) {
        Ok(Some(info)) => info,
        Ok(None) => {
            eprintln!("No manifest account found at PDA {}", pda_b58);
            eprintln!();
            eprintln!("The program {} does not have an on-chain Hopper manifest.", program_id_str);
            eprintln!("To publish a manifest, use the hopper_manifest!() macro in your program.");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("RPC error: {}", e);
            process::exit(1);
        }
    };

    let manifest = match rpc::decode_manifest_account(&account.data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to decode manifest account: {}", e);
            eprintln!("Account owner: {}", account.owner);
            eprintln!("Account size:  {} bytes", account.data.len());
            process::exit(1);
        }
    };

    eprintln!("Manifest version: {}", manifest.version);
    eprintln!("JSON size:        {} bytes", manifest.json.len());
    eprintln!();

    manifest.json
}

fn cmd_fetch(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper fetch <program-id> [--rpc <url>] [--json]");
        eprintln!();
        eprintln!("Fetch a program's Hopper manifest from on-chain and display it.");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --rpc <url>  Solana RPC endpoint (default: SOLANA_RPC_URL env or mainnet)");
        eprintln!("  --json       Output raw manifest JSON instead of summary");
        process::exit(1);
    }

    let (program_id, rpc_override, json_mode) = parse_fetch_args(args);
    let json = fetch_manifest_json(&program_id, rpc_override.as_deref());

    if json_mode {
        println!("{}", json);
    } else {
        let prog = load_program_manifest_from_json(&json);
        println!("{}", prog);
    }
}

fn cmd_manager_fetch(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager fetch <program-id> [--rpc <url>]");
        eprintln!();
        eprintln!("Fetch a program's Hopper manifest from on-chain and show manager summary.");
        process::exit(1);
    }

    let (program_id, rpc_override, _) = parse_fetch_args(args);
    let json = fetch_manifest_json(&program_id, rpc_override.as_deref());
    let prog = load_program_manifest_from_json(&json);
    println!("{}", prog);
}

fn cmd_interactive(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper interactive <manifest>");
        eprintln!("       hopper manager interactive <manifest>");
        eprintln!("       hopper ui <manifest>");
        eprintln!();
        eprintln!("Launch an interactive terminal UI for exploring a program manifest.");
        eprintln!("Manifest can be inline JSON or @path/to/file.json.");
        process::exit(1);
    }

    let prog = load_program_manifest(&args[0]);
    let mut session = interactive::Session::new(&prog);
    if let Err(e) = session.run() {
        eprintln!("Interactive session error: {}", e);
        process::exit(1);
    }
}

fn load_program_manifest(arg: &str) -> ProgramManifest {
    let resolved = resolve_manifest_arg(arg);
    load_program_manifest_from_json(&resolved)
}

fn load_program_manifest_from_json(json: &str) -> ProgramManifest {
    match parse_program_manifest_json(json) {
        Ok(m) => to_program_manifest(&m),
        Err(e) => {
            eprintln!("Program manifest parse error: {}", e);
            process::exit(1);
        }
    }
}

fn cmd_manager_summary(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager summary <manifest>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);
    println!("{}", prog);
}

fn cmd_manager_identify(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper manager identify <manifest> <hex-data>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);
    let data = match hex_decode(&args[1]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Hex decode error: {}", e);
            process::exit(1);
        }
    };

    if data.len() < 16 {
        eprintln!("Data too short for Hopper header (need 16 bytes, got {})", data.len());
        process::exit(1);
    }

    let header = require_header(&data);
    println!("=== Account Identification ===");
    println!("  Data size    : {} bytes", data.len());
    println!("  Header disc  : {}", header.disc);
    println!("  Header ver   : {}", header.version);
    println!("  Layout ID    : {}", hex_encode(&header.layout_id));
    println!();

    match prog.identify_from_data(&data) {
        Some(layout) => {
            println!("  MATCH: {} v{}", layout.name, layout.version);
            println!("  Expected size: {} bytes", layout.total_size);
            println!("  Fields       : {}", layout.field_count);
            if data.len() != layout.total_size {
                println!("  WARNING: data size ({}) != expected size ({})",
                    data.len(), layout.total_size);
            }
            println!();
            println!("Use 'hopper manager decode' to see field values.");
        }
        None => {
            println!("  NO MATCH: This account does not match any layout in the manifest.");
            println!();
            println!("Known layouts:");
            for l in prog.layouts.iter() {
                println!("    {} v{} (disc={}, id={})",
                    l.name, l.version, l.disc, hex_encode(&l.layout_id));
            }
        }
    }
}

fn cmd_manager_decode(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper manager decode <manifest> <hex-data>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);
    let data = match hex_decode(&args[1]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Hex decode error: {}", e);
            process::exit(1);
        }
    };

    if data.len() < 16 {
        eprintln!("Data too short for Hopper header");
        process::exit(1);
    }

    let header = require_header(&data);

    let layout = match prog.identify_from_data(&data) {
        Some(l) => l,
        None => {
            eprintln!("Cannot identify account type (disc={}, layout_id={})",
                header.disc, hex_encode(&header.layout_id));
            eprintln!("Use 'hopper manager identify' for diagnostics.");
            process::exit(1);
        }
    };

    println!("=== Account Decode: {} v{} ===", layout.name, layout.version);
    println!("  Size: {} bytes (expected {})", data.len(), layout.total_size);
    println!("  Flags: {} (0x{:04x})", format_flags(header.flags), header.flags);
    println!();

    if layout.field_count == 0 {
        println!("  (no field descriptors in manifest)");
        return;
    }

    let (count, fields) = decode_account_fields::<64>(&data, layout);
    let mut buf = [0u8; 128];
    println!(
        "  {:>4}  {:>20}  {:>12}  {:>6}  {:>6}  Value",
        "#", "Field", "Type", "Offset", "Size"
    );
    println!("  {}", "-".repeat(76));
    for (i, slot) in fields.iter().enumerate().take(count) {
        if let Some(ref field) = slot {
            let val_len = field.format_value(&mut buf);
            let val_str = std::str::from_utf8(&buf[..val_len]).unwrap_or("???");
            println!(
                "  {:>4}  {:>20}  {:>12}  {:>6}  {:>6}  {}",
                i, field.name, field.canonical_type, field.offset, field.size, val_str,
            );
        }
    }

    // Summary
    println!();
    println!("  Decoded {}/{} fields.", count, layout.field_count);
}

fn cmd_manager_instruction(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper manager instruction <manifest> <tag>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);
    let tag: u8 = match args[1].parse() {
        Ok(t) => t,
        Err(_) => {
            // Try matching by name
            let name = &args[1];
            let mut found = None;
            for ix in prog.instructions.iter() {
                if ix.name == name.as_str() {
                    found = Some(ix.tag);
                    break;
                }
            }
            match found {
                Some(t) => t,
                None => {
                    eprintln!("Unknown instruction: '{}'. Known:", name);
                    for ix in prog.instructions.iter() {
                        eprintln!("  {}  {}", ix.tag, ix.name);
                    }
                    process::exit(1);
                }
            }
        }
    };

    let ix = match prog.find_instruction(tag) {
        Some(ix) => ix,
        None => {
            eprintln!("No instruction with tag {}", tag);
            process::exit(1);
        }
    };

    println!("=== Instruction: {} (tag {}) ===", ix.name, ix.tag);
    println!();

    if !ix.accounts.is_empty() {
        println!("  Accounts ({}):", ix.accounts.len());
        for (i, acct) in ix.accounts.iter().enumerate() {
            let mut flags = Vec::new();
            if acct.writable { flags.push("writable"); }
            if acct.signer { flags.push("signer"); }
            let flag_str = if flags.is_empty() { "read-only".to_string() } else { flags.join(", ") };
            let layout_str = if acct.layout_ref.is_empty() { "" } else { acct.layout_ref };
            if layout_str.is_empty() {
                println!("    [{}] {:20} ({})", i, acct.name, flag_str);
            } else {
                println!("    [{}] {:20} ({}) -> {}", i, acct.name, flag_str, layout_str);
            }
        }
        println!();
    }

    if !ix.args.is_empty() {
        println!("  Arguments ({}):", ix.args.len());
        for arg in ix.args.iter() {
            println!("    {:20} : {} ({} bytes)", arg.name, arg.canonical_type, arg.size);
        }
        println!();
    }

    if !ix.capabilities.is_empty() {
        println!("  Capabilities:");
        for cap in ix.capabilities.iter() {
            println!("    - {}", cap);
        }
        println!();
    }

    if !ix.policy_pack.is_empty() {
        println!("  Policy pack: {}", ix.policy_pack);
        if let Some(policy) = prog.find_policy(ix.policy_pack) {
            println!("    Requirements:");
            for req in policy.requirements.iter() {
                println!("      - {}", req);
            }
        }
        println!();
    }

    println!("  Receipt expected: {}", if ix.receipt_expected { "YES" } else { "NO" });
}

fn cmd_manager_layouts(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager layouts <manifest>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);

    println!("=== Layouts ({}) ===", prog.layout_count());
    println!();

    for layout in prog.layouts.iter() {
        println!("  {} v{} (disc={}, {} bytes)", layout.name, layout.version, layout.disc, layout.total_size);
        println!("    Layout ID: {}", hex_encode(&layout.layout_id));
        if layout.field_count > 0 {
            println!("    Fields ({}):", layout.field_count);
            for i in 0..layout.field_count {
                let f = &layout.fields[i];
                println!(
                    "      [{:>3}..{:>3}] {:20} : {} ({} bytes)",
                    f.offset,
                    f.offset + f.size,
                    f.name,
                    f.canonical_type,
                    f.size,
                );
            }
        }
        println!();
    }
}

fn cmd_manager_policies(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager policies <manifest>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);

    if prog.policies.is_empty() {
        println!("No policies defined in manifest.");
        return;
    }

    println!("=== Policy Packs ({}) ===", prog.policies.len());
    println!();

    for policy in prog.policies.iter() {
        println!("  {}", policy.name);
        if !policy.capabilities.is_empty() {
            println!("    Capabilities:");
            for cap in policy.capabilities.iter() {
                println!("      - {}", cap);
            }
        }
        if !policy.requirements.is_empty() {
            println!("    Requirements:");
            for req in policy.requirements.iter() {
                println!("      - {}", req);
            }
        }
        println!();
    }

    // Show which instructions use which policies
    println!("  Instruction -> Policy mapping:");
    for ix in prog.instructions.iter() {
        if !ix.policy_pack.is_empty() {
            println!("    {:20} -> {}", ix.name, ix.policy_pack);
        }
    }
}

fn cmd_manager_events(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager events <manifest>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);

    if prog.events.is_empty() {
        println!("No events defined in manifest.");
        return;
    }

    println!("=== Events ({}) ===", prog.events.len());
    println!();

    for event in prog.events.iter() {
        println!("  {} (tag {})", event.name, event.tag);
        if event.fields.is_empty() {
            println!("    (no fields)");
        } else {
            println!("    Fields ({}):", event.fields.len());
            for f in event.fields.iter() {
                println!(
                    "      [{:>3}..{:>3}] {:20} : {} ({} bytes)",
                    f.offset,
                    f.offset + f.size,
                    f.name,
                    f.canonical_type,
                    f.size,
                );
            }
        }
        println!();
    }

    // Show which instructions emit which events
    let has_receipt_ix: Vec<_> = prog.instructions.iter()
        .filter(|ix| ix.receipt_expected)
        .collect();
    if !has_receipt_ix.is_empty() {
        println!("  Instructions with receipt emissions:");
        for ix in &has_receipt_ix {
            println!("    {:20} (tag {})", ix.name, ix.tag);
        }
    }
}

fn cmd_manager_fingerprints(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager fingerprints <manifest>");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);

    println!("=== Layout Fingerprints ===");
    println!();
    println!("  {:>20}  {:>3}  {:>3}  {:>6}  Layout ID", "Name", "D", "V", "Size");
    println!("  {}", "-".repeat(60));

    for layout in prog.layouts.iter() {
        println!(
            "  {:>20}  {:>3}  {:>3}  {:>6}  {}",
            layout.name, layout.disc, layout.version, layout.total_size,
            hex_encode(&layout.layout_id),
        );
    }

    // Check for disc collisions
    println!();
    let mut seen_discs = std::collections::HashMap::new();
    for layout in prog.layouts.iter() {
        if let Some(prev) = seen_discs.insert(layout.disc, layout.name) {
            println!("  WARNING: Disc {} shared by '{}' and '{}'", layout.disc, prev, layout.name);
        }
    }

    // Check for layout_id collisions
    let mut seen_ids = std::collections::HashMap::new();
    for layout in prog.layouts.iter() {
        let id_hex = hex_encode(&layout.layout_id);
        if let Some(prev) = seen_ids.insert(id_hex.clone(), layout.name) {
            println!("  WARNING: Layout ID {} shared by '{}' and '{}'", id_hex, prev, layout.name);
        }
    }

    if seen_discs.len() == prog.layouts.len() && seen_ids.len() == prog.layouts.len() {
        println!("  All discriminators and layout IDs are unique.");
    }
}

fn cmd_manager_compat(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: hopper manager compat <manifest> <hex-old> <hex-new>");
        eprintln!("  Compares two account data blobs and reports compatibility.");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);
    let old_data = match hex_decode(&args[1]) {
        Ok(d) => d,
        Err(e) => { eprintln!("Hex decode error (old): {}", e); process::exit(1); }
    };
    let new_data = match hex_decode(&args[2]) {
        Ok(d) => d,
        Err(e) => { eprintln!("Hex decode error (new): {}", e); process::exit(1); }
    };

    if old_data.len() < 16 || new_data.len() < 16 {
        eprintln!("Both data blobs must be at least 16 bytes (header).");
        process::exit(1);
    }

    let old_header = require_header(&old_data);
    let new_header = require_header(&new_data);

    println!("=== Compatibility Analysis ===");
    println!();
    println!("  Old: disc={}, ver={}, layout_id={}, size={}",
        old_header.disc, old_header.version,
        hex_encode(&old_header.layout_id), old_data.len());
    println!("  New: disc={}, ver={}, layout_id={}, size={}",
        new_header.disc, new_header.version,
        hex_encode(&new_header.layout_id), new_data.len());
    println!();

    if old_header.disc != new_header.disc {
        println!("  RESULT: Different discriminators. These are different account types.");
        return;
    }

    if old_header.layout_id == new_header.layout_id {
        println!("  RESULT: Same layout ID. Same schema version, no compat issue.");
        return;
    }

    // Try to find both layouts in the manifest
    let old_layout = prog.identify_from_data(&old_data);
    let new_layout = prog.identify_from_data(&new_data);

    match (old_layout, new_layout) {
        (Some(ol), Some(nl)) => {
            println!("  Old layout: {} v{}", ol.name, ol.version);
            println!("  New layout: {} v{}", nl.name, nl.version);

            let report = compare_fields::<64>(ol, nl);
            println!();
            println!("  Field-level changes:");
            for i in 0..report.len() {
                if let Some(entry) = report.get(i) {
                    let status = match entry.status {
                        FieldCompat::Identical => "identical",
                        FieldCompat::Changed => "CHANGED",
                        FieldCompat::Added => "added",
                        FieldCompat::Removed => "REMOVED",
                    };
                    println!("    {:20} : {}", entry.name, status);
                }
            }
            let verdict = CompatibilityVerdict::between(ol, nl);
            println!();
            println!("  Verdict: {}", verdict.name());
            match verdict {
                CompatibilityVerdict::Identical => {
                    println!("  RESULT: Identical layout. No changes.");
                }
                CompatibilityVerdict::WireCompatible => {
                    println!("  RESULT: Wire-compatible. Byte layout identical, semantic metadata differs.");
                }
                CompatibilityVerdict::AppendSafe => {
                    println!("  RESULT: Append-safe. Old field prefix preserved, no migration needed.");
                }
                CompatibilityVerdict::MigrationRequired => {
                    println!("  RESULT: Migration required. Data is not directly backward-readable.");
                }
                CompatibilityVerdict::Incompatible => {
                    println!("  RESULT: Incompatible. Breaking change detected.");
                }
            }
        }
        (Some(ol), None) => {
            println!("  Old layout identified: {} v{}", ol.name, ol.version);
            println!("  New layout: NOT IN MANIFEST");
            println!("  RESULT: Cannot determine compatibility (new layout unknown).");
        }
        (None, Some(nl)) => {
            println!("  Old layout: NOT IN MANIFEST");
            println!("  New layout identified: {} v{}", nl.name, nl.version);
            println!("  RESULT: Cannot determine compatibility (old layout unknown).");
        }
        (None, None) => {
            println!("  Neither layout found in manifest.");
            println!("  RESULT: Cannot determine compatibility.");
        }
    }
}

fn cmd_manager_receipt(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager receipt <hex-64-bytes>");
        process::exit(1);
    }
    let data = match hex_decode(&args[0]) {
        Ok(d) => d,
        Err(e) => { eprintln!("Hex decode error: {}", e); process::exit(1); }
    };

    if data.len() < 64 {
        eprintln!("Receipt data must be exactly 64 bytes (got {})", data.len());
        process::exit(1);
    }

    let r = match DecodedReceipt::from_bytes(&data) {
        Some(r) => r,
        None => { eprintln!("Failed to decode receipt"); process::exit(1); }
    };

    let phase = Phase::from_tag(r.phase);
    let impact = CompatImpact::from_tag(r.compat_impact);

    println!("=== State Receipt ===");
    println!();
    println!("  Layout ID           : {}", hex_encode(&r.layout_id));
    println!("  Phase               : {} ({})", phase.name(), r.phase);
    println!("  Committed           : {}", r.committed);
    println!();
    println!("  Changed fields mask : 0x{:016x}", r.changed_fields);
    println!("  Changed bytes       : {}", r.changed_bytes);
    println!("  Changed regions     : {}", r.changed_regions);
    println!("  Was resized         : {} ({} -> {} bytes)", r.was_resized, r.old_size, r.new_size);
    println!();
    println!("  Before fingerprint  : {}", hex_encode(&r.before_fingerprint));
    println!("  After fingerprint   : {}", hex_encode(&r.after_fingerprint));
    let fp_changed = r.before_fingerprint != r.after_fingerprint;
    println!("  Fingerprint changed : {}", fp_changed);
    println!();
    println!("  Invariants passed   : {}", r.invariants_passed);
    println!("  Invariants checked  : {}", r.invariants_checked);
    println!("  CPI invoked         : {} ({} calls)", r.cpi_invoked, r.cpi_count);
    println!("  Journal appends     : {}", r.journal_appends);
    println!("  Segment mask        : 0x{:04x}", r.segment_changed_mask);
    println!("  Policy flags        : 0x{:08x}", r.policy_flags);
    println!();
    println!("  Compat impact       : {} ({})", impact.name(), r.compat_impact);
    println!("  Validation bundle   : {}", r.validation_bundle_id);
    println!("  Migration flags     : 0b{:08b}", r.migration_flags);
    if r.migration_flags & 0x01 != 0 { println!("    - Migration triggered"); }
    if r.migration_flags & 0x02 != 0 { println!("    - Realloc performed"); }
    if r.migration_flags & 0x04 != 0 { println!("    - Schema version bumped"); }
}

fn cmd_manager_explain(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: hopper manager explain <manifest>");
        eprintln!("  Aggregated human-readable summary of the program manifest.");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);

    println!("=== Program Explanation ===");
    println!();
    println!("  Name        : {}", prog.name);
    println!("  Version     : {}", prog.version);
    println!("  Description : {}", prog.description);
    println!();

    // Layouts
    println!("  Layouts ({})", prog.layouts.len());
    for l in prog.layouts.iter() {
        let fp = LayoutFingerprint::from_manifest(l);
        println!("    {} v{} | disc={} | {} fields | {} bytes | wire={} sem={}",
            l.name, l.version, l.disc, l.field_count, l.total_size,
            hex_encode(&fp.wire_hash), hex_encode(&fp.semantic_hash));
    }
    println!();

    // Instructions
    println!("  Instructions ({})", prog.instructions.len());
    for ix in prog.instructions.iter() {
        println!("    [{}] {} | {} args | {} accounts",
            ix.tag, ix.name, ix.args.len(), ix.accounts.len());
    }
    println!();

    // Policies
    println!("  Policies ({})", prog.policies.len());
    for p in prog.policies.iter() {
        println!("    {} | {}cap {}req {}inv | receipt={}",
            p.name, p.capabilities.len(), p.requirements.len(),
            p.invariants.len(), p.receipt_profile);
    }
    println!();

    // Events
    println!("  Events ({})", prog.events.len());
    for ev in prog.events.iter() {
        println!("    [{}] {} | {} fields", ev.tag, ev.name, ev.fields.len());
    }
    println!();

    // Compat pairs
    if !prog.compatibility_pairs.is_empty() {
        println!("  Compatibility Rules ({})", prog.compatibility_pairs.len());
        for cp in prog.compatibility_pairs.iter() {
            let policy = match cp.policy {
                MigrationPolicy::NoOp => "noop",
                MigrationPolicy::AppendOnly => "append-only",
                MigrationPolicy::RequiresMigration => "migration",
                MigrationPolicy::Incompatible => "incompatible",
            };
            println!("    {} v{} → {} v{} | {} | backward={}",
                cp.from_layout, cp.from_version,
                cp.to_layout, cp.to_version,
                policy, cp.backward_readable);
        }
        println!();
    }

    // Tooling hints
    if !prog.tooling_hints.is_empty() {
        println!("  Tooling Hints");
        for h in prog.tooling_hints.iter() {
            println!("    - {}", h);
        }
    }
}

fn cmd_manager_diff(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: hopper manager diff <manifest> <hex-before> <hex-after>");
        eprintln!("  Semantic field-level diff between two account states.");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);
    let before = match hex_decode(&args[1]) {
        Ok(d) => d,
        Err(e) => { eprintln!("Hex decode error (before): {}", e); process::exit(1); }
    };
    let after = match hex_decode(&args[2]) {
        Ok(d) => d,
        Err(e) => { eprintln!("Hex decode error (after): {}", e); process::exit(1); }
    };

    if before.len() < 16 || after.len() < 16 {
        eprintln!("Both data blobs must be at least 16 bytes (header).");
        process::exit(1);
    }

    let before_header = require_header(&before);
    let after_header = require_header(&after);

    println!("=== Semantic Diff ===");
    println!();

    let before_layout = prog.identify_from_data(&before);
    let after_layout = prog.identify_from_data(&after);

    match (before_layout, after_layout) {
        (Some(bl), Some(al)) => {
            println!("  Before : {} v{} (disc={})", bl.name, bl.version, bl.disc);
            println!("  After  : {} v{} (disc={})", al.name, al.version, al.disc);
            println!();

            // Verdict
            let verdict = CompatibilityVerdict::between(bl, al);
            println!("  Verdict: {}", verdict.name());
            println!();

            // Field-level diff with values
            let (_bc, before_fields) = decode_account_fields::<64>(&before, bl);
            let (_ac, after_fields) = decode_account_fields::<64>(&after, al);

            println!("  Field-level changes:");
            // Compare shared fields by index
            let max_fields = std::cmp::max(bl.field_count, al.field_count);
            let mut diffs_found = 0usize;
            for i in 0..max_fields {
                if i < bl.field_count && i < al.field_count {
                    let bf = &bl.fields[i];
                    let af = &al.fields[i];
                    let bv = before_fields[i].as_ref().map(|f| f.raw).unwrap_or(&[]);
                    let av = after_fields[i].as_ref().map(|f| f.raw).unwrap_or(&[]);
                    if bv != av || bf.name != af.name || bf.canonical_type != af.canonical_type {
                        println!("    {:20} : {} → {}",
                            bf.name,
                            hex_encode(bv),
                            hex_encode(av));
                        diffs_found += 1;
                    }
                } else if i < al.field_count {
                    let af = &al.fields[i];
                    let av = after_fields[i].as_ref().map(|f| f.raw).unwrap_or(&[]);
                    println!("    {:20} : (added) = {}", af.name, hex_encode(av));
                    diffs_found += 1;
                } else if i < bl.field_count {
                    let bf = &bl.fields[i];
                    println!("    {:20} : (removed)", bf.name);
                    diffs_found += 1;
                }
            }

            if diffs_found == 0 {
                println!("    (no field-level differences)");
            }

            // Size diff
            if before.len() != after.len() {
                println!();
                println!("  Size: {} → {} bytes ({}{})",
                    before.len(), after.len(),
                    if after.len() > before.len() { "+" } else { "" },
                    after.len() as isize - before.len() as isize);
            }
        }
        (Some(bl), None) => {
            println!("  Before : {} v{}", bl.name, bl.version);
            println!("  After  : UNKNOWN LAYOUT (id={})", hex_encode(&after_header.layout_id));
            println!("  Cannot compute semantic diff: after layout not in manifest.");
        }
        (None, Some(al)) => {
            println!("  Before : UNKNOWN LAYOUT (id={})", hex_encode(&before_header.layout_id));
            println!("  After  : {} v{}", al.name, al.version);
            println!("  Cannot compute semantic diff: before layout not in manifest.");
        }
        (None, None) => {
            println!("  Before : UNKNOWN LAYOUT (id={})", hex_encode(&before_header.layout_id));
            println!("  After  : UNKNOWN LAYOUT (id={})", hex_encode(&after_header.layout_id));
            println!("  Cannot compute semantic diff: neither layout is in the manifest.");
        }
    }
}

fn cmd_manager_simulate(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: hopper manager simulate <manifest> <instruction-name|tag>");
        eprintln!("  Preview what an instruction requires: accounts, args, policies, receipt.");
        process::exit(1);
    }
    let prog = load_program_manifest(&args[0]);
    let query = &args[1];

    // Find instruction by name or tag
    let ix = prog.instructions.iter().find(|ix| {
        ix.name == query.as_str() || format!("{}", ix.tag) == query.as_str()
    });

    let ix = match ix {
        Some(ix) => ix,
        None => {
            eprintln!("Instruction '{}' not found.", query);
            eprintln!();
            eprintln!("Available instructions:");
            for ix in prog.instructions.iter() {
                eprintln!("  [{}] {}", ix.tag, ix.name);
            }
            process::exit(1);
        }
    };

    println!("=== Simulate: {} (tag {}) ===", ix.name, ix.tag);
    println!();

    // Required accounts
    println!("  Accounts required ({}):", ix.accounts.len());
    for (i, acc) in ix.accounts.iter().enumerate() {
        let mut flags = Vec::new();
        if acc.signer { flags.push("SIGNER"); }
        if acc.writable { flags.push("WRITABLE"); }
        let flag_str = if flags.is_empty() { "read-only".to_string() }
            else { flags.join(" + ") };
        let layout_note = if acc.layout_ref.is_empty() { String::new() }
            else { format!(" → layout:{}", acc.layout_ref) };
        println!("    #{}: {} [{}]{}", i, acc.name, flag_str, layout_note);
    }
    println!();

    // Required arguments (instruction data after tag byte)
    if ix.args.is_empty() {
        println!("  Arguments: none (tag byte only)");
    } else {
        let total_size: u16 = ix.args.iter().map(|a| a.size).sum();
        println!("  Arguments ({}, {} bytes after tag):", ix.args.len(), total_size);
        let mut offset = 1usize; // tag byte
        for arg in ix.args.iter() {
            println!("    @{}: {} ({}, {} bytes)",
                offset, arg.name, arg.canonical_type, arg.size);
            offset += arg.size as usize;
        }
        println!("  Total instruction data: {} bytes", offset);
    }
    println!();

    // Policy constraints
    if !ix.policy_pack.is_empty() {
        println!("  Policy: {}", ix.policy_pack);
        // Look up the policy descriptor for full details
        if let Some(pol) = prog.policies.iter().find(|p| p.name == ix.policy_pack) {
            if !pol.capabilities.is_empty() {
                println!("    Capabilities:");
                for cap in pol.capabilities.iter() {
                    println!("      - {}", cap);
                }
            }
            if !pol.requirements.is_empty() {
                println!("    Requirements:");
                for req in pol.requirements.iter() {
                    println!("      - {}", req);
                }
            }
            if !pol.invariants.is_empty() {
                println!("    Invariants checked:");
                for inv in pol.invariants.iter() {
                    println!("      - {}", inv);
                }
            }
        }
    } else {
        println!("  Policy: (none / custom)");
    }
    println!();

    // Receipt preview
    if ix.receipt_expected {
        println!("  Receipt: YES. This instruction emits a state receipt.");
        println!("    The receipt captures:");
        println!("      - Phase (Init/Update/Close/Migrate)");
        println!("      - Changed field bitmask");
        println!("      - Segment change mask");
        println!("      - Fingerprint before/after");
        println!("      - Compatibility impact");
        println!("      - CPI invocation flag");
    } else {
        println!("  Receipt: NO. This instruction does not emit a receipt.");
    }

    // Capability summary
    if !ix.capabilities.is_empty() {
        println!();
        println!("  Capabilities:");
        for cap in ix.capabilities.iter() {
            println!("    - {}", cap);
        }
    }
}

fn parse_or_exit(json: &str) -> OwnedManifest {
    let resolved = resolve_manifest_arg(json);
    match parse_manifest_json(&resolved) {
        Ok(p) => p.into(),
        Err(e) => {
            eprintln!("JSON parse error: {}", e);
            process::exit(1);
        }
    }
}

/// Convert an OwnedManifest to a LayoutManifest with borrowed field descriptors.
///
/// Returns the manifest and the owned field descriptor vector (must outlive manifest).
fn to_manifest(m: &OwnedManifest) -> (LayoutManifest, Vec<FieldDescriptor>) {
    let fields: Vec<FieldDescriptor> = m
        .fields
        .iter()
        .map(|f| FieldDescriptor {
            name: leak_str(&f.name),
            canonical_type: leak_str(&f.canonical_type),
            size: f.size,
            offset: f.offset,
            intent: FieldIntent::Custom,
        })
        .collect();

    let manifest = LayoutManifest {
        name: leak_str(&m.name),
        disc: m.disc,
        version: m.version,
        layout_id: m.layout_id,
        total_size: m.total_size,
        field_count: fields.len(),
        fields: leak_slice(&fields),
    };

    (manifest, fields)
}

/// Leak a string to get a 'static reference.
///
/// This is acceptable for a short-lived CLI binary.
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Leak a vec to get a 'static slice reference.
fn leak_slice(v: &[FieldDescriptor]) -> &'static [FieldDescriptor] {
    Box::leak(v.to_vec().into_boxed_slice())
}
