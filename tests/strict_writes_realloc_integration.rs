//! Regression: `strict_writes` + `realloc` must NOT silently degrade a
//! `mut(seg, ...)` field to a whole-account write grant.
//!
//! A field declaring BOTH `realloc = ...` and `mut(seg, ...)` used to be
//! classified `DeclaredRange::Whole` by the context macro — the `realloc`
//! attribute alone flipped it to whole-account and the segment scoping was
//! silently discarded. A program that combined `strict_writes` with
//! `realloc` therefore *believed* it had byte-range protection while
//! actually publishing (and enforcing) a whole-account grant.
//!
//! The fix: when `realloc` is combined with explicit `mut(seg)` segments,
//! the SEGMENT ranges govern the handler surface. `realloc` stays a
//! bind-time lifecycle (it resizes the account and tops up rent lamports —
//! both outside the `Context` byte-range gate, and the account stays in
//! the implied lamport set because that scan keys off `realloc.is_some()`
//! directly).
//!
//! Each assertion below is annotated with what it would have done BEFORE
//! the fix:
//!   (a) `WRITE_RANGES` is exactly the `fee_bps` segment — before the fix
//!       this was a single whole-account range `[0, u32::MAX)`.
//!   (b) a runtime write to an undeclared field on that account is refused
//!       with `Custom(0xD000 | idx)` — before the fix the whole-account
//!       grant admitted every offset, so this write was ALLOWED.
//!   (c) the `realloc` lifecycle still runs at bind — unaffected by the
//!       fix (it never crossed the write-range gate).

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_runtime::write_policy::{write_policy_violation, WritePolicy, WriteRange};
use hopper_svm::{AccountFixture, HopperSvm};

// ── Program under test ─────────────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 77, version = 1)]
pub struct FeeVault {
    /// The one field the adjust handler is allowed to write.
    pub fee_bps: WireU16,
    /// An undeclared sibling: writing it must be refused.
    pub admin_flag: WireU16,
    /// A wider undeclared sibling.
    pub balance: WireU64,
}

/// `strict_writes` context whose `vault` field combines `realloc` (a
/// bind-time grow) with `mut(fee_bps)` (the only writable byte range).
/// The realloc pays rent top-up from `payer` and zero-fills grown bytes.
#[hopper::context(strict_writes)]
pub struct AdjustFee {
    #[account(
        mut(fee_bps),
        realloc = FeeVault::LEN + 64,
        realloc_payer = payer,
        realloc_zero = true
    )]
    pub vault: FeeVault,

    #[signer]
    pub payer: AccountView<'static>,
}

fn expected_fee_bps_range() -> WriteRange {
    WriteRange::new(
        0,
        HEADER_LEN as u32 + FeeVault::FEE_BPS_OFFSET,
        core::mem::size_of::<WireU16>() as u32,
    )
}

// ── (a) Published surface is the segment, not the whole account ────

#[test]
fn realloc_plus_segment_publishes_only_the_segment_range() {
    assert!(AdjustFee::STRICT_WRITES);

    // Exactly one declared range: the `fee_bps` segment. Before the fix
    // this slice was `[WriteRange::whole_account(0)]` instead.
    assert_eq!(AdjustFee::WRITE_RANGES.len(), 1);
    assert_eq!(AdjustFee::WRITE_RANGES, &[expected_fee_bps_range()]);

    // The schema/manifest surface is the same shared const.
    assert_eq!(
        AdjustFee::SCHEMA_METADATA.write_ranges,
        AdjustFee::WRITE_RANGES
    );

    // The published range is the declared field, 2 bytes at HEADER_LEN.
    assert_eq!(AdjustFee::WRITE_RANGES[0].offset, HEADER_LEN as u32);
    assert_eq!(AdjustFee::WRITE_RANGES[0].size, 2);

    // Crucially NOT a whole-account grant: a tail-less field policy must
    // refuse CPI delegation and whole-account loads on the vault. Before
    // the fix this assertion FAILED — the range was `[0, u32::MAX)`.
    let policy = WritePolicy::new(AdjustFee::WRITE_RANGES);
    assert!(
        !policy.allows_whole_account_write(0),
        "realloc+segment must not publish a whole-account grant"
    );
}

// ── (b)/(c) Runtime enforcement + realloc lifecycle at bind ────────

fn adjust_fee_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let len_before = accounts[0].data_len();

    let mut ctx = Context::new(program_id, accounts, instruction_data);
    {
        // Bind installs the segment-only WritePolicy, then we drive the
        // realloc lifecycle explicitly (proving (c): it works even under
        // the segment-only policy, because it never crosses the gate).
        let bound = AdjustFee::bind(&mut ctx)?;
        bound.realloc_vault()?;
    }

    // (c) The account actually grew — the realloc ran to completion.
    let len_after = accounts[0].data_len();
    assert!(
        len_after > len_before,
        "(c) realloc must grow the vault at bind time: {len_before} -> {len_after}"
    );
    assert_eq!(len_after, FeeVault::LEN + 64);

    // The installed policy is exactly the published segment set.
    let policy = ctx
        .write_policy()
        .expect("strict_writes bind() must install a WritePolicy");
    assert_eq!(policy.allows, AdjustFee::WRITE_RANGES);

    // (b) Declared `fee_bps` write is admitted; the undeclared `admin_flag`
    // and `balance` siblings are refused at acquisition with the indexed
    // policy error. Before the fix the whole-account grant admitted ALL of
    // these, so the two `is_err()` / refusal assertions FAILED.
    let fee_off = HEADER_LEN as u32 + FeeVault::FEE_BPS_OFFSET;
    let flag_off = HEADER_LEN as u32 + FeeVault::ADMIN_FLAG_OFFSET;
    let balance_off = HEADER_LEN as u32 + FeeVault::BALANCE_OFFSET;
    assert!(ctx.segment_mut::<[u8; 2]>(0, fee_off).is_ok());
    assert_eq!(
        ctx.segment_mut::<[u8; 2]>(0, flag_off).map(|_| ()),
        Err(write_policy_violation(0)),
        "(b) undeclared admin_flag write must be refused Custom(0xD000)"
    );
    assert!(ctx.segment_mut::<[u8; 8]>(0, balance_off).is_err());

    // A whole-account load is refused too: the segment policy is not a
    // whole-account grant (before the fix this was ALLOWED).
    assert!(ctx.load_mut::<FeeVault>(0).is_err());
    Ok(())
}

#[test]
fn realloc_plus_segment_enforces_segment_and_reallocs_at_bind() {
    let program_id = Address::new_from_array([9u8; 32]);

    let mut vault_data = vec![0u8; FeeVault::LEN];
    write_header(
        &mut vault_data,
        FeeVault::DISC,
        FeeVault::VERSION,
        &FeeVault::LAYOUT_ID,
    )
    .unwrap();
    // Fund the vault well past the rent-exempt minimum for the grown size
    // so the realloc has no lamport deficit (no payer top-up needed).
    let vault = AccountFixture::with_data(
        Address::new_from_array([1u8; 32]),
        program_id,
        100_000_000,
        vault_data,
    )
    .writable();
    let payer = AccountFixture::new(
        Address::new_from_array([2u8; 32]),
        Address::new_from_array([0u8; 32]),
        100_000_000,
        0,
    )
    .signer()
    .writable();

    let result =
        HopperSvm::new().process_instruction(program_id, &[], &[vault, payer], adjust_fee_handler);
    assert!(
        result.program_result.is_ok(),
        "realloc+segment enforcement failed: {:?}",
        result.program_result
    );
}
