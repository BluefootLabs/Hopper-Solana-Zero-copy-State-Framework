//! Hopper's policy-driven zero-copy runtime, demonstrated.
//!
//! This example ships three sibling programs that differ *only* in
//! the `#[hopper::program(...)]` attribute. They exercise the three
//! shipping modes Hopper offers:
//!
//! - `strict_vault` — [`HopperProgramPolicy::STRICT`]: every lever on.
//!   Recommended for production protocols. Segment-borrow tracking,
//!   layout-header validation, auto-injected `validate(ctx)?`, and
//!   `unsafe` allowed-but-isolated.
//!
//! - `sealed_vault` — [`HopperProgramPolicy::SEALED`]: strict + token
//!   checks + **no** `unsafe` anywhere inside handlers. The program
//!   macro emits `#[deny(unsafe_code)]` on each handler so a stray
//!   `unsafe { ... }` block is a compile error. Demonstrates the
//!   `#[instruction(N, unsafe_memory)]` per-instruction opt-in that
//!   re-enables raw pointer access for exactly one "fast path" while
//!   leaving every other handler sealed.
//!
//! - `raw_vault` — [`HopperProgramPolicy::RAW`]: Pinocchio-parity.
//!   Every lever off. Used when the protocol author wants full
//!   control and documented responsibility for the invariants Hopper
//!   would otherwise enforce.
//!
//! Every mode compiles to the same `process_instruction` dispatch
//! shape. The difference is the compile-time const each program
//! emits (`HOPPER_PROGRAM_POLICY`) and the compile-time deny-by-default
//! on handlers in sealed mode.
//!
//! [`HopperProgramPolicy::STRICT`]: hopper::hopper_runtime::HopperProgramPolicy::STRICT
//! [`HopperProgramPolicy::SEALED`]: hopper::hopper_runtime::HopperProgramPolicy::SEALED
//! [`HopperProgramPolicy::RAW`]: hopper::hopper_runtime::HopperProgramPolicy::RAW

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

// ══════════════════════════════════════════════════════════════════════
//  Shared layout
// ══════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    pub balance: WireU64,
    pub pending_rewards: WireU64,
}

#[hopper::context]
pub struct Deposit {
    #[account(mut(balance))]
    pub vault: Vault,

    #[signer]
    pub authority: AccountView,
}

#[hopper::context]
pub struct Sweep {
    #[account(mut)]
    pub vault: Vault,

    #[signer]
    pub authority: AccountView,
}

// ══════════════════════════════════════════════════════════════════════
//  STRICT mode: every safety lever engaged. Default policy.
// ══════════════════════════════════════════════════════════════════════

#[hopper::program(strict)]
pub mod strict_vault {
    use super::*;

    #[instruction(0)]
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
        let mut balance = ctx.vault_balance_mut()?;
        let next = balance
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *balance = WireU64::new(next);
        Ok(())
    }

    #[instruction(1)]
    pub fn sweep(ctx: Context<Sweep>) -> ProgramResult {
        let mut vault = ctx.vault_load_mut()?;
        vault.pending_rewards = WireU64::new(0);
        Ok(())
    }
}

// ══════════════════════════════════════════════════════════════════════
//  SEALED mode: strict + token checks + no `unsafe` allowed.
// ══════════════════════════════════════════════════════════════════════
//
//  Every handler is wrapped in `#[deny(unsafe_code)]` by the program
//  macro unless it opts in with `#[instruction(N, unsafe_memory)]`.
//  The `fast_path` handler below opts in to demonstrate the per-
//  instruction override; every other handler in this module is
//  compile-rejected from dropping to raw pointer access.

#[hopper::program(sealed)]
pub mod sealed_vault {
    use super::*;
    use hopper::require;

    /// Default: zero `unsafe` permitted. A stray `unsafe {}` inside
    /// this handler body would fail to compile under the
    /// `#[deny(unsafe_code)]` attribute the program macro emits.
    #[instruction(0)]
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
        let mut balance = ctx.vault_balance_mut()?;
        let next = balance
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *balance = WireU64::new(next);
        Ok(())
    }

    /// Opt-in fast path: `#[instruction(1, unsafe_memory)]` re-enables
    /// `unsafe` blocks for this handler only. This is the
    /// "safe-by-default, raw-where-needed" Hopper promise.
    #[instruction(1, unsafe_memory)]
    #[allow(unused_unsafe)]
    pub fn fast_sweep(ctx: Context<Sweep>) -> ProgramResult {
        // The sealed-mode default would reject this block; the
        // per-instruction override restores the normal lint level.
        // The body happens to be safe today, but the module is
        // signalling it may reach raw pointer territory later
        // without requiring a policy change.
        let cleared = unsafe {
            let mut vault = ctx.vault_load_mut()?;
            vault.pending_rewards = WireU64::new(0);
            vault.pending_rewards.get()
        };
        // Sealed-mode invariants still hold: token checks and
        // layout-header validation ran in `Sweep::bind`.
        require!(cleared == 0);
        Ok(())
    }
}

// ══════════════════════════════════════════════════════════════════════
//  RAW mode: Pinocchio parity. Every lever off.
// ══════════════════════════════════════════════════════════════════════
//
//  `strict = false` skips `ContextSpec::bind(ctx)?`, so handlers take
//  a raw `&mut Context<'_>` and the author is responsible for every
//  check. This is the mode to reach for when the protocol has its
//  own validation flow that Hopper should not second-guess.

#[hopper::program(raw)]
pub mod raw_vault {
    use super::*;

    #[instruction(0)]
    pub fn deposit(ctx: &mut Context<'_>, amount: u64) -> ProgramResult {
        // Raw mode: the author does their own account resolution.
        let mut vault = ctx.load_mut::<Vault>(0)?;
        let next = vault
            .balance
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        vault.balance = WireU64::new(next);
        Ok(())
    }

    /// Skip even the token-check promise for this handler. Useful
    /// when the instruction does its own SPL plumbing and Hopper's
    /// pre-check would double up.
    #[instruction(1, skip_token_checks)]
    pub fn raw_sweep(ctx: &mut Context<'_>) -> ProgramResult {
        let mut vault = ctx.load_mut::<Vault>(0)?;
        vault.pending_rewards = WireU64::new(0);
        Ok(())
    }

    /// Raw-mode + per-instruction `unsafe_memory` demonstrates the
    /// canonical escape hatch the audit documents:
    /// `unsafe { ctx.as_mut_ptr(index)?.add(offset) as *mut T }`.
    /// Raw mode keeps this available for hand-tuned parsing of
    /// non-standard account layouts that the typed API cannot
    /// express at zero cost.
    #[instruction(2, unsafe_memory)]
    pub fn raw_pointer_reset(ctx: &mut Context<'_>) -> ProgramResult {
        // SAFETY: Vault body starts immediately after the 16-byte
        // Hopper header. The `pending_rewards` field lives at offset
        // `8` inside the body, so its absolute offset is `16 + 8 = 24`.
        // We hold no other borrow on account 0 across this write.
        hopper::hopper_unsafe_region!("zero pending_rewards via raw ptr", {
            let ptr = ctx.as_mut_ptr(0)?;
            let field = ptr.add(24) as *mut u64;
            field.write_unaligned(0);
        });
        Ok(())
    }

    /// The "MIXED" pattern from the audit's Section 2: safe segment
    /// write, drop to raw pointer for a fast-path memset, then back
    /// to safe code for an invariant check. All three regions live
    /// in the same handler without a policy change.
    ///
    /// Demonstrates that Hopper's raw escape hatch composes with the
    /// typed API — the unsafe block is surgical, not contagious.
    #[instruction(3, unsafe_memory)]
    pub fn hybrid_bump(ctx: &mut Context<'_>, amount: u64) -> ProgramResult {
        // Safe region: add to balance through the typed accessor.
        {
            let mut vault = ctx.load_mut::<Vault>(0)?;
            let next = vault
                .balance
                .get()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            vault.balance = WireU64::new(next);
        }

        // Unsafe region: zero the pending_rewards field in place
        // without re-borrowing the whole vault. The typed API above
        // dropped before we get here, so there's no alias.
        // SAFETY: `pending_rewards` is at body offset 8, absolute 24.
        hopper::hopper_unsafe_region!("zero pending_rewards after balance bump", {
            let ptr = ctx.as_mut_ptr(0)?;
            (ptr.add(24) as *mut u64).write_unaligned(0);
        });

        // Safe region again: read the result through the typed API
        // and assert the invariant.
        let vault = ctx.load::<Vault>(0)?;
        hopper::require!(vault.pending_rewards.get() == 0);
        Ok(())
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Compile-time policy assertions
// ══════════════════════════════════════════════════════════════════════
//
//  Each of the three programs emits `HOPPER_PROGRAM_POLICY`. The
//  assertions below run at `const` evaluation time, so a regression
//  where the macro stops emitting the right policy fails the build
//  rather than waiting for a test to execute.

const _STRICT_POLICY_IS_STRICT: () = {
    assert!(strict_vault::HOPPER_PROGRAM_POLICY.strict);
    assert!(strict_vault::HOPPER_PROGRAM_POLICY.enforce_token_checks);
    assert!(strict_vault::HOPPER_PROGRAM_POLICY.allow_unsafe);
};

const _SEALED_POLICY_IS_SEALED: () = {
    assert!(sealed_vault::HOPPER_PROGRAM_POLICY.strict);
    assert!(sealed_vault::HOPPER_PROGRAM_POLICY.enforce_token_checks);
    assert!(!sealed_vault::HOPPER_PROGRAM_POLICY.allow_unsafe);
};

const _RAW_POLICY_IS_RAW: () = {
    assert!(!raw_vault::HOPPER_PROGRAM_POLICY.strict);
    assert!(!raw_vault::HOPPER_PROGRAM_POLICY.enforce_token_checks);
    assert!(raw_vault::HOPPER_PROGRAM_POLICY.allow_unsafe);
};

const _SEALED_FAST_SWEEP_OPTS_INTO_UNSAFE: () = {
    assert!(sealed_vault::FAST_SWEEP_POLICY.unsafe_memory);
    assert!(!sealed_vault::FAST_SWEEP_POLICY.skip_token_checks);
};

const _RAW_RAW_SWEEP_SKIPS_TOKEN_CHECKS: () = {
    assert!(raw_vault::RAW_SWEEP_POLICY.skip_token_checks);
    assert!(!raw_vault::RAW_SWEEP_POLICY.unsafe_memory);
};

const _RAW_POINTER_RESET_OPTS_INTO_UNSAFE: () = {
    assert!(raw_vault::RAW_POINTER_RESET_POLICY.unsafe_memory);
    assert!(!raw_vault::RAW_POINTER_RESET_POLICY.skip_token_checks);
};

const _RAW_HYBRID_BUMP_OPTS_INTO_UNSAFE: () = {
    assert!(raw_vault::HYBRID_BUMP_POLICY.unsafe_memory);
    assert!(!raw_vault::HYBRID_BUMP_POLICY.skip_token_checks);
};

#[cfg(test)]
mod tests {
    use super::*;
    use hopper::hopper_runtime::HopperProgramPolicy;

    #[test]
    fn strict_matches_named_constant() {
        assert_eq!(
            strict_vault::HOPPER_PROGRAM_POLICY,
            HopperProgramPolicy::STRICT
        );
    }

    #[test]
    fn sealed_matches_named_constant() {
        assert_eq!(
            sealed_vault::HOPPER_PROGRAM_POLICY,
            HopperProgramPolicy::SEALED
        );
    }

    #[test]
    fn raw_matches_named_constant() {
        assert_eq!(
            raw_vault::HOPPER_PROGRAM_POLICY,
            HopperProgramPolicy::RAW
        );
    }
}
