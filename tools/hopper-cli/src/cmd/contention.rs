//! `hopper contention` — the write-lock and signature footprint a program
//! *declares*, computed from its manifest.
//!
//! Every number here comes from the declaration, not a measurement: the
//! account list plus the same `writeRanges` / `lamportAccounts` the
//! runtime enforces. That makes the output exact and reproducible offline,
//! and it is why the demotion column exists at all — an account declared
//! writable that a *mutation-complete* write set proves is never mutated
//! can be sent read-only, which removes a real write lock
//! (`WRITE_LOCK_UNITS` = 300 CU) and one account the transaction
//! serializes against.
//!
//! # This is NOT a transaction's block cost
//!
//! Agave charges five terms: signatures, write locks, instruction-data
//! bytes, the **requested compute limit** (`programs_execution_cost` —
//! usually the dominant term, 200,000 CU by default), and the requested
//! loaded-data limit. Only the first two are fixed by a declaration; the
//! rest are caller choices no manifest analysis can supply. So the
//! `Lock CU` column is the declaration's share of the leader's price, not
//! the price. Two further scope limits, both real:
//!
//! - the **fee payer**'s write lock is not counted (it is a property of
//!   the message, not of any instruction), and
//! - locks and signatures are charged once per transaction over
//!   deduplicated keys, so summing across instructions that share an
//!   account over-counts.
//!
//! It also does not report the program's own compute consumption: no
//! static model can derive that honestly, and Hopper does not fabricate it
//! (`cuEstimate` stays 0 until something measures it).

use std::process;

use hopper_schema::{cost_model, ProgramManifest};

/// Render the contention table for `manifest`. Returns the number of
/// instructions that exceeded `max_block_cost`, when a ceiling was given.
pub fn report(manifest: &ProgramManifest, max_block_cost: Option<u64>) -> u32 {
    println!("hopper contention");
    println!("  program: {} v{}", manifest.name, manifest.version);
    println!();
    println!(
        "{:<24} {:>4} {:>6} {:>5} {:>8} {:>7} {:>10} {:>5}",
        "Instruction", "W", "W-eff", "Sigs", "Lock CU", "Saved", "Proven RO", "Rem"
    );
    println!("{}", "-".repeat(80));

    let mut total_declared = 0u64;
    let mut total_effective = 0u64;
    let mut total_saved = 0u64;
    let mut instructions_with_provable = 0u32;
    let mut max_provable_in_one = 0u32;
    let mut demotion_capable = 0u32;
    let mut remaining_capable = 0u32;
    let mut over_budget = 0u32;

    for instruction in manifest.instructions {
        let profile = instruction.contention_profile();
        let lock_cost = profile.declared_lock_and_signature_cost();
        let saved = profile.write_lock_cost_saved();
        total_declared += profile.declared_write_lock_cost();
        total_effective += profile.effective_write_lock_cost();
        total_saved += saved;
        if profile.provably_read_only > 0 {
            instructions_with_provable += 1;
            max_provable_in_one = max_provable_in_one.max(profile.provably_read_only);
        }
        if profile.demotion_available {
            demotion_capable += 1;
        }
        if profile.remaining_accounts_max > 0 {
            remaining_capable += 1;
        }

        let over = max_block_cost.is_some_and(|ceiling| lock_cost > ceiling);
        if over {
            over_budget += 1;
        }
        println!(
            "{:<24} {:>4} {:>6} {:>5} {:>8} {:>7} {:>10} {:>5}{}",
            instruction.name,
            profile.declared_writable,
            profile.effective_writable,
            profile.signers,
            lock_cost,
            if saved > 0 {
                saved.to_string()
            } else {
                "-".to_string()
            },
            if profile.provably_read_only > 0 {
                profile.provably_read_only.to_string()
            } else {
                "-".to_string()
            },
            if profile.remaining_accounts_max > 0 {
                profile.remaining_accounts_max.to_string()
            } else {
                "-".to_string()
            },
            if over { "  OVER" } else { "" },
        );
    }

    println!("{}", "-".repeat(80));
    println!(
        "  {} instruction(s); {} with a mutation-complete write set (demotion eligible)",
        manifest.instructions.len(),
        demotion_capable,
    );
    println!(
        "  declared write locks: {total_declared} CU -> {total_effective} CU effective \
         ({total_saved} CU removed by proven-read-only demotion). Per-instruction sums; a \
         transaction charges each unique key once.",
    );
    if instructions_with_provable > 0 {
        println!(
            "  {instructions_with_provable} instruction(s) name accounts the write set PROVES are \
             read-only (up to {max_provable_in_one} in one instruction). Each one a client \
             needlessly marks writable costs {} CU and makes the transaction serialize on it.",
            cost_model::WRITE_LOCK_UNITS,
        );
    } else if demotion_capable == 0 {
        println!(
            "  no account is provably read-only: proving it requires a mutation-complete \
             context (`strict_writes` + `lamports(...)`), which no instruction here declares."
        );
    } else {
        println!(
            "  no account is provably read-only: every account of the {demotion_capable} \
             mutation-complete instruction(s) carries a declared byte range or lamport \
             permission, so none can be proven untouched."
        );
    }
    if remaining_capable > 0 {
        println!(
            "  {remaining_capable} instruction(s) accept caller-supplied remaining accounts (Rem \
             column). Those arrive with caller-chosen flags, so each writable one adds {} CU \
             beyond the figures above — the declaration cannot bound it.",
            cost_model::WRITE_LOCK_UNITS,
        );
    }
    println!(
        "  reference: {} CU per write lock, {} CU per signature; block limit {} CU, \
         per-account cap {} CU",
        cost_model::WRITE_LOCK_UNITS,
        cost_model::SIGNATURE_COST,
        cost_model::MAX_BLOCK_UNITS,
        cost_model::MAX_WRITABLE_ACCOUNT_UNITS,
    );
    println!(
        "  scope: Lock CU is the DECLARATION's share of a transaction's block cost, not the \
         whole of it. Agave also charges the requested compute limit ({} CU by default, usually \
         the largest term), the requested loaded-data limit ({} CU by default), and \
         instruction-data bytes — all caller choices — plus the fee payer's own write lock.",
        cost_model::DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT,
        cost_model::DEFAULT_LOADED_ACCOUNTS_DATA_COST,
    );

    over_budget
}

/// Entry point for `hopper contention <manifest> [--max-block-cost N]`.
pub fn cmd_contention(args: &[String], load: impl FnOnce(&str) -> ProgramManifest) {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        if args.is_empty() {
            process::exit(1);
        }
        return;
    }

    let mut manifest_arg: Option<&String> = None;
    let mut max_block_cost: Option<u64> = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--max-block-cost" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    eprintln!("hopper contention: --max-block-cost requires a CU value");
                    process::exit(1);
                };
                set_ceiling(&mut max_block_cost, value);
            }
            other if other.starts_with("--max-block-cost=") => {
                let value = other.trim_start_matches("--max-block-cost=");
                set_ceiling(&mut max_block_cost, value);
            }
            other if other.starts_with('-') => {
                eprintln!("hopper contention: unknown option {other}");
                print_usage();
                process::exit(1);
            }
            _ => {
                // Refuse a second positional rather than silently letting
                // the last one win: in CI that reads as "the gate ran on
                // the manifest I named", when it ran on a different one.
                if manifest_arg.is_some() {
                    eprintln!(
                        "hopper contention: more than one manifest given ({} and {})",
                        manifest_arg.unwrap(),
                        args[index],
                    );
                    process::exit(1);
                }
                manifest_arg = Some(&args[index]);
            }
        }
        index += 1;
    }

    let Some(manifest_arg) = manifest_arg else {
        eprintln!("hopper contention: a manifest path is required");
        print_usage();
        process::exit(1);
    };

    // Accept a plain filesystem path, not just the shared resolver's
    // `@path` / raw-JSON spellings: this command is meant to run in CI as
    // `hopper contention hopper.manifest.json --max-block-cost N`, and
    // silently reading a bare path as JSON would fail with a parse error
    // that says nothing about the real mistake. Reading it here (rather
    // than changing the shared resolver) leaves every other command's
    // argument contract untouched.
    let path = std::path::Path::new(manifest_arg);
    let resolved = if path.is_file() {
        match std::fs::read_to_string(manifest_arg) {
            Ok(contents) => contents,
            Err(err) => {
                eprintln!("hopper contention: cannot read {manifest_arg}: {err}");
                process::exit(1);
            }
        }
    } else if path.is_dir() {
        eprintln!("hopper contention: {manifest_arg} is a directory, not a manifest");
        process::exit(1);
    } else if !manifest_arg.trim_start().starts_with('{') && !manifest_arg.starts_with('@') {
        // Looks like a path but nothing is there. Say so, instead of
        // handing it to the JSON parser and reporting "Expected JSON
        // object" — an error that describes the parser's confusion rather
        // than the user's actual mistake.
        eprintln!("hopper contention: no such manifest file: {manifest_arg}");
        process::exit(1);
    } else {
        manifest_arg.clone()
    };

    let manifest = load(&resolved);

    // Fail closed when a gate was requested but there is nothing to gate.
    // A manifest that parses to zero instructions is far more often the
    // wrong file, or a schema drift that dropped the key, than a real
    // program with no instructions — and reporting "under budget" for it
    // turns a blind gate into a green one. Informational runs (no
    // ceiling) still just print the empty table.
    if max_block_cost.is_some() && manifest.instructions.is_empty() {
        eprintln!();
        eprintln!(
            "FAIL: the manifest declares no instructions, so --max-block-cost gated nothing. \
             Check the manifest path, or regenerate it with `hopper compile --emit manifest`."
        );
        process::exit(1);
    }

    let over_budget = report(&manifest, max_block_cost);
    if over_budget > 0 {
        eprintln!();
        eprintln!(
            "FAIL: {over_budget} instruction(s) exceed the declared block-cost ceiling. Reduce \
             the writable account count, or declare `lamports(...)` alongside `strict_writes` so \
             provably-untouched accounts can be demoted."
        );
        process::exit(1);
    }
}

/// Parse and store a `--max-block-cost` value, refusing a second one.
/// Silently letting the last flag win reads, in a CI log, as though the
/// gate ran at the ceiling the author meant.
fn set_ceiling(slot: &mut Option<u64>, value: &str) {
    match value.parse::<u64>() {
        Ok(parsed) => {
            if let Some(existing) = *slot {
                eprintln!(
                    "hopper contention: --max-block-cost given twice ({existing} and {parsed})"
                );
                process::exit(1);
            }
            *slot = Some(parsed);
        }
        Err(_) => {
            eprintln!("hopper contention: --max-block-cost expects an integer number of CU");
            process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("Usage: hopper contention <manifest> [--max-block-cost <CU>]");
    eprintln!();
    eprintln!("Reports the write-lock and signature footprint each instruction declares,");
    eprintln!("and how much of it a proven write set removes. Declaration only: this is");
    eprintln!("NOT a transaction's block cost (Agave also charges the requested compute");
    eprintln!("limit, the requested loaded-data limit, instruction bytes, and the fee");
    eprintln!("payer's own lock) and NOT the compute a handler burns.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --max-block-cost <CU>   Fail (exit 1) if any instruction's Lock CU exceeds this");
    eprintln!("                          ceiling. A CI gate on declared lock footprint.");
    eprintln!();
    eprintln!("Columns:");
    eprintln!("  W         accounts declared writable");
    eprintln!("  W-eff     still writable after sound demotion");
    eprintln!("  Sigs      required signers");
    eprintln!("  Lock CU   write locks (after demotion) + signatures");
    eprintln!("  Saved     write-lock CU removed by demotion");
    eprintln!("  Proven RO non-signer accounts the write set proves are never mutated — a");
    eprintln!("            client must send these read-only or waste a lock on each");
    eprintln!("  Rem       ceiling on caller-supplied remaining accounts, whose flags the");
    eprintln!("            declaration cannot bound");
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_schema::{AccountEntry, ContentionProfile, InstructionDescriptor, WriteRange};

    static ACCOUNTS: &[AccountEntry] = &[
        AccountEntry {
            name: "admin",
            writable: false,
            signer: true,
            layout_ref: "",
            seeds: &[],
        },
        AccountEntry {
            name: "config",
            writable: true,
            signer: false,
            layout_ref: "Config",
            seeds: &[],
        },
        AccountEntry {
            name: "untouched",
            writable: true,
            signer: false,
            layout_ref: "",
            seeds: &[],
        },
    ];

    fn descriptor(name: &'static str, mutation_complete: bool) -> InstructionDescriptor {
        InstructionDescriptor {
            name,
            tag: 1,
            discriminator: &[1],
            args: &[],
            accounts: ACCOUNTS,
            remaining_accounts: None,
            capabilities: &[],
            policy_pack: "",
            receipt_expected: false,
            strict_writes: true,
            write_ranges: &[WriteRange {
                account_index: 1,
                offset: 0,
                size: 8,
            }],
            parametric_write_ranges: &[],
            mutation_complete,
            lamport_accounts: &[],
            cu_estimate: 0,
        }
    }

    fn manifest(instructions: &'static [InstructionDescriptor]) -> ProgramManifest {
        ProgramManifest {
            name: "p",
            version: "1.0.0",
            description: "",
            layouts: &[],
            layout_metadata: &[],
            instructions,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[],
            contexts: &[],
        }
    }

    #[test]
    fn a_ceiling_that_fits_reports_no_failures() {
        static IX: &[InstructionDescriptor] = &[];
        let mut rows = manifest(IX);
        // 2 writable, 1 demotable -> 1 effective lock (300) + 1 sig (720).
        let owned = [descriptor("pause", true)];
        let leaked: &'static [InstructionDescriptor] = Box::leak(Box::new(owned));
        rows.instructions = leaked;
        assert_eq!(report(&rows, Some(1_020)), 0);
    }

    #[test]
    fn a_ceiling_below_the_declared_cost_counts_the_offender() {
        let owned = [descriptor("pause", true)];
        let leaked: &'static [InstructionDescriptor] = Box::leak(Box::new(owned));
        let mut rows = manifest(&[]);
        rows.instructions = leaked;
        assert_eq!(report(&rows, Some(1_019)), 1);
    }

    #[test]
    fn an_incomplete_write_set_pays_for_every_declared_lock() {
        // Same accounts, no lamport dimension: nothing may be demoted, so
        // both write locks are charged (600) plus the signature (720).
        let owned = [descriptor("pause", false)];
        let leaked: &'static [InstructionDescriptor] = Box::leak(Box::new(owned));
        let mut rows = manifest(&[]);
        rows.instructions = leaked;
        assert_eq!(report(&rows, Some(1_319)), 1, "1320 CU exceeds a 1319 gate");
        assert_eq!(report(&rows, Some(1_320)), 0);
    }

    /// An empty instruction list must not report "under budget": `report`
    /// itself counts zero offenders (correct — there is nothing to
    /// exceed), so the fail-closed decision belongs to the gate path in
    /// `cmd_contention`, which refuses it. This pins `report`'s half of
    /// that contract so the two cannot drift into both being permissive.
    #[test]
    fn an_empty_manifest_has_no_offenders_to_report() {
        let rows = manifest(&[]);
        assert_eq!(report(&rows, Some(0)), 0);
        assert_eq!(report(&rows, None), 0);
    }

    #[test]
    fn no_ceiling_never_fails() {
        let owned = [descriptor("pause", false)];
        let leaked: &'static [InstructionDescriptor] = Box::leak(Box::new(owned));
        let mut rows = manifest(&[]);
        rows.instructions = leaked;
        assert_eq!(report(&rows, None), 0);
    }

    #[test]
    fn provably_read_only_counts_non_signers_the_write_set_clears() {
        // Fixture: admin (signer, ro), config (writable + data range),
        // untouched (writable, no range, no lamport permission).
        let complete = descriptor("pause", true).contention_profile();
        // Only `untouched` qualifies: `config` carries a range, and `admin`
        // is a signer (the fee payer must stay writable at the transaction
        // level, so advising read-only there would break transactions).
        assert_eq!(complete.provably_read_only, 1);
        // The waste is one flat lock per over-marked account — not that
        // count multiplied by anything.
        assert_eq!(
            ContentionProfile::cost_per_over_marked_account(),
            cost_model::WRITE_LOCK_UNITS
        );

        // Without the lamport dimension nothing is provable at all.
        let incomplete = descriptor("pause", false).contention_profile();
        assert_eq!(incomplete.provably_read_only, 0);
    }

    #[test]
    fn the_profile_matches_the_reported_arithmetic() {
        let complete = descriptor("pause", true).contention_profile();
        assert_eq!(complete.declared_writable, 2);
        assert_eq!(complete.effective_writable, 1, "`untouched` is demoted");
        assert_eq!(complete.write_lock_cost_saved(), 300);
        assert_eq!(complete.declared_lock_and_signature_cost(), 300 + 720);

        let incomplete = descriptor("pause", false).contention_profile();
        assert_eq!(incomplete.effective_writable, 2);
        assert_eq!(incomplete.write_lock_cost_saved(), 0);
        assert_eq!(incomplete.declared_lock_and_signature_cost(), 600 + 720);
    }

    /// A caller-supplied remaining-accounts ceiling is surfaced, not folded
    /// into the gated figure: the declaration fixes the latter and cannot
    /// bound the former.
    #[test]
    fn remaining_accounts_ceiling_is_reported_but_not_gated() {
        let mut ix = descriptor("settle", true);
        ix.remaining_accounts = Some(hopper_schema::RemainingAccountsDescriptor { max: 20 });
        let profile = ix.contention_profile();
        assert_eq!(profile.remaining_accounts_max, 20);
        assert_eq!(profile.remaining_accounts_worst_case_lock_cost(), 20 * 300);
        // Unchanged by the ceiling: still one effective lock + one signature.
        assert_eq!(profile.declared_lock_and_signature_cost(), 300 + 720);
    }
}
