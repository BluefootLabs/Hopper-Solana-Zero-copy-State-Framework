//! Hopper Native -- sovereign raw backend for Solana.
//!
//! Direct syscall-native runtime layer purpose-built for zero-copy state
//! frameworks. A sovereign substrate with genuinely novel features no other
//! framework provides:
//!
//! - **Alignment-safe wire types**: `LeU64`, `LeU32`, `LeBool` etc. --
//!   alignment-1 types with checked arithmetic by default, explicit
//!   endianness, const constructors. The foundation for safe zero-copy
//!   structs. (`wire`)
//! - **Verified CPI**: `LamportSnapshot`, `DataFingerprint` -- snapshot
//!   state before CPI, verify post-conditions after. First framework to
//!   provide substrate-level CPI result verification. (`verify`)
//! - **Cross-program lenses**: `read_address()`, `read_le_u64()` -- read
//!   specific fields from foreign program accounts by byte offset without
//!   importing their types at compile time. (`lens`)
//! - **Instruction introspection**: `is_cpi()`, `require_top_level()`,
//!   `require_ed25519_instruction()` -- CPI guard and precompile
//!   signature verification patterns. (`introspect`)
//! - **SVM-optimized memory**: `memcpy`, `memset`, `memcmp` -- dispatch
//!   to the VM's JIT-compiled intrinsics instead of Rust's libc. (`mem`)
//! - **Lazy account parsing**: `LazyContext` -- dispatch on instruction
//!   data before touching any accounts, parse only what you need. (`lazy`)
//! - **Compile-time capability types**: `SignerView`, `WritableView`,
//!   `MutableView`, `OwnedView` -- prove account roles in the type system
//!   with zero runtime cost after boundary validation. (`capability`)
//! - **Zero-copy struct projection**: `project::<T>()` with bounds,
//!   alignment, and discriminator checks in one operation. (`project`)
//! - **CU budget tracking**: `CuBudget` snapshots and `cu_trace!` macro
//!   for structured profiling. (`budget`)
//! - **Hash syscall wrappers**: `sha256`, `keccak256` -- zero-alloc
//!   multi-part hashing via direct syscalls. (`hash`)
//! - **Typed CPI return data**: `invoke_and_read::<T>()` -- CPI +
//!   deserialization in one step. (`return_data`)
//! - **Chainable validation**: `account.check_signer()?.check_writable()?`
//!   -- Steel-inspired fluent validation, improved and built in. (`account_view`)
//! - **Packed flags**: `account.flags()`, `account.expect_flags(SIGNER|WRITABLE)`
//!   -- check multiple account properties in a single comparison. (`account_view`)
//! - **Full sysvar access**: Clock, Rent, EpochSchedule with computed
//!   helpers. (`sysvar`)
//! - **Batch operations**: `close_and_transfer`, `realloc_checked`,
//!   `require_account_type` with proper atomicity. (`batch`)
//!
//! `no_std`, `no_alloc`, zero external runtime dependencies.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

// ── Core modules (always available) ──────────────────────────────────

pub mod pod;
pub mod address;
pub mod error;
pub mod account_view;
pub mod raw_account;
pub mod raw_input;
pub mod borrow;
pub mod syscalls;
pub mod log;
pub mod entrypoint;
pub mod pda;

// ── Innovation modules ───────────────────────────────────────────────

pub mod wire;
pub mod verify;
pub mod lens;
pub mod introspect;
pub mod mem;
pub mod lazy;
pub mod capability;
/// Cross-program projection lens traits (`Projectable`, `SafeProjectable`).
///
/// **Tier-C escape hatch** per the Hopper Safety Audit. The module
/// stays compiled because other low-level helpers (wire overlays,
/// typed return-data, the `expert` tier) use `Projectable` internally,
/// but its public re-export is gated behind the default-on
/// `legacy-projectable` feature. New code should prefer `Pod`-bounded
/// helpers (`lens::read_field_pod`, the `ZeroCopy` trait family in
/// `hopper-runtime`, `AccountView::segment_ref`/`segment_mut`).
#[doc(hidden)]
pub mod project;
pub mod budget;
pub mod hash;
pub mod return_data;
pub mod sysvar;
pub mod batch;

// ── Safety tier modules ──────────────────────────────────────────────

pub mod safe;
pub mod expert;
pub mod raw;

// ── CPI modules (feature-gated) ─────────────────────────────────────

#[cfg(feature = "cpi")]
pub mod instruction;
#[cfg(feature = "cpi")]
pub mod cpi;
#[cfg(feature = "cpi")]
pub mod system;
#[cfg(feature = "cpi")]
pub mod token;

// ── Re-exports ───────────────────────────────────────────────────────

pub use address::Address;
pub use error::ProgramError;
pub use account_view::AccountView;
pub use borrow::{Ref, RefMut};
pub use pod::Pod;

// Re-export bytemuck so downstream macros can reference it through
// the hopper dependency chain without every user adding bytemuck to
// their own Cargo.toml. `#[hopper::state]` / `#[hopper::pod]` emit
// `#[derive(::hopper::__runtime::__hopper_native::bytemuck::Pod, ...)]`
// which resolves here.
#[cfg(feature = "bytemuck")]
#[doc(hidden)]
pub use bytemuck;
pub use raw_account::RuntimeAccount;

// Innovation re-exports.
pub use lazy::LazyContext;
pub use capability::{SignerView, WritableView, MutableView, OwnedView, ReadonlyView, ExecutableView};
#[cfg(feature = "legacy-projectable")]
pub use project::Projectable;
pub use budget::CuBudget;
pub use return_data::ReturnData;
pub use verify::{LamportSnapshot, BalanceSnapshot, DataFingerprint};
pub use wire::{LeU64, LeU32, LeU16, LeI64, LeI32, LeI16, LeBool, LeU128};
pub use pda::verify_pda_strict;
pub use pda::{find_bump_for_address, read_bump_from_account, verify_pda_from_stored_bump};

/// Result type for Solana program instructions.
pub type ProgramResult = core::result::Result<(), ProgramError>;

/// Maximum number of accounts in a single transaction.
pub const MAX_TX_ACCOUNTS: usize = 254;

/// Success return code for the BPF entrypoint.
pub const SUCCESS: u64 = 0;

/// Maximum permitted data increase during realloc (10 KiB).
pub const MAX_PERMITTED_DATA_INCREASE: usize = 10_240;

/// Borrow state value indicating the account is not currently borrowed.
pub const NOT_BORROWED: u8 = u8::MAX;

// ── Convenience re-exports ───────────────────────────────────────────

#[cfg(feature = "cpi")]
pub use instruction::{InstructionView, InstructionAccount, Seed, Signer, CpiAccount};
