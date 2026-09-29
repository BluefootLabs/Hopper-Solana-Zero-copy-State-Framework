//! `#[check(..)]` value rules on layout fields, and the `accessors` option.
//!
//! A rule is a boolean expression over `value`, the field's native value.
//! The layout checks its rules with `check_rules`, refuses a bad write in
//! `try_set_<field>`, publishes the rules (and the integer bounds they
//! decide) in `FIELD_RULES`, and `#[derive(Accounts)]` checks the stored
//! values of every existing account it binds. A layout without rules gets
//! none of it and costs nothing.

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper::schema::{FieldRule, RuleChange};
use hopper_svm::{AccountFixture, HopperSvm};

const MAX_FEE_BPS: u16 = 1_000;

#[hopper::error_code]
pub enum PoolError {
    FeeTooHigh = 6_100,
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 91, version = 1, accessors)]
pub struct Pool {
    pub authority: Address,
    #[check(value >= 1 && value <= 10)]
    pub tier: u8,
    #[check(value <= MAX_FEE_BPS, error = PoolError::FeeTooHigh)]
    pub fee_bps: WireU16,
    /// Two rules on one field; the second reads another field.
    #[check(value >= 100)]
    #[check(value <= self.cap.get())]
    pub deposited: WireU64,
    pub cap: WireU64,
    pub paused: WireBool,
    #[check(-5 < value && value < 5)]
    pub skew: WireI32,
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 92, version = 1)]
pub struct Plain {
    pub total: WireU64,
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(compact, disc = 93, version = 1)]
pub struct CompactLimit {
    #[check(value != 0)]
    pub limit: WireU32,
}

fn valid_pool() -> Pool {
    Pool::new(
        Address::new_from_array([7; 32]),
        3,
        250,
        500,
        10_000,
        false,
        0,
    )
}

#[test]
fn check_rules_returns_the_first_failure_in_field_order() {
    let pool = valid_pool();
    assert_eq!(pool.check_rules(), Ok(()));

    let mut bad = pool;
    bad.tier = 0;
    assert_eq!(bad.check_rules(), Err(ProgramError::InvalidAccountData));
    bad.tier = 11;
    assert_eq!(bad.check_rules(), Err(ProgramError::InvalidAccountData));

    let mut bad = pool;
    bad.fee_bps = WireU16::new(1_001);
    assert_eq!(bad.check_rules(), Err(PoolError::FeeTooHigh.into()));
    // The tier rule is declared first, so it is the one reported.
    bad.tier = 0;
    assert_eq!(bad.check_rules(), Err(ProgramError::InvalidAccountData));

    let mut bad = pool;
    bad.deposited = WireU64::new(99);
    assert!(bad.check_rules().is_err());
    bad.deposited = WireU64::new(10_001);
    assert!(bad.check_rules().is_err(), "the cross-field rule holds");
    bad.cap = WireU64::new(10_001);
    assert_eq!(bad.check_rules(), Ok(()));

    let mut bad = pool;
    bad.skew = WireI32::new(-5);
    assert!(bad.check_rules().is_err());
    bad.skew = WireI32::new(-4);
    assert_eq!(bad.check_rules(), Ok(()));
}

#[test]
fn try_set_refuses_a_bad_value_and_leaves_the_field_unchanged() {
    let mut pool = valid_pool();
    assert_eq!(pool.try_set_tier(10), Ok(()));
    assert_eq!(pool.tier, 10);
    assert_eq!(pool.try_set_tier(11), Err(ProgramError::InvalidAccountData));
    assert_eq!(pool.tier, 10);

    assert_eq!(pool.try_set_fee_bps(1_000), Ok(()));
    assert_eq!(
        pool.try_set_fee_bps(1_001),
        Err(PoolError::FeeTooHigh.into())
    );
    assert_eq!(pool.fee_bps(), 1_000);

    // The new value is checked against the other field as it stands.
    assert_eq!(pool.try_set_deposited(10_000), Ok(()));
    assert!(pool.try_set_deposited(10_001).is_err());
    pool.set_cap(20_000);
    assert_eq!(pool.try_set_deposited(10_001), Ok(()));
    assert_eq!(pool.deposited(), 10_001);

    assert!(pool.try_set_skew(5).is_err());
    assert_eq!(pool.try_set_skew(4), Ok(()));
    assert_eq!(pool.skew(), 4);
}

#[test]
fn accessors_speak_native_values() {
    let mut pool = valid_pool();
    assert_eq!(pool.fee_bps(), 250u16);
    assert_eq!(pool.cap(), 10_000u64);
    assert!(!pool.paused());
    pool.set_paused(true);
    pool.set_cap(u64::MAX);
    assert!(pool.paused());
    assert_eq!(pool.cap, WireU64::new(u64::MAX));
    // A const context reads through the getter.
    const CAP: u64 = Pool::new(Address::new_from_array([0; 32]), 1, 0, 100, 9, false, 0).cap();
    assert_eq!(CAP, 9);
}

#[test]
fn field_rules_publish_the_text_and_the_bounds() {
    let rules = Pool::FIELD_RULES;
    assert_eq!(rules.len(), 5);
    assert_eq!(rules[0].field, "tier");
    assert_eq!((rules[0].min, rules[0].max), (Some(1), Some(10)));
    assert!(rules[0].rule.contains("value >= 1"));
    // A bound against a constant is not a literal bound.
    assert_eq!(rules[1].field, "fee_bps");
    assert_eq!((rules[1].min, rules[1].max), (None, None));
    assert_eq!(rules[2].field, "deposited");
    assert_eq!((rules[2].min, rules[2].max), (Some(100), None));
    assert_eq!(rules[3].field, "deposited");
    assert_eq!((rules[3].min, rules[3].max), (None, None));
    // Strict comparisons and a literal on the left normalize.
    assert_eq!(rules[4].field, "skew");
    assert_eq!((rules[4].min, rules[4].max), (Some(-4), Some(4)));
    // Only rules made of literal comparisons are decided by their bounds.
    let exact: std::vec::Vec<bool> = rules.iter().map(|r| r.exact).collect();
    assert_eq!(exact, [true, false, true, false, true]);

    assert!(Plain::FIELD_RULES.is_empty());
    assert_eq!(CompactLimit::FIELD_RULES.len(), 1);
}

#[test]
fn a_rule_change_is_classified_by_its_bounds() {
    let old = FieldRule {
        field: "tier",
        rule: "value >= 1 && value <= 10",
        min: Some(1),
        max: Some(10),
        exact: true,
    };
    let rule = |rule, min, max| FieldRule {
        field: "tier",
        rule,
        min,
        max,
        exact: true,
    };
    assert_eq!(old.change_to(&old), RuleChange::Unchanged);
    assert_eq!(
        old.change_to(&rule("value >= 1 && value <= 100", Some(1), Some(100))),
        RuleChange::Widened
    );
    assert_eq!(
        old.change_to(&rule("value >= 1", Some(1), None)),
        RuleChange::Widened
    );
    assert_eq!(
        old.change_to(&rule("value >= 2 && value <= 10", Some(2), Some(10))),
        RuleChange::Tightened
    );
    // Tighter below, wider above: a value the old rule refused passes.
    assert_eq!(
        old.change_to(&rule("value >= 2 && value <= 11", Some(2), Some(11))),
        RuleChange::Widened
    );
    // The same bounds spelled another way admit the same values.
    assert_eq!(
        old.change_to(&rule("1 <= value && value < 11", Some(1), Some(10))),
        RuleChange::Unchanged
    );
    // A condition beyond the bounds cannot be ordered against them.
    let inexact = FieldRule {
        field: "tier",
        rule: "value >= 2 && value <= self.cap.get()",
        min: Some(2),
        max: None,
        exact: false,
    };
    assert_eq!(old.change_to(&inexact), RuleChange::Widened);
    let inexact = FieldRule {
        max: Some(10),
        ..inexact
    };
    assert_eq!(old.change_to(&inexact), RuleChange::Rewritten);
}

// ── The accounts derive checks stored values ───────────────────────

const PROGRAM_ID: Address = Address::new_from_array([9u8; 32]);

#[derive(hopper::Accounts)]
pub struct UsePool<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,
    pub plain: Account<'info, Plain>,
    pub spare: Option<Account<'info, Pool>>,
}

#[derive(hopper::Accounts)]
pub struct RepairPool<'info> {
    #[account(mut, skip_rules)]
    pub pool: Account<'info, Pool>,
}

fn use_pool<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = UsePool::bind(&mut ctx)?;
    bound
        .accounts
        .pool
        .with_mut(|pool| pool.try_set_tier(instruction_data[0]))?;
    Ok(())
}

fn repair_pool<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = RepairPool::bind(&mut ctx)?;
    bound.accounts.pool.with_mut(|pool| {
        pool.tier = 1;
        pool.check_rules()
    })?;
    Ok(())
}

fn pool_fixture(addr_byte: u8, pool: Pool) -> AccountFixture {
    let mut data = vec![0u8; Pool::LEN];
    write_header(&mut data, Pool::DISC, Pool::VERSION, &Pool::LAYOUT_ID).unwrap();
    *Pool::overlay_mut(&mut data[HEADER_LEN..]).unwrap() = pool;
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
    .writable()
}

fn plain_fixture(addr_byte: u8) -> AccountFixture {
    let mut data = vec![0u8; Plain::LEN];
    write_header(&mut data, Plain::DISC, Plain::VERSION, &Plain::LAYOUT_ID).unwrap();
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
}

fn absent_fixture() -> AccountFixture {
    AccountFixture::new(PROGRAM_ID, Address::new_from_array([0u8; 32]), 1, 0)
}

#[test]
fn bind_accepts_stored_values_that_satisfy_the_rules() {
    let accounts = [
        pool_fixture(0x11, valid_pool()),
        plain_fixture(0x12),
        absent_fixture(),
    ];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[9], &accounts, use_pool);
    assert_eq!(result.program_result, Ok(()));
    assert_eq!(result.resulting_accounts[0].data[HEADER_LEN + 32], 9);

    // The handler's own write is refused by the same rule.
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[11], &accounts, use_pool);
    assert_eq!(result.program_result, Err(ProgramError::InvalidAccountData));
}

#[test]
fn bind_refuses_stored_values_that_break_a_rule() {
    let mut broken = valid_pool();
    broken.fee_bps = WireU16::new(5_000);
    let accounts = [
        pool_fixture(0x21, broken),
        plain_fixture(0x22),
        absent_fixture(),
    ];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[2], &accounts, use_pool);
    assert_eq!(result.program_result, Err(PoolError::FeeTooHigh.into()));

    // A present optional is checked; an absent one is not looked at.
    let accounts = [
        pool_fixture(0x23, valid_pool()),
        plain_fixture(0x24),
        pool_fixture(0x25, broken),
    ];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[2], &accounts, use_pool);
    assert_eq!(result.program_result, Err(PoolError::FeeTooHigh.into()));
}

#[test]
fn skip_rules_lets_a_repair_instruction_bind_broken_state() {
    let mut broken = valid_pool();
    broken.tier = 0;
    let accounts = [pool_fixture(0x31, broken)];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, repair_pool);
    assert_eq!(result.program_result, Ok(()));
    assert_eq!(result.resulting_accounts[0].data[HEADER_LEN + 32], 1);
}

#[test]
// The borrow is the point: `(&probe).method()` is the autoref selection the
// derive emits, and it must resolve the same way here.
#[allow(clippy::needless_borrow)]
fn the_probe_selects_the_check_only_for_ruled_layouts() {
    use hopper::__runtime::layout::FieldRules;

    fn assert_rules<T: FieldRules>() {}
    assert_rules::<Pool>();
    assert_rules::<CompactLimit>();

    let limit = CompactLimit::new(0);
    assert_eq!(limit.check_rules(), Err(ProgramError::InvalidAccountData));
    assert_eq!(CompactLimit::new(1).check_rules(), Ok(()));
}
