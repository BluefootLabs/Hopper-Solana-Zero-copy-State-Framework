//! PDA (Program Derived Address) helpers for Hopper programs.
//!
//! Re-exports the Hopper-owned PDA functions from hopper-native and provides
//! additional ergonomic helpers for common patterns.
//!
//! PDA functions require Solana syscalls and are only available on BPF targets.

/// Derive a PDA from seeds and a program ID.
///
/// Returns the derived address. Fails if the seed combination does not
/// produce a valid off-curve point.
#[cfg(target_os = "solana")]
pub use hopper_runtime::pda::create_program_address;

/// Find a PDA and its bump seed.
///
/// Iterates bump seeds from 255 down to 0 until a valid off-curve address
/// is found. Returns `(address, bump)`.
#[cfg(target_os = "solana")]
pub use hopper_runtime::pda::find_program_address;

/// Verify that an account's address matches the expected PDA.
#[cfg(target_os = "solana")]
pub use hopper_runtime::pda::verify_pda;

/// Verify a PDA with an explicit bump seed appended to the seed list.
#[cfg(target_os = "solana")]
pub use hopper_runtime::pda::verify_pda_with_bump;
