//! Manifest Phase B: `hopper::program_manifest!` exports a full
//! `PROGRAM_MANIFEST` with near-zero authoring.
//!
//! The author writes ONE short block naming the program module and the
//! layout/event types; everything deep — instruction rows, const-eval
//! account conversion, layout field tables, event descriptors — comes
//! from the same macro-generated consts the runtime enforces. These
//! tests bind a real `#[hopper::program]` + `#[derive(Accounts)]` +
//! `#[account]` + `#[hopper::event]` set, then assert the aggregated
//! manifest (and its `ManifestJson` rendering — the exact surface
//! `hopper compile --emit manifest` writes and `hopper tx explain`
//! decodes against) carries the enforced truth.

#![cfg(feature = "proc-macros")]

use hopper::hopper_schema::codama::ManifestJson;
use hopper::hopper_schema::{FieldIntent, SchemaExport};
use hopper::prelude::*;

// ── Program under test ─────────────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 3, version = 1)]
pub struct Ledger {
    pub authority: Address,
    #[role = "balance"]
    pub balance: WireU64,
}

#[derive(Accounts)]
pub struct Transfer<'info> {
    #[account(mut, has_one = authority)]
    pub ledger: Account<'info, Ledger>,
    pub authority: Signer<'info>,
}

#[hopper::event(tag = 7)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Credited {
    pub amount: WireU64,
    pub total: WireU32,
}

// The handlers are aggregation fixtures — nothing dispatches them here
// (the dispatch paths have their own integration suites).
#[allow(dead_code)]
#[hopper::program(entrypoint = false)]
mod manifest_prog {
    use super::*;

    #[instruction(0)]
    #[allow(unused_mut, unused_variables)]
    pub fn transfer(ctx: Ctx<Transfer>, amount: u64) -> ProgramResult {
        Ok(())
    }

    /// Raw handler: opaque to the schema layer, must publish no row.
    #[instruction(1)]
    #[allow(unused_variables)]
    pub fn poke(ctx: &mut Context<'_>) -> ProgramResult {
        Ok(())
    }
}

// The whole authoring surface. Name/version default to CARGO_PKG_*.
hopper::program_manifest! {
    program = manifest_prog,
    description = "manifest phase B integration fixture",
    layouts = [Ledger],
    events = [Credited],
}

/// A second invocation proving every optional key overrides (and that
/// the layouts/events lists may be omitted entirely).
mod overrides {
    use super::manifest_prog;

    hopper::program_manifest! {
        program = manifest_prog,
        name = "custom-name",
        version = "9.9.9",
        description = "custom description",
    }
}

// ── Aggregation ────────────────────────────────────────────────────

#[test]
fn name_version_default_to_cargo_pkg_and_override_cleanly() {
    assert_eq!(PROGRAM_MANIFEST.name, env!("CARGO_PKG_NAME"));
    assert_eq!(PROGRAM_MANIFEST.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        PROGRAM_MANIFEST.description,
        "manifest phase B integration fixture"
    );

    assert_eq!(overrides::PROGRAM_MANIFEST.name, "custom-name");
    assert_eq!(overrides::PROGRAM_MANIFEST.version, "9.9.9");
    assert_eq!(
        overrides::PROGRAM_MANIFEST.description,
        "custom description"
    );
    assert!(overrides::PROGRAM_MANIFEST.layouts.is_empty());
    assert!(overrides::PROGRAM_MANIFEST.events.is_empty());
}

#[test]
fn typed_handlers_publish_rows_raw_handlers_stay_opaque() {
    // `transfer` (typed) publishes; `poke` (raw) is opaque.
    assert_eq!(PROGRAM_MANIFEST.instructions.len(), 1);
    let ix = &PROGRAM_MANIFEST.instructions[0];
    assert_eq!(ix.name, "transfer");
    assert_eq!(ix.tag, 0);

    // Typed args: real name, real canonical type, table wire size.
    assert_eq!(ix.args.len(), 1);
    assert_eq!(ix.args[0].name, "amount");
    assert_eq!(ix.args[0].canonical_type, "u64");
    assert_eq!(ix.args[0].size, 8);

    // Never-fabricated columns.
    assert!(ix.capabilities.is_empty());
    assert_eq!(ix.policy_pack, "");
    assert!(!ix.receipt_expected, "no #[receipt] on the handler");
    assert_eq!(ix.cu_estimate, 0);

    // Write-surface columns are the spec's own generated consts.
    assert_eq!(ix.strict_writes, Transfer::STRICT_WRITES);
    assert_eq!(ix.write_ranges, Transfer::WRITE_RANGES);
    assert_eq!(ix.mutation_complete, Transfer::MUTATION_COMPLETE);
    assert_eq!(ix.lamport_accounts, Transfer::LAMPORT_ACCOUNTS);
}

#[test]
fn account_rows_are_converted_verbatim_from_schema_metadata() {
    let ix = &PROGRAM_MANIFEST.instructions[0];
    let described = Transfer::SCHEMA_METADATA.accounts;
    assert_eq!(ix.accounts.len(), described.len());
    for (entry, descriptor) in ix.accounts.iter().zip(described) {
        assert_eq!(entry.name, descriptor.name);
        assert_eq!(entry.writable, descriptor.writable);
        assert_eq!(entry.signer, descriptor.signer);
        assert_eq!(entry.layout_ref, descriptor.layout_ref);
        assert!(entry.seeds.is_empty(), "seeds stay empty in this pass");
    }

    // The concrete shape, pinned: ledger is the writable Ledger-typed
    // slot, authority the read-only signer.
    assert_eq!(ix.accounts[0].name, "ledger");
    assert!(ix.accounts[0].writable);
    assert!(!ix.accounts[0].signer);
    assert_eq!(ix.accounts[0].layout_ref, "Ledger");
    assert_eq!(ix.accounts[1].name, "authority");
    assert!(!ix.accounts[1].writable);
}

/// A TYPE-LEVEL `Signer<'info>` wrapper must publish `signer: true` —
/// `validate()` enforces it (`expect_signer_writable`), so the manifest
/// claiming otherwise would under-report the account requirements.
/// (Regression: SCHEMA_METADATA previously only honored the attribute
/// spelling `#[signer]` / `#[account(signer)]`.)
#[test]
fn wrapper_signers_publish_signer_true() {
    let authority = Transfer::SCHEMA_METADATA
        .find_account("authority")
        .expect("authority described");
    assert!(authority.signer, "Signer<'info> wrapper => signer: true");

    let ix = &PROGRAM_MANIFEST.instructions[0];
    assert!(ix.accounts[1].signer, "conversion carries the signer bit");
}

#[test]
fn context_descriptors_are_the_specs_schema_metadata() {
    assert_eq!(PROGRAM_MANIFEST.contexts.len(), 1);
    let ctx = &PROGRAM_MANIFEST.contexts[0];
    assert_eq!(ctx.name, "Transfer");
    assert_eq!(ctx.account_count(), 2);
    assert_eq!(ctx.signer_count(), 1);
}

// ── Layouts ────────────────────────────────────────────────────────

#[test]
fn layout_manifest_carries_real_types_offsets_and_declared_intents() {
    assert_eq!(PROGRAM_MANIFEST.layouts.len(), 1);
    let layout = &PROGRAM_MANIFEST.layouts[0];
    assert_eq!(layout.name, "Ledger");
    assert_eq!(layout.disc, Ledger::DISC);
    assert_eq!(layout.version, Ledger::VERSION);
    assert_eq!(layout.layout_id, Ledger::LAYOUT_ID);
    assert_eq!(layout.total_size, Ledger::LEN);
    assert_eq!(layout.field_count, 2);

    // Header-relative offsets over the REAL authored types.
    let authority = &layout.fields[0];
    assert_eq!(authority.name, "authority");
    assert_eq!(authority.canonical_type, "Address");
    assert_eq!(authority.offset, 16);
    assert_eq!(authority.size, 32);
    assert_eq!(authority.intent, FieldIntent::Custom);

    let balance = &layout.fields[1];
    assert_eq!(balance.name, "balance");
    assert_eq!(balance.canonical_type, "WireU64");
    assert_eq!(balance.offset, 48);
    assert_eq!(balance.size, 8);
    // Declared via `#[role = "balance"]` — the macro publishes what the
    // author declared, never a guess.
    assert_eq!(balance.intent, FieldIntent::Balance);
}

#[test]
fn schema_export_and_layout_manifest_agree() {
    let via_trait = <Ledger as SchemaExport>::layout_manifest();
    let via_const = Ledger::LAYOUT_MANIFEST;
    assert_eq!(via_trait.layout_id, via_const.layout_id);
    assert_eq!(via_trait.total_size, via_const.total_size);
    assert_eq!(via_trait.field_count, via_const.field_count);
    for (a, b) in via_trait.fields.iter().zip(via_const.fields) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.canonical_type, b.canonical_type);
        assert_eq!(a.offset, b.offset);
        assert_eq!(a.size, b.size);
        assert_eq!(a.intent, b.intent);
    }
}

// ── Events ─────────────────────────────────────────────────────────

#[test]
fn event_descriptor_publishes_tag_and_payload_relative_fields() {
    assert_eq!(PROGRAM_MANIFEST.events.len(), 1);
    let event = &PROGRAM_MANIFEST.events[0];
    assert_eq!(event.name, "Credited");
    assert_eq!(event.tag, Credited::EVENT_TAG);
    assert_eq!(event.tag, 7);

    // Payload-relative offsets: field 0 starts at 0 (the tag byte is
    // prepended by the emitters, not part of `as_bytes`).
    assert_eq!(event.fields.len(), 2);
    assert_eq!(event.fields[0].name, "amount");
    assert_eq!(event.fields[0].canonical_type, "WireU64");
    assert_eq!(event.fields[0].offset, 0);
    assert_eq!(event.fields[0].size, 8);
    assert_eq!(event.fields[1].name, "total");
    assert_eq!(event.fields[1].canonical_type, "WireU32");
    assert_eq!(event.fields[1].offset, 8);
    assert_eq!(event.fields[1].size, 4);
}

// ── JSON rendering (the `--emit manifest` / `tx explain` surface) ──

#[test]
fn manifest_json_renders_the_decode_surfaces() {
    let json = format!("{}", ManifestJson(&PROGRAM_MANIFEST));

    // Layout fields resolve write offsets (tx explain's touch-map join).
    assert!(json.contains(
        "{ \"name\": \"balance\", \"type\": \"WireU64\", \"size\": 8, \"offset\": 48, \"intent\": \"balance\" }"
    ));
    // Event fields resolve payload slices (tx explain's event join).
    assert!(json.contains("\"tag\": 7,"));
    assert!(json.contains(
        "{ \"name\": \"amount\", \"type\": \"WireU64\", \"size\": 8, \"offset\": 0, \"intent\": \"custom\" }"
    ));
    // Instruction row with the converted accounts.
    assert!(json.contains("\"name\": \"transfer\""));
    assert!(json.contains(
        "{ \"name\": \"ledger\", \"writable\": true, \"signer\": false, \"layoutRef\": \"Ledger\" }"
    ));
    assert!(json.contains("{ \"name\": \"authority\", \"writable\": false, \"signer\": true }"));
}

/// The checked-in hopper-smoke manifest (generated by
/// `hopper compile --emit manifest --package hopper-smoke` from the
/// one-liner export) carries exactly the surfaces `hopper tx explain`
/// joins against: `events[].tag` + field spans for self-CPI event
/// decoding (`render_event_fields`), and `layouts[].fields` offsets for
/// touch-map write attribution (`field_write_targets`).
#[test]
fn checked_in_smoke_manifest_carries_tx_explain_decode_surfaces() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/hopper-smoke/hopper.manifest.json"
    );
    let json = std::fs::read_to_string(path)
        .expect("examples/hopper-smoke/hopper.manifest.json is checked in")
        .replace("\r\n", "\n");

    // events[]: DepositReceipt rides tag 2 with balance/deposit_count —
    // the exact join key + spans the self-CPI event decoder slices.
    assert!(json.contains("\"name\": \"DepositReceipt\",\n      \"tag\": 2,"));
    assert!(json.contains(
        "{ \"name\": \"balance\", \"type\": \"WireU64\", \"size\": 8, \"offset\": 0, \"intent\": \"custom\" }"
    ));
    assert!(json.contains(
        "{ \"name\": \"deposit_count\", \"type\": \"WireU32\", \"size\": 4, \"offset\": 8, \"intent\": \"custom\" }"
    ));
    // DepositEvent rides tag 1 — the same byte `emit_event_tagged(1, ..)`
    // writes on the wire.
    assert!(json.contains("\"name\": \"DepositEvent\",\n      \"tag\": 1,"));

    // layouts[]: Vault.balance at account-absolute offset 48, size 8 —
    // the span touch-map write attribution resolves against. Pin it to
    // the Vault block (it must appear after Vault opens and before the
    // next layout begins).
    let vault = json.find("\"name\": \"Vault\"").expect("Vault layout");
    let next_layout = json.find("\"name\": \"NoteV1\"").expect("NoteV1 layout");
    let balance_at_48 = json
        .find("{ \"name\": \"balance\", \"type\": \"WireU64\", \"size\": 8, \"offset\": 48, \"intent\": \"custom\" }")
        .expect("Vault.balance field row");
    assert!(
        vault < balance_at_48 && balance_at_48 < next_layout,
        "balance@48 must be published as a Vault field"
    );

    // And the withdraw instruction publishes the ENFORCED 8-byte balance
    // write range over that same span.
    assert!(json.contains("\"name\": \"withdraw\""));
    assert!(json
        .contains("{ \"account\": \"vault\", \"accountIndex\": 1, \"offset\": 48, \"size\": 8 }"));
}
