//! Minimal macro-first Hopper counter.
//!
//! This is the first-contact example: account, context, handler, done.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[derive(Accounts)]
pub struct Increment<'info> {
    #[account(mut, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

#[program(profile = "tiny")]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        let mut counter = ctx.accounts.counter.get_mut()?;
        counter.value.checked_add_assign(1)?;
        Ok(())
    }
}

// --- Program manifest (exported schema) ------------------------------
//
// `hopper compile --emit manifest --package hopper-counter` runs a tiny
// generated harness that prints `ManifestJson(&PROGRAM_MANIFEST)` and
// writes `hopper.manifest.json` — so the published schema is rendered
// from the SAME consts the runtime enforces (`Increment::SCHEMA_METADATA`,
// `STRICT_WRITES`, `WRITE_RANGES`, ...) and cannot drift from the code.
// Today the aggregation below is written out once by hand; the
// `#[program]` macro is slated to emit it automatically, at which point
// this block deletes.

/// Counter's layout manifest, built from the same contract consts the
/// runtime checks (offsets are header-relative: 16-byte header first).
pub static COUNTER_LAYOUT: hopper::hopper_schema::LayoutManifest =
    hopper::hopper_schema::LayoutManifest {
        name: "Counter",
        version: Counter::VERSION,
        disc: Counter::DISC,
        layout_id: Counter::LAYOUT_ID,
        total_size: Counter::LEN,
        field_count: 2,
        fields: &[
            hopper::hopper_schema::FieldDescriptor {
                name: "authority",
                canonical_type: "Address",
                size: 32,
                offset: 16,
                intent: hopper::hopper_schema::FieldIntent::Authority,
            },
            hopper::hopper_schema::FieldDescriptor {
                name: "value",
                canonical_type: "WireU64",
                size: 8,
                offset: 48,
                intent: hopper::hopper_schema::FieldIntent::Counter,
            },
        ],
    };

/// The program-wide schema manifest `hopper compile --emit manifest`
/// exports. Instruction rows reference the macro-generated context
/// consts, so the published write-surface claims are the enforced ones.
pub static PROGRAM_MANIFEST: hopper::hopper_schema::ProgramManifest =
    hopper::hopper_schema::ProgramManifest {
        name: "hopper-counter",
        version: "0.2.1",
        description: "Minimal macro-first Hopper counter.",
        layouts: &[COUNTER_LAYOUT],
        layout_metadata: &[],
        instructions: &[hopper::hopper_schema::InstructionDescriptor {
            name: "increment",
            tag: 0,
            args: &[],
            accounts: &[
                hopper::hopper_schema::AccountEntry {
                    name: "counter",
                    writable: true,
                    signer: false,
                    layout_ref: "Counter",
                    seeds: &[],
                },
                hopper::hopper_schema::AccountEntry {
                    name: "authority",
                    writable: false,
                    signer: true,
                    layout_ref: "",
                    seeds: &[],
                },
            ],
            capabilities: &[],
            policy_pack: "",
            receipt_expected: false,
            strict_writes: Increment::STRICT_WRITES,
            write_ranges: Increment::WRITE_RANGES,
            mutation_complete: Increment::MUTATION_COMPLETE,
            lamport_accounts: Increment::LAMPORT_ACCOUNTS,
            cu_estimate: 0,
        }],
        events: &[],
        policies: &[],
        compatibility_pairs: &[],
        tooling_hints: &[],
        contexts: &[Increment::SCHEMA_METADATA],
    };
