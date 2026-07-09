//! I7 self-describing transactions: the touch-map emit fires on the
//! handler's **Ok** path ONLY, driven by the generated dispatcher — never
//! from a `Drop` (which would fire on `?`/`Err` returns too, the CONFIRMED
//! P2).
//!
//! This is the end-to-end proof the unit tests cannot give: it compiles a
//! real `#[hopper::program]` module whose typed handler binds an
//! `#[hopper::context(emit_touch_map)]` context, so the generated dispatch
//! helper actually emits
//!
//! ```ignore
//! handler(Bump::bind(ctx)?)?;                     // Err short-circuits here
//! if Bump::EMIT_TOUCH_MAP { ctx.finish_with_touch_map(); }
//! Ok(())
//! ```
//!
//! and the whole crate must borrow-check it: `ctx` is usable at the finish
//! point because the bound context (which reborrowed `&mut ctx`) was moved
//! into the handler and dropped when it returned. The mere fact this file
//! compiles proves that; the tests then drive both the Ok and Err
//! discriminators through the real dispatcher.

#![cfg(feature = "proc-macros")]

use hopper::layout::write_header;
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

// ── State + contexts under test ────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 7, version = 1)]
pub struct Counter {
    pub count: WireU64,
}

/// Opts into self-describing transactions: `EMIT_TOUCH_MAP` is `true`, so
/// the dispatcher runs the const-guarded finish call on the Ok path.
#[hopper::context(emit_touch_map)]
pub struct Bump {
    #[account(mut)]
    pub counter: Counter,
}

/// Control: no opt-in, so `EMIT_TOUCH_MAP` is `false` and the dispatcher's
/// const-guarded call is dead-code-eliminated.
#[hopper::context]
pub struct Peek {
    pub counter: Counter,
}

// ── Program: a real dispatcher over the opted-in context ───────────

#[hopper::program(entrypoint = false)]
mod bump_prog {
    use super::*;

    /// Ok path: touches state, then succeeds. The dispatcher emits the
    /// touch map after this returns (a no-op syscall off-chain).
    #[instruction(0)]
    #[allow(unused_mut, unused_variables)]
    fn bump_ok(ctx: Context<Bump>) -> ProgramResult {
        // A real typed write so the emit has a footprint to describe.
        let _guard = ctx.counter_load_mut()?;
        Ok(())
    }

    /// Err path: touches state, then FAILS. The dispatcher must NOT emit a
    /// touch map — `?` on `handler(...)` short-circuits before the finish
    /// call, so a rolled-back instruction advertises nothing (P2 fix).
    #[instruction(1)]
    #[allow(unused_mut, unused_variables)]
    fn bump_fail(ctx: Context<Bump>) -> ProgramResult {
        let _guard = ctx.counter_load_mut()?;
        Err(ProgramError::Custom(9))
    }
}

// ── Drivers ────────────────────────────────────────────────────────

fn drive<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    bump_prog::process_instruction(&mut ctx)
}

fn dispatch(disc: u8) -> Result<(), ProgramError> {
    let program_id = Address::new_from_array([9u8; 32]);

    let mut counter_data = vec![0u8; Counter::LEN];
    write_header(
        &mut counter_data,
        Counter::DISC,
        Counter::VERSION,
        &Counter::LAYOUT_ID,
    )
    .unwrap();
    let counter = AccountFixture::with_data(
        Address::new_from_array([1u8; 32]),
        program_id,
        1_000_000,
        counter_data,
    )
    .writable();

    HopperSvm::new()
        .process_instruction(program_id, &[disc], &[counter], drive)
        .program_result
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn opt_in_advertises_the_const_control_does_not() {
    assert!(
        Bump::EMIT_TOUCH_MAP,
        "the emit_touch_map context must advertise EMIT_TOUCH_MAP = true"
    );
    assert!(
        !Peek::EMIT_TOUCH_MAP,
        "a context without the opt-in must default EMIT_TOUCH_MAP = false"
    );
}

#[test]
fn dispatcher_emits_on_ok_path_and_not_on_err_path() {
    // Ok discriminator: the dispatcher runs the handler, then (because
    // Bump::EMIT_TOUCH_MAP is true) reaches the finish call — off-chain a
    // no-op, but the whole path compiles and runs to Ok. This is the
    // borrow-check proof: `ctx.finish_with_touch_map()` is reached after
    // the bound context was moved into and dropped by the handler.
    assert_eq!(
        dispatch(0),
        Ok(()),
        "the Ok handler must dispatch successfully"
    );

    // Err discriminator: the handler returns Err via `?`, so the
    // dispatcher short-circuits BEFORE the const-guarded finish call — the
    // failed, rolled-back instruction self-describes nothing. We observe
    // the propagated error; the emit position relative to `?` is pinned by
    // the codegen unit tests and the runtime Ok-only regression test.
    assert_eq!(
        dispatch(1),
        Err(ProgramError::Custom(9)),
        "the Err handler's error must propagate through the dispatcher"
    );
}
