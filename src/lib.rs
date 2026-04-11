//! Optional proc macro DX layer for Hopper.
//!
//! Provides `#[hopper_state]`, `#[hopper_context]`, and `#[hopper_program]`
//! attribute macros that generate zero-cost code targeting Hopper's runtime
//! primitives.
//!
//! **Not required.** Every feature these macros provide is achievable through
//! Hopper's declarative `macro_rules!` macros or hand-written code. These
//! exist purely for developer velocity — the generated code compiles to the
//! exact same pointer arithmetic as raw Pinocchio.
//!
//! # Design Philosophy
//!
//! - **Macros generate code, not behavior.** No hidden runtime logic.
//! - **Everything inlines.** No function calls that wouldn't exist in hand-written code.
//! - **No heap.** Generated code is `no_std`, `no_alloc`.
//! - **Optional.** Core Hopper never depends on this crate.

extern crate proc_macro;

mod state;
mod context;
mod program;

use proc_macro::TokenStream;

/// Generate a `SegmentMap` implementation for a zero-copy layout struct.
///
/// Computes field offsets at compile time and emits a const segment table.
/// The generated code is zero-cost — segment lookups resolve to const loads.
///
/// # Example
///
/// ```ignore
/// #[hopper_state]
/// #[repr(C)]
/// pub struct Vault {
///     pub authority: [u8; 32],  // TypedAddress<Authority>
///     pub balance: [u8; 8],     // WireU64
///     pub bump: u8,
/// }
///
/// // Generated:
/// // impl SegmentMap for Vault { ... }
/// // const VAULT_SEGMENTS: ... (for direct access)
/// ```
#[proc_macro_attribute]
pub fn hopper_state(attr: TokenStream, item: TokenStream) -> TokenStream {
    state::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Generate typed context accessors with segment-level borrow tracking.
///
/// Each field annotated with `#[account(mut(field1, field2))]` gets accessor
/// methods that:
/// 1. Look up the segment by const offset (no string matching)
/// 2. Register a segment borrow in the registry
/// 3. Return a typed reference via pointer cast
///
/// # Example
///
/// ```ignore
/// #[hopper_context]
/// pub struct Deposit {
///     #[account(signer, mut)]
///     pub depositor: AccountView,
///
///     #[account(mut(balance))]
///     pub vault: Vault,
/// }
///
/// // Generated:
/// // impl<'a> Deposit<'a> {
/// //     pub fn vault_balance_mut(&mut self) -> Result<RefMut<WireU64>, ProgramError> { ... }
/// // }
/// ```
#[proc_macro_attribute]
pub fn hopper_context(attr: TokenStream, item: TokenStream) -> TokenStream {
    context::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Generate a dispatch table for a Hopper program module.
///
/// Maps instruction discriminator bytes to handler functions, generating
/// a clean entrypoint with minimal branching.
///
/// # Example
///
/// ```ignore
/// #[hopper_program]
/// mod vault {
///     pub fn deposit(ctx: &mut Context, amount: u64) -> ProgramResult { ... }
///     pub fn withdraw(ctx: &mut Context, amount: u64) -> ProgramResult { ... }
/// }
///
/// // Generated:
/// // pub fn __hopper_dispatch(program_id, accounts, data) -> ProgramResult { ... }
/// ```
#[proc_macro_attribute]
pub fn hopper_program(attr: TokenStream, item: TokenStream) -> TokenStream {
    program::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}
