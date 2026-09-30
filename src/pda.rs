//! PDA (Program Derived Address) helpers for Hopper programs.
//!
//! Re-exports the Hopper-owned PDA functions from the runtime and provides
//! additional ergonomic helpers for common patterns.
//!
//! Every function here runs on the host as well as on SBF: off chain the
//! hash is the const SHA-256 and the curve check is Hopper's own, the same
//! bytes and the same refusal the syscalls give, so PDA logic can be tested
//! in a plain unit test. `#[derive(Accounts)]` / `#[hopper::context]`
//! lowering that references them (a `seeds = [...]` + `bump` account, or a
//! PDA `init`) type-checks and runs on both targets.

/// Derive a PDA from seeds and a program ID.
///
/// Returns the derived address. Fails if the seed combination does not
/// produce a valid off-curve point.
pub use hopper_runtime::pda::create_program_address;

/// Find a PDA and its bump seed.
///
/// Iterates bump seeds from 255 down to 0 until a valid off-curve address
/// is found. Returns `(address, bump)`.
pub use hopper_runtime::pda::find_program_address;

/// Verify that an account's address matches the expected PDA.
pub use hopper_runtime::pda::verify_pda;

/// Verify a PDA with an explicit bump seed appended to the seed list.
pub use hopper_runtime::pda::verify_pda_with_bump;

/// One-sha256 PDA verification for an address already bound to a
/// program-owned or about-to-be-created account, and its curve-checked twin.
/// `#[derive(Accounts)]` picks between them per field.
pub use hopper_runtime::pda::{
    find_bump_for_address, find_canonical_bump_checked, verify_pda_address,
    verify_pda_address_checked, verify_pda_address_cold,
};

/// A PDA evaluated at compile time for all-literal seeds; check the account
/// with `#[account(address = ...)]`, a 32-byte compare and no hash on chain.
/// `hopper::const_pda!` spells the seed list inline.
pub use hopper_runtime::pda::const_program_address;

/// The canonical address and bump of `const` seeds, searched at compile
/// time with a `const fn` curve check. Unlike `canonical_pda!`, the seeds
/// may be any `const` expressions, a declared program id included.
pub use hopper_runtime::pda::find_program_address_const;

/// The address the System Program's `*WithSeed` instructions derive from a
/// base, a seed, and an owner, and the check of an account against it. One
/// `sol_sha256` on chain; computed with the const SHA-256 on the host.
pub use hopper_runtime::pda::{create_with_seed, verify_address_with_seed, MAX_SEED_LEN};
