//! Optional proc macro DX layer for Hopper.
//!
//! Provides both the canonical `#[hopper::state]`, `#[hopper::context]`,
//! `#[hopper::program]` surface and the legacy `#[hopper_state]`,
//! `#[hopper_context]`, `#[hopper_program]` aliases. All entry points generate
//! zero-cost code targeting Hopper's runtime primitives.
//!
//! **Not required.** Every feature these macros provide is achievable through
//! Hopper's declarative `macro_rules!` macros or hand-written code. These
//! exist purely for developer velocity. The generated code compiles to the
//! exact same pointer arithmetic as raw Pinocchio.
//!
//! # Design Philosophy
//!
//! - **Macros generate code, not behavior.** No hidden runtime logic.
//! - **Everything inlines.** No function calls that wouldn't exist in hand-written code.
//! - **No heap.** Generated code is `no_std`, `no_alloc`.
//! - **Optional.** Core Hopper never depends on this crate.

extern crate proc_macro;

mod crank;
mod declare_program;
mod migrate;
mod pod;
mod state;
mod context;
mod program;
mod event;
mod error;
mod args;
mod dynamic;

use proc_macro::TokenStream;

/// Generate a `SegmentMap` implementation for a zero-copy layout struct.
///
/// Computes field offsets at compile time and emits a const segment table.
/// The generated code is zero-cost. Segment lookups resolve to const loads.
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

#[proc_macro_attribute]
pub fn state(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_state(attr, item)
}

/// Anchor / Quasar naming alias for [`state`].
///
/// Declare a zero-copy account layout with the familiar `#[account]`
/// spelling. Functionally identical to `#[hopper::state]`. The same
/// `#[account(mut(...))]` field-attribute syntax inside a
/// `#[hopper::context]` keeps working because field-level attrs are
/// consumed by the outer macro, not by this proc macro.
#[proc_macro_attribute]
pub fn account(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_state(attr, item)
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

#[proc_macro_attribute]
pub fn context(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_context(attr, item)
}

/// Anchor-style plural alias for [`context`].
///
/// Anchor writes `#[derive(Accounts)]` on the accounts struct. Hopper
/// uses attribute macros instead of derive macros, so this is the
/// closest naturally-spelled alias: `#[accounts]`. Functionally
/// identical to `#[hopper::context]`. Pick whichever spelling reads
/// best to the rest of your codebase.
#[proc_macro_attribute]
pub fn accounts(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_context(attr, item)
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

#[proc_macro_attribute]
pub fn program(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_program(attr, item)
}

/// Derive the Hopper zero-copy marker contract for a user-defined struct.
///
/// Unlike `#[hopper::state]` (which emits the full Hopper layout: 16-byte
/// header, layout_id, schema export, typed load helpers), `#[hopper::pod]`
/// is the minimal opt-in: it asserts that the struct satisfies the
/// Pod + FixedLayout + alignment-1 + non-padded + non-zero-sized contract
/// at compile time, and emits the matching `unsafe impl Pod` and
/// `impl FixedLayout` so it can participate in every Hopper segment /
/// raw access API.
///
/// This is the Hopper Safety Audit's "derive macros for Pod and layout"
/// recommendation delivered standalone: use it on sub-structs, wire
/// helpers, or any `#[repr(C)]` overlay that isn't a full top-level
/// account layout.
///
/// # Example
///
/// ```ignore
/// #[hopper::pod]
/// #[repr(C)]
/// pub struct Cursor {
///     pub head: WireU64,
///     pub tail: WireU64,
///     pub capacity: WireU64,
/// }
///
/// // Now usable as:
/// let c: Ref<'_, Cursor> = account.segment_ref::<Cursor>(&mut borrows, 0, 24)?;
/// ```
#[proc_macro_attribute]
pub fn hopper_pod(attr: TokenStream, item: TokenStream) -> TokenStream {
    pod::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Short alias: `#[hopper::pod]`. Functionally identical to `#[hopper_pod]`.
#[proc_macro_attribute]
pub fn pod(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_pod(attr, item)
}

/// Declare a schema-epoch migration edge.
///
/// Decorates a function of signature
/// `fn(&mut [u8]) -> Result<(), ProgramError>` that mutates an
/// account body in-place from schema epoch `from` to epoch `to`.
/// The macro emits the fn unchanged plus a paired
/// `<FN_NAME>_EDGE: hopper_runtime::MigrationEdge` constant so the
/// layout author can compose edges via `hopper::layout_migrations!`.
///
/// Closes Hopper Safety Audit innovation I4 ("Schema epoch with
/// in-place migration helpers"). Runtime chain application and
/// atomic-per-edge `schema_epoch` bump live in
/// `hopper_runtime::migrate`.
///
/// # Example
///
/// ```ignore
/// #[hopper::migrate(from = 1, to = 2)]
/// pub fn vault_v1_to_v2(body: &mut [u8]) -> ProgramResult {
///     // Reinterpret bytes to match the epoch-2 shape.
///     Ok(())
/// }
/// ```
#[proc_macro_attribute]
pub fn hopper_migrate(attr: TokenStream, item: TokenStream) -> TokenStream {
    migrate::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Short alias: `#[hopper::migrate]`. Functionally identical to
/// `#[hopper_migrate]`.
#[proc_macro_attribute]
pub fn migrate(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_migrate(attr, item)
}

// -----------------------------------------------------------------------------
// Newly added derives (added alongside the existing surface, not replacing it):
//   - `#[hopper::event]`. segment-tagged events with a stable tag byte.
//   - `#[hopper::error]`. error codes linked to invariant IDs.
//   - `#[hopper::args]`. borrowing zero-copy instruction argument parser.
//   - `#[hopper::dynamic]`- field-level dynamic tail opt-in.
// -----------------------------------------------------------------------------

/// Derive a Hopper event: emits a stable 1-byte tag, optional segment source,
/// a `NAME` string, a `FIELD_COUNT` const, and an `as_bytes(&self)` view for
/// the framework's log-emission pathway.
///
/// # Example
/// ```ignore
/// #[hopper::event(tag = 7, segment = 1)]
/// #[repr(C)]
/// pub struct Deposited {
///     pub amount: [u8; 8],
///     pub depositor: [u8; 32],
/// }
/// ```
#[proc_macro_attribute]
pub fn hopper_event(attr: TokenStream, item: TokenStream) -> TokenStream {
    event::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Short alias: `#[hopper::event]`.
#[proc_macro_attribute]
pub fn event(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_event(attr, item)
}

/// Mark an instruction handler as an autonomous crank.
///
/// Attaches a `"Crank"` capability tag to the instruction descriptor
/// in the program manifest and optionally captures
/// `seeds(account_name = [...])` hints so a keeper-bot CLI can
/// resolve every PDA without per-program config.
///
/// Cranks must be zero-arg handlers. Any value argument is a
/// compile-time error, because the crank runner cannot invent
/// instruction data on behalf of the caller.
#[proc_macro_attribute]
pub fn hopper_crank(attr: TokenStream, item: TokenStream) -> TokenStream {
    crank::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Short alias: `#[hopper::crank]`.
#[proc_macro_attribute]
pub fn crank(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_crank(attr, item)
}

/// Generate a typed CPI surface from an on-disk Hopper manifest.
///
/// ```ignore
/// hopper::declare_program!(amm, "idl/amm.json");
/// ```
///
/// Emits a module with `PROGRAM_NAME`, `PROGRAM_ID_STR`, a
/// `FINGERPRINT: [u8; 32]` compile-time manifest-hash const, and one
/// builder per instruction. See the declare_program module for the
/// full contract.
#[proc_macro]
pub fn declare_program(input: TokenStream) -> TokenStream {
    declare_program::expand(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Derive a Hopper error-code enum. Emits `code()`, `variant_name()`,
/// `From<T> for u32`, and two const tables (`CODE_TABLE`, `INVARIANT_TABLE`)
/// that the schema crate surfaces in the manifest.
///
/// Per-variant `#[invariant = "name"]` attributes are the innovation: when
/// a runtime invariant check fails, the corresponding error carries the
/// invariant name, and the off-chain SDK can render "Invariant `x` failed"
/// instead of an opaque hex code.
///
/// # Example
/// ```ignore
/// #[hopper::error]
/// #[repr(u32)]
/// pub enum VaultError {
///     #[invariant = "balance_nonzero"]
///     InsufficientBalance = 0x1001,
///     MigrationRequired,   // auto-assigned stable code
/// }
/// ```
#[proc_macro_attribute]
pub fn hopper_error(attr: TokenStream, item: TokenStream) -> TokenStream {
    error::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Short alias: `#[hopper::error]`.
#[proc_macro_attribute]
pub fn error(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_error(attr, item)
}

/// Derive a zero-copy borrowing parser for an instruction argument struct.
///
/// Emits `parse(&[u8]) -> Result<&Self, ArgParseError>`, `PACKED_SIZE`,
/// `ARG_DESCRIPTORS`, and `CU_HINT`. The `cu` attribute lets a program
/// declare a compute-unit budget clients can inspect via the manifest before
/// submission.
///
/// # Example
/// ```ignore
/// #[hopper::args(cu = 1200)]
/// #[repr(C)]
/// pub struct DepositArgs {
///     pub amount: [u8; 8],
///     pub memo:   [u8; 16],
/// }
/// ```
#[proc_macro_attribute]
pub fn hopper_args(attr: TokenStream, item: TokenStream) -> TokenStream {
    args::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Short alias: `#[hopper::args]`.
#[proc_macro_attribute]
pub fn args(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_args(attr, item)
}

/// Declare which field of a `#[repr(C)]` struct is the dynamic-tail region.
///
/// Attaches to the **struct**, not the field, because stable Rust does not
/// permit `#[proc_macro_attribute]` macros on struct fields. The field name
/// is passed as a string via `field = "<name>"`.
///
/// # Example
///
/// ```ignore
/// #[hopper::dynamic(field = "entries")]
/// #[hopper::state]
/// #[repr(C)]
/// pub struct Ledger {
///     pub head: WireU64,
///     pub tail: WireU64,
///     pub entries: DynamicRegion<LedgerEntry>,
/// }
/// ```
#[proc_macro_attribute]
pub fn hopper_dynamic(attr: TokenStream, item: TokenStream) -> TokenStream {
    dynamic::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Short alias: `#[hopper::dynamic(field = "…")]`.
#[proc_macro_attribute]
pub fn dynamic(attr: TokenStream, item: TokenStream) -> TokenStream {
    hopper_dynamic(attr, item)
}
