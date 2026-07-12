//! Composite v2: the container's options compose across the nesting
//! boundary — end to end, through the hopper-svm host harness.
//!
//! Composite v1 refused `strict_writes` / `lamports(...)` / `event_cpi` /
//! `emit_touch_map` / `auto_lifecycle` on any context embedding a
//! `#[composite]` field. v2 composes them instead; these tests prove the
//! three flagship combinations behave, not just expand:
//!
//! 1. **strict_writes**: the outer's authority set is compile-time
//!    COMPOSED — the inner context's declared `mut(balance)` segment is
//!    spliced at its FLATTENED account index, so the inner lease is
//!    enforceable from the outer gate exactly as it would be standalone:
//!    the declared segment write succeeds, an out-of-range write through
//!    the outer context fails with the policy error
//!    (`Custom(0xD000 | flattened_index)`).
//! 2. **event_cpi**: the two synthetic trailing slots append AFTER the
//!    flattened set, bind validates them there, and the emit one-liner
//!    self-invokes with the captured bump (host CPI capture asserts the
//!    exact wire bytes). A control composite context keeps the
//!    pre-feature shape.
//! 3. **emit_touch_map**: the const is advertised, the dispatcher's
//!    Ok-path emit runs against a composite container, and the touch
//!    map's records land at the correct FLATTENED slots (record-level
//!    assertions run under the `touch-map` feature — enabled in the
//!    workspace test lane via hopper-smoke's feature unification).
//!
//! The INNER context stays a plain validation context throughout: the
//! v2 lift is about the OUTER's options; inner-side embeddability rules
//! are unchanged (and pinned by expansion tests).

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_runtime::write_policy::WriteRange;
use hopper_svm::{AccountFixture, HopperSvm};

// ── State under test ────────────────────────────────────────────────
//
// Two fields, so the declared `balance` segment is a strict subset of
// the body and the undeclared `nonce` sibling proves refusal.

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 73, version = 1)]
pub struct Vault {
    pub balance: WireU64,
    pub nonce: WireU64,
}

// ── strict_writes across the boundary ──────────────────────────────

/// The INNER (nested) context: a plain validation context, embeddable.
/// Its vault is writable ONLY in the `balance` segment — the exact
/// structure the outer's composed policy must preserve.
#[derive(hopper::Accounts)]
pub struct VaultCheck<'info> {
    pub authority: Signer<'info>,

    #[account(mut(balance))]
    pub vault: Account<'info, Vault>,
}

/// The OUTER container opts into `strict_writes` (refused outright in
/// v1). Flattened slot order:
///
/// `[0] payer · [1] check.authority · [2] check.vault · [3] tail`
#[derive(hopper::Accounts)]
#[accounts(strict_writes)]
pub struct OperateStrict<'info> {
    pub payer: Signer<'info>,

    #[composite]
    pub check: VaultCheck<'info>,

    #[account(mut)]
    pub tail: Account<'info, Vault>,
}

const PROGRAM_ID: Address = Address::new_from_array([11u8; 32]);

fn vault_fixture(addr_byte: u8) -> AccountFixture {
    let mut data = vec![0u8; Vault::LEN];
    write_header(&mut data, Vault::DISC, Vault::VERSION, &Vault::LAYOUT_ID).unwrap();
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
    .writable()
}

fn signer_fixture(addr_byte: u8) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer()
}

fn balance_of(fixture: &AccountFixture) -> u64 {
    let off = HEADER_LEN + Vault::BALANCE_OFFSET as usize;
    u64::from_le_bytes(fixture.data[off..off + 8].try_into().unwrap())
}

/// Declared == published == composed: the inner's declared const carries
/// its LOCAL structure; the outer's authority set carries the SAME
/// ranges rebased to the flattened indices, plus the outer's own leaf
/// grant — and the schema publishes the identical slice.
#[test]
fn composed_write_ranges_splice_the_inner_segment_rebased() {
    // The inner context publishes its declared structure at LOCAL
    // indices (vault is inner slot 1), independent of strict_writes.
    assert_eq!(
        VaultCheck::__HOPPER_DECLARED_WRITE_RANGES,
        &[WriteRange::new(
            1,
            HEADER_LEN as u32 + Vault::BALANCE_OFFSET,
            core::mem::size_of::<WireU64>() as u32,
        )],
        "the inner declared const must carry the local mut(balance) range"
    );
    // The inner's own authority stays empty: declared structure confers
    // no enforcement without the (outer's) strict_writes opt-in.
    assert!(!VaultCheck::STRICT_WRITES);
    assert!(VaultCheck::WRITE_RANGES.is_empty());

    // The outer's composed set: inner spliced at flattened slot 2, the
    // post-composite leaf at flattened slot 3.
    assert!(OperateStrict::STRICT_WRITES);
    assert_eq!(
        OperateStrict::WRITE_RANGES,
        &[
            WriteRange::new(
                2,
                HEADER_LEN as u32 + Vault::BALANCE_OFFSET,
                core::mem::size_of::<WireU64>() as u32,
            ),
            WriteRange::whole_account(3),
        ],
        "the composed authority set must rebase the inner range to slot 2"
    );
    // Published == enforced: schema reads the same composed const.
    assert_eq!(
        OperateStrict::SCHEMA_METADATA.write_ranges,
        OperateStrict::WRITE_RANGES
    );
    // Composite containers still refuse embedding (inner rules did not
    // loosen with the v2 lift).
    assert!(VaultCheck::__HOPPER_EMBEDDABLE);
    assert!(!OperateStrict::__HOPPER_EMBEDDABLE);
}

fn strict_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    {
        let mut bound = OperateStrict::bind(&mut ctx)?;

        // The inner-declared `mut(balance)` lease works through the
        // inner bound view at its flattened slot.
        {
            let mut check = bound.check()?;
            let mut balance = check.vault_balance_mut()?;
            *balance = WireU64::new(111);
        }

        // The outer's own whole-account grant works too.
        bound.tail_load_mut()?.balance.checked_add_assign(222)?;
    }

    // Out-of-range write THROUGH THE OUTER context: the undeclared
    // `nonce` sibling on the inner vault (flattened slot 2) is refused
    // at acquisition with the indexed policy error.
    let nonce_off = HEADER_LEN as u32 + Vault::NONCE_OFFSET;
    assert_eq!(
        ctx.segment_mut::<[u8; 8]>(2, nonce_off).err(),
        Some(ProgramError::Custom(0xD000 | 2)),
        "an undeclared range on the inner slot must fail with 0xD000|2"
    );
    // While the declared balance range on the same slot stays writable.
    assert!(ctx
        .segment_mut::<[u8; 8]>(2, HEADER_LEN as u32 + Vault::BALANCE_OFFSET)
        .is_ok());
    // And the installed policy is literally the published composed set.
    let policy = ctx
        .write_policy()
        .expect("strict_writes bind() must install a WritePolicy");
    assert_eq!(policy.allows, OperateStrict::WRITE_RANGES);
    Ok(())
}

fn strict_handler_entry<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    strict_handler(program_id, accounts, instruction_data)
}

/// Behavioral proof: the composed policy enforces the inner's declared
/// structure from the outer gate — declared segment write lands, the
/// undeclared sibling range is refused with `Custom(0xD000 | 2)`.
#[test]
fn inner_segment_lease_enforceable_from_the_outer_gate() {
    let accounts = [
        signer_fixture(0x21), // [0] payer
        signer_fixture(0x22), // [1] check.authority
        vault_fixture(0x23),  // [2] check.vault
        vault_fixture(0x24),  // [3] tail
    ];
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, strict_handler_entry);
    assert!(
        result.program_result.is_ok(),
        "the strict composite handler must succeed: {:?}",
        result.program_result
    );
    assert_eq!(
        balance_of(&result.resulting_accounts[2]),
        111,
        "the inner-declared segment write must persist at flattened slot 2"
    );
    assert_eq!(
        balance_of(&result.resulting_accounts[3]),
        222,
        "the outer's own whole-account write must persist at slot 3"
    );
}

// ── event_cpi across the boundary ───────────────────────────────────
//
// Lifetime-free `#[hopper::context]` shapes so a real
// `#[hopper::program]` dispatcher drives them (the same conventions as
// the event_cpi dispatch suite).

/// Inner context for the dispatcher-driven programs: plain validation,
/// embeddable, `mut(balance)` on the vault.
#[hopper::context]
pub struct BalanceCheck {
    #[signer]
    pub authority: AccountView<'static>,

    #[account(mut(balance))]
    pub vault: Vault,
}

/// The emitted event: 8 payload bytes.
#[hopper::event(tag = 0x51)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Moved {
    pub amount: WireU64,
}

/// Composite container + `event_cpi` (refused outright in v1): the two
/// synthetic slots trail the FLATTENED set.
///
/// `[0] payer · [1] check.authority · [2] check.vault · [3] event_authority · [4] event_program`
#[hopper::context(event_cpi)]
pub struct OperateEvents {
    #[signer]
    pub payer: AccountView<'static>,

    #[composite]
    pub check: BalanceCheck,
}

/// Control: same composite shape without the option — pre-feature
/// account shape, no emit surface.
#[hopper::context]
pub struct OperatePlain {
    #[signer]
    pub payer: AccountView<'static>,

    #[composite]
    pub check: BalanceCheck,
}

#[hopper::program(entrypoint = false)]
mod composite_event_prog {
    use super::*;

    /// State change through the composite at its flattened slot, then
    /// the emit one-liner.
    #[instruction(0)]
    fn do_move(ctx: Context<OperateEvents>, amount: u64) -> ProgramResult {
        {
            let mut check = ctx.check()?;
            let mut balance = check.vault_balance_mut()?;
            *balance = WireU64::new(amount);
        }
        ctx.emit_event_cpi(&Moved {
            amount: WireU64::new(amount),
        })?;
        Ok(())
    }
}

const AUTHORITY_ADDR: [u8; 32] = [7u8; 32];

/// The event-authority fixture: off-chain there is no sha256 syscall to
/// pin the PDA address, and the host CPI emulation checks the signer
/// dimension — same convention as the event_cpi dispatch suite.
fn authority_fixture(signer: bool) -> AccountFixture {
    let fixture = AccountFixture::new(
        Address::new_from_array(AUTHORITY_ADDR),
        Address::new_from_array([0u8; 32]),
        0,
        0,
    );
    if signer {
        fixture.signer()
    } else {
        fixture
    }
}

fn program_fixture() -> AccountFixture {
    AccountFixture::new(PROGRAM_ID, Address::new_from_array([0u8; 32]), 0, 0).executable()
}

fn drive_event<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    composite_event_prog::process_instruction(&mut ctx)
}

/// `[disc = 0, amount u64 LE]`.
fn move_data(amount: u64) -> [u8; 9] {
    let mut data = [0u8; 9];
    data[1..].copy_from_slice(&amount.to_le_bytes());
    data
}

/// The synthetic slots extend the FLATTENED total and the control keeps
/// the pre-feature shape.
#[test]
fn event_cpi_appends_after_the_flattened_set_and_control_is_unchanged() {
    assert_eq!(BalanceCheck::ACCOUNT_COUNT, 2);
    assert_eq!(
        OperateEvents::ACCOUNT_COUNT,
        5,
        "1 payer + 2 flattened inner slots + 2 synthetic trailing slots"
    );
    assert_eq!(
        OperatePlain::ACCOUNT_COUNT,
        3,
        "the control composite context keeps the pre-feature shape"
    );
    assert!(OperateEvents::EVENT_CPI);
    assert!(!OperatePlain::EVENT_CPI);
    assert!(composite_event_prog::__HOPPER_EVENT_CPI_ENABLED);

    // The schema splice covers every flattened slot AND the synthetics,
    // in slot order, length-pinned to ACCOUNT_COUNT.
    let accounts = OperateEvents::SCHEMA_METADATA.accounts;
    assert_eq!(accounts.len(), OperateEvents::ACCOUNT_COUNT);
    let names: std::vec::Vec<&str> = accounts.iter().map(|a| a.name).collect();
    assert_eq!(
        names,
        [
            "payer",
            "authority",
            "vault",
            "event_authority",
            "event_program"
        ],
        "the schema must splice the inner slots and keep the synthetics trailing"
    );
}

/// The decisive end-to-end: dispatch through the generated dispatcher,
/// write through the composite at its flattened slot, emit, and capture
/// the exact self-CPI wire bytes.
#[test]
fn composite_event_emit_produces_the_exact_wire_bytes() {
    use hopper::hopper_runtime::cpi_event::{
        decode_event_cpi, encode_event_cpi, take_host_captured_event_cpis, CPI_EVENT_MARKER,
    };

    let _ = take_host_captured_event_cpis();
    let amount: u64 = 4_242;

    let result = HopperSvm::new().process_instruction(
        PROGRAM_ID,
        &move_data(amount),
        &[
            signer_fixture(0x31),    // [0] payer
            signer_fixture(0x32),    // [1] check.authority
            vault_fixture(0x33),     // [2] check.vault
            authority_fixture(true), // [3] event_authority (synthetic)
            program_fixture(),       // [4] event_program (synthetic)
        ],
        drive_event,
    );
    assert_eq!(
        result.program_result,
        Ok(()),
        "the composite event instruction must succeed"
    );

    // The composite write landed at the FLATTENED slot 2.
    assert_eq!(
        balance_of(&result.resulting_accounts[2]),
        amount,
        "the inner segment write must persist at flattened slot 2"
    );

    // Exactly one self-CPI, signed by the trailing authority slot,
    // carrying the exact encoder bytes.
    let captured = take_host_captured_event_cpis();
    assert_eq!(captured.len(), 1, "exactly one event must be emitted");
    let event = &captured[0];
    assert_eq!(event.program_id, PROGRAM_ID, "self-CPI must target self");
    assert_eq!(
        event.authority,
        Address::new_from_array(AUTHORITY_ADDR),
        "the appended trailing authority slot must sign the CPI"
    );
    let expected_event = Moved {
        amount: WireU64::new(amount),
    };
    let mut expected = [0u8; 3 + core::mem::size_of::<Moved>()];
    let expected_len =
        encode_event_cpi(Moved::EVENT_TAG, expected_event.as_bytes(), &mut expected).unwrap();
    assert_eq!(
        event.data,
        &expected[..expected_len],
        "emitted bytes must round-trip the public encoder byte-for-byte"
    );
    assert_eq!(&event.data[..2], &CPI_EVENT_MARKER, "marker first");
    let (tag, payload) = decode_event_cpi(&event.data).expect("decodable");
    assert_eq!(tag, Moved::EVENT_TAG);
    assert_eq!(payload, expected_event.as_bytes());
}

/// Binding without the two trailing synthetic slots must fail: they are
/// real required accounts appended after the flattened set.
#[test]
fn composite_event_context_requires_the_trailing_slots() {
    let result = HopperSvm::new().process_instruction(
        PROGRAM_ID,
        &move_data(1),
        &[
            signer_fixture(0x31),
            signer_fixture(0x32),
            vault_fixture(0x33),
        ],
        drive_event,
    );
    assert!(
        result.program_result.is_err(),
        "the flattened-only account shape must fail require_accounts"
    );
}

// ── emit_touch_map across the boundary ──────────────────────────────

/// Composite container + `emit_touch_map` (refused outright in v1).
///
/// `[0] payer · [1] check.authority · [2] check.vault · [3] tail`
#[hopper::context(emit_touch_map)]
pub struct OperateTraced {
    #[signer]
    pub payer: AccountView<'static>,

    #[composite]
    pub check: BalanceCheck,

    #[account(mut)]
    pub tail: Vault,
}

#[hopper::program(entrypoint = false)]
mod composite_touch_prog {
    use super::*;

    /// Ok path: touch the inner segment (flattened slot 2) and the
    /// trailing whole account (slot 3), then succeed — the dispatcher
    /// emits the touch map after this returns.
    #[instruction(0)]
    fn trace_ok(ctx: Context<OperateTraced>) -> ProgramResult {
        {
            let mut check = ctx.check()?;
            let mut balance = check.vault_balance_mut()?;
            *balance = WireU64::new(9);
        }
        let _tail = ctx.tail_load_mut()?;
        Ok(())
    }

    /// Err path: same touches, then fail — the dispatcher must emit
    /// nothing (Ok-only contract, unchanged by composition).
    #[instruction(1)]
    fn trace_fail(ctx: Context<OperateTraced>) -> ProgramResult {
        {
            let mut check = ctx.check()?;
            let mut balance = check.vault_balance_mut()?;
            *balance = WireU64::new(9);
        }
        Err(ProgramError::Custom(9))
    }
}

fn drive_traced<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    composite_touch_prog::process_instruction(&mut ctx)
}

fn traced_accounts() -> [AccountFixture; 4] {
    [
        signer_fixture(0x41), // [0] payer
        signer_fixture(0x42), // [1] check.authority
        vault_fixture(0x43),  // [2] check.vault
        vault_fixture(0x44),  // [3] tail
    ]
}

/// The const is advertised on a container and the generated dispatcher
/// reaches the Ok-path emit against a composite context (borrow-check +
/// behavior proof; off-chain the syscall is a no-op). The Err path
/// still short-circuits before the emit.
#[test]
fn composite_dispatcher_emits_on_ok_and_not_on_err() {
    assert!(
        OperateTraced::EMIT_TOUCH_MAP,
        "the container must advertise EMIT_TOUCH_MAP = true"
    );
    let ok =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[0u8], &traced_accounts(), drive_traced);
    assert_eq!(ok.program_result, Ok(()), "the Ok handler must dispatch");
    let err =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[1u8], &traced_accounts(), drive_traced);
    assert_eq!(
        err.program_result,
        Err(ProgramError::Custom(9)),
        "the Err handler's error must propagate"
    );
}

/// Record-level proof that the emitted map describes the FLATTENED
/// slots: the same encoder the dispatcher's emit snapshots is decoded
/// host-side, and the Write records sit at slots 2 (the inner segment,
/// exact offset/size) and 3 (the trailing whole account).
///
/// Runs when the `touch-map` feature is unified into the build (the
/// `--workspace` lane, via hopper-smoke); the root-only lane compiles
/// it out exactly like hopper-runtime's own feature-gated tests.
#[cfg(feature = "touch-map")]
#[test]
fn touch_map_records_land_at_flattened_slots() {
    use hopper::hopper_runtime::segment_borrow::{
        TOUCH_MAP_HEADER_LEN, TOUCH_MAP_MAGIC, TOUCH_MAP_RECORD_LEN, TOUCH_MAP_VERSION,
    };

    /// Decode the v1 wire format against the PUBLIC constants (the same
    /// 1:1 replication the hopper-smoke SBF e2e uses), yielding
    /// `(slot, offset, size, write)` rows.
    fn decode(buf: &[u8]) -> std::vec::Vec<(u8, u32, u32, bool)> {
        assert_eq!(buf[0], TOUCH_MAP_MAGIC);
        assert_eq!(buf[1], TOUCH_MAP_VERSION);
        let count = buf[3] as usize;
        assert_eq!(
            buf.len(),
            TOUCH_MAP_HEADER_LEN + count * TOUCH_MAP_RECORD_LEN
        );
        (0..count)
            .map(|i| {
                let at = TOUCH_MAP_HEADER_LEN + i * TOUCH_MAP_RECORD_LEN;
                let packed = u32::from_le_bytes(buf[at + 1..at + 5].try_into().unwrap());
                let size = u32::from_le_bytes(buf[at + 5..at + 9].try_into().unwrap());
                (
                    buf[at],
                    packed & 0x7FFF_FFFF,
                    size,
                    packed & 0x8000_0000 != 0,
                )
            })
            .collect()
    }

    fn traced_handler<'info>(
        program_id: &'info Address,
        accounts: &'info [AccountView<'info>],
        instruction_data: &'info [u8],
    ) -> ProgramResult {
        let mut ctx = Context::new(program_id, accounts, instruction_data);
        {
            let mut bound = OperateTraced::bind(&mut ctx)?;
            {
                let mut check = bound.check()?;
                let mut balance = check.vault_balance_mut()?;
                *balance = WireU64::new(9);
            }
            let _tail = bound.tail_load_mut()?;
        }
        // Snapshot the same cumulative log `finish_with_touch_map`
        // emits, and pin the WRITE records to the flattened slots.
        let (buf, len) = ctx.encode_touch_map();
        let writes: std::vec::Vec<(u8, u32, u32, bool)> =
            decode(&buf[..len]).into_iter().filter(|r| r.3).collect();
        assert_eq!(
            writes,
            vec![
                (
                    2u8,
                    HEADER_LEN as u32 + Vault::BALANCE_OFFSET,
                    core::mem::size_of::<WireU64>() as u32,
                    true
                ),
                (3u8, 0, Vault::LEN as u32, true),
            ],
            "write records must sit at the FLATTENED slots 2 and 3"
        );
        Ok(())
    }

    fn traced_entry<'info>(
        program_id: &'info Address,
        accounts: &'info [AccountView<'info>],
        instruction_data: &'info [u8],
    ) -> ProgramResult {
        traced_handler(program_id, accounts, instruction_data)
    }

    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &traced_accounts(), traced_entry);
    assert_eq!(
        result.program_result,
        Ok(()),
        "the traced composite handler must succeed"
    );
}
