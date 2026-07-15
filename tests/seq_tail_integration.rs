//! `Seq<T>` — the growable typed sequence tail, end-to-end.
//!
//! A `#[hopper::account]` layout with a fixed head plus a
//! `members: Seq<'a, Address>` growable tail, driven under a
//! `#[hopper::context(strict_writes)]` handler that declares `tail(members)`.
//!
//! Proves the acceptance bar:
//!   (a) `push` through the gated cursor succeeds and is visible on re-read;
//!   (b) a write to a HEAD field is REFUSED `Custom(0xD000 | idx)` — the
//!       open-ended tail range does not leak backwards onto the head;
//!   (c) `allows_whole_account_write == false` for the roster account (so
//!       CPI writable-meta delegation stays refused);
//!   (d) the layout id is capacity-independent — two accounts of different
//!       data lengths are the SAME account type;
//!   (e) growing via `realloc` + pushing past the old capacity works
//!       end-to-end.
//!
//! Design note: the growable-tail grant is spelled `tail(members)` in the
//! context (not `mut(members)`) because the context proc-macro cannot
//! introspect the layout to tell a `Seq` tail from a fixed segment; `tail`
//! is explicit, lowers to an open-ended `WriteRange::tail_from`, and
//! composes with `realloc`.

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_runtime::write_policy::{write_policy_violation, WritePolicy};
use hopper_runtime::ProgramError;
use hopper_svm::{AccountFixture, HopperSvm};

// ── Layout under test: fixed head + growable Seq<Address> tail ──────

#[hopper::account(disc = 55, version = 1)]
pub struct Roster<'a> {
    /// Head field 0 (undeclared: a write here must be refused).
    pub admin: WireU64,
    /// Head field 1 (undeclared).
    pub epoch: WireU64,
    /// The growable tail: a capacity-free typed sequence of addresses.
    pub members: Seq<'a, Address>,
}

/// `strict_writes` handler: only the `members` tail is writable (via the
/// open-ended `tail_from` range), and it grows through `realloc`.
#[hopper::context(strict_writes)]
pub struct AddMember {
    #[account(
        tail(members),
        realloc = Roster::space_for(8),
        realloc_payer = payer,
        realloc_zero = true
    )]
    pub roster: Roster,

    #[signer]
    pub payer: AccountView<'static>,
}

// ── (c)/(d) Static properties: capacity-independent id, tail-only policy ─

#[test]
fn layout_id_is_capacity_independent() {
    // Two roster accounts sized for different capacities share ONE layout
    // (same disc + LAYOUT_ID): growing never changes the account type.
    let small = Roster::space_for(1);
    let large = Roster::space_for(64);
    assert!(large > small);
    assert_eq!(Roster::MIN_SPACE, Roster::LEN + 4);
    assert_eq!(small, Roster::LEN + 4 + 32);
    assert_eq!(large, Roster::LEN + 4 + 64 * 32);

    let mut d_small = vec![0u8; small];
    let mut d_large = vec![0u8; large];
    write_header(
        &mut d_small,
        Roster::DISC,
        Roster::VERSION,
        &Roster::LAYOUT_ID,
    )
    .unwrap();
    write_header(
        &mut d_large,
        Roster::DISC,
        Roster::VERSION,
        &Roster::LAYOUT_ID,
    )
    .unwrap();

    // Different live capacities...
    assert_eq!(Roster::members(&d_small).unwrap().capacity(), 1);
    assert_eq!(Roster::members(&d_large).unwrap().capacity(), 64);
    // ...but the layout id (the account type) is one fixed value, because
    // the `seq<Address>` schema carries no capacity. Both zeroed tails read
    // as an empty sequence.
    assert_eq!(Roster::members(&d_small).unwrap().len(), 0);
    assert_eq!(Roster::members(&d_large).unwrap().len(), 0);
}

#[test]
fn tail_context_publishes_a_single_open_ended_tail_range() {
    assert!(AddMember::STRICT_WRITES);
    // Exactly one declared range: the open-ended tail. NOT whole-account.
    assert_eq!(AddMember::WRITE_RANGES.len(), 1);
    let r = AddMember::WRITE_RANGES[0];
    assert_eq!(r.account_index, 0);
    // The range starts at the tail region (past the fixed head) and is
    // open-ended.
    assert_eq!(r.offset, Roster::TAIL_PREFIX_OFFSET as u32);
    assert_eq!(r.offset, HEADER_LEN as u32 + Roster::MEMBERS_OFFSET);
    assert_eq!(r.size, u32::MAX);

    // Published == enforced, the tail edition: the manifest surface is the
    // SAME const the runtime installs — the open-ended range is what
    // schedulers/verifiers see AND what the gate enforces.
    assert_eq!(
        AddMember::SCHEMA_METADATA.write_ranges,
        AddMember::WRITE_RANGES
    );

    // (c) An open tail range starting past the head is NOT a whole-account
    // grant, so CPI writable-meta delegation stays refused.
    let policy = WritePolicy::new(AddMember::WRITE_RANGES);
    assert!(!policy.allows_whole_account_write(0));
}

// ── Systems-mode spelling: `#[tail(seq<T>)]` in `#[hopper::dynamic_account]` ─

#[hopper::dynamic_account(disc = 56, version = 1)]
pub struct ScoreBoard {
    pub round: u64,

    #[tail(seq<u64>)]
    pub scores: Vec<u64>,
}

/// The documented systems-mode spelling generates the same capacity-free
/// sizing, cursors, and open-ended-range constants as the pretty
/// `Seq<'a, T>` authoring type.
#[test]
fn systems_mode_seq_tail_spelling_generates_the_same_cursors() {
    assert_eq!(ScoreBoard::MIN_SPACE, ScoreBoard::LEN + 4);
    assert_eq!(ScoreBoard::space_for(3), ScoreBoard::LEN + 4 + 3 * 8);

    let mut data = vec![0u8; ScoreBoard::space_for(3)];
    write_header(
        &mut data,
        ScoreBoard::DISC,
        ScoreBoard::VERSION,
        &ScoreBoard::LAYOUT_ID,
    )
    .unwrap();

    // A zeroed tail reads empty; push/get roundtrip through the cursors.
    {
        let mut seq = ScoreBoard::scores_mut(&mut data).unwrap();
        assert_eq!(seq.capacity(), 3);
        assert!(seq.is_empty());
        seq.push(11).unwrap();
        seq.push(22).unwrap();
    }
    let seq = ScoreBoard::scores(&data).unwrap();
    assert_eq!(seq.len(), 2);
    assert_eq!(seq.get(0).unwrap(), 11);
    assert_eq!(seq.get(1).unwrap(), 22);

    // The tail-range constants a `tail(scores)` context declaration
    // resolves against line up with the layout.
    assert_eq!(ScoreBoard::SCORES_OFFSET, ScoreBoard::BODY_SIZE as u32);
    assert_eq!(
        ScoreBoard::SCORES_ABS_OFFSET,
        ScoreBoard::TAIL_PREFIX_OFFSET as u32
    );
    assert_eq!(ScoreBoard::SCORES_SIZE, u32::MAX);
}

// ── (a)/(b)/(e) Runtime enforcement, push, growth ──────────────────

fn add_member_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let m0 = Address::new_from_array([1u8; 32]);
    let m1 = Address::new_from_array([2u8; 32]);
    let tail_off = Roster::TAIL_PREFIX_OFFSET as u32;

    let mut ctx = Context::new(program_id, accounts, instruction_data);

    // Bind installs the tail-only write policy.
    {
        let _bound = AddMember::bind(&mut ctx)?;
    }

    // (c) The installed policy is one open-ended tail range, not whole-account.
    {
        let policy = ctx.write_policy().expect("strict_writes installs a policy");
        assert_eq!(policy.allows.len(), 1);
        assert_eq!(policy.allows[0].offset, tail_off);
        assert_eq!(policy.allows[0].size, u32::MAX);
        assert!(!policy.allows_whole_account_write(0));
    }

    // (a) push through the gated cursor (capacity 1), visible on re-read.
    {
        let mut w = ctx.tail_seq_mut::<Address>(0, tail_off)?;
        w.seq_mut()?.push(m0)?;
    }
    {
        let r = ctx.tail_seq_ref::<Address>(0, tail_off)?;
        let seq = r.seq()?;
        assert_eq!(seq.len(), 1);
        assert_eq!(seq.get(0)?, m0);
    }

    // (e, first half) at capacity 1 a second push is refused — grow needed.
    {
        let mut w = ctx.tail_seq_mut::<Address>(0, tail_off)?;
        assert_eq!(
            w.seq_mut()?.push(m1),
            Err(ProgramError::AccountDataTooSmall),
            "push past capacity must ask for a grow"
        );
    }

    // (b) writing a HEAD field is refused with the indexed policy error —
    // the open tail range does not leak backwards onto the head.
    let admin_off = HEADER_LEN as u32 + Roster::ADMIN_OFFSET;
    let epoch_off = HEADER_LEN as u32 + Roster::EPOCH_OFFSET;
    assert_eq!(
        ctx.segment_mut::<[u8; 8]>(0, admin_off).map(|_| ()),
        Err(write_policy_violation(0)),
        "(b) undeclared head write must be refused Custom(0xD000)"
    );
    assert!(ctx.segment_mut::<[u8; 8]>(0, epoch_off).is_err());

    // (e, second half) grow via realloc, then push past the OLD capacity.
    {
        let bound = AddMember::bind(&mut ctx)?;
        bound.realloc_roster()?;
    }
    {
        let mut w = ctx.tail_seq_mut::<Address>(0, tail_off)?;
        assert_eq!(
            w.seq_mut()?.capacity(),
            8,
            "realloc raised the live capacity"
        );
        w.seq_mut()?.push(m1)?; // now succeeds past the old capacity of 1
    }
    {
        let r = ctx.tail_seq_ref::<Address>(0, tail_off)?;
        let seq = r.seq()?;
        assert_eq!(seq.len(), 2);
        assert_eq!(seq.get(0)?, m0);
        assert_eq!(seq.get(1)?, m1);
    }
    Ok(())
}

#[test]
fn seq_tail_push_grow_and_head_protection_end_to_end() {
    let program_id = Address::new_from_array([9u8; 32]);

    // Start at capacity 1 so the second push must trigger a grow.
    let mut roster_data = vec![0u8; Roster::space_for(1)];
    write_header(
        &mut roster_data,
        Roster::DISC,
        Roster::VERSION,
        &Roster::LAYOUT_ID,
    )
    .unwrap();
    // Fund past the rent-exempt minimum for the grown size (no deficit).
    let roster = AccountFixture::with_data(
        Address::new_from_array([1u8; 32]),
        program_id,
        100_000_000,
        roster_data,
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
        HopperSvm::new().process_instruction(program_id, &[], &[roster, payer], add_member_handler);
    assert!(
        result.program_result.is_ok(),
        "seq tail end-to-end failed: {:?}",
        result.program_result
    );
}
