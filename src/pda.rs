//! PDA (Program Derived Address) helpers for Hopper programs.
//!
//! Re-exports the Hopper-owned PDA functions from the runtime and provides
//! additional ergonomic helpers for common patterns.
//!
//! These functions compile on every target so `#[derive(Accounts)]` /
//! `#[hopper::context]` lowering that references them (a `seeds = [...]` +
//! `bump` account, or a PDA `init`) type-checks on the host as well as on
//! SBF. On non-SVM hosts the sha256-backed derivations (`find_program_address`,
//! `verify_pda`) panic if actually called, because the syscall is
//! unavailable — host tests must exercise PDA paths through the SVM harness
//! (`hopper-svm` / `hopper-test`), where the syscall is emulated. Pre-0.3.0
//! these re-exports were gated `#[cfg(target_os = "solana")]`, which made any
//! host build of a seeds-bearing context fail to resolve
//! `hopper::pda::find_program_address`; the runtime functions themselves were
//! always host-compilable, so the gate was removed.

/// Derive a PDA from seeds and a program ID.
///
/// Returns the derived address. Fails if the seed combination does not
/// produce a valid off-curve point. Requires the sha256 syscall, so it
/// panics on non-SVM hosts — run PDA derivations through the SVM harness.
pub use hopper_runtime::pda::create_program_address;

/// Find a PDA and its bump seed.
///
/// Iterates bump seeds from 255 down to 0 until a valid off-curve address
/// is found. Returns `(address, bump)`. Requires the sha256 syscall, so it
/// panics on non-SVM hosts — run PDA derivations through the SVM harness.
pub use hopper_runtime::pda::find_program_address;

/// Verify that an account's address matches the expected PDA.
pub use hopper_runtime::pda::verify_pda;

/// Verify a PDA with an explicit bump seed appended to the seed list.
pub use hopper_runtime::pda::verify_pda_with_bump;
