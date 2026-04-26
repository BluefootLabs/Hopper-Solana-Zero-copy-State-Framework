//! Bundled SPL program simulators — `BuiltinProgram` impls of
//! the most-used SPL programs, registered against their canonical
//! IDs via builder methods on [`crate::HopperSvm`].
//!
//! ## Why simulators (Phase 1 path) and not bundled `.so`?
//!
//! Three reasons:
//!
//! 1. **Version stability.** Bundled `.so` files need to be
//!    re-vendored on every Anza release. A pure-Rust simulator
//!    is a single source-of-truth that updates with our normal
//!    semver cycle.
//! 2. **Speed.** Phase 1 builtin dispatch is 10-100× faster
//!    than going through the BPF interpreter for the same
//!    instruction. Token transfers in tests run essentially
//!    free.
//! 3. **Hopper-owned.** Every layer of `hopper-svm` is hand-
//!    written Rust we can audit. Embedding third-party `.so`
//!    bytes would be an opaque trust dependency.
//!
//! ## Coverage
//!
//! - [`token`] — SPL Token. Covers the 8 most-used
//!   instructions: `InitializeMint` (0), `InitializeAccount`
//!   (1), `Transfer` (3), `Approve` (4), `Revoke` (5),
//!   `MintTo` (7), `Burn` (8), `CloseAccount` (9). Validation
//!   matches `spl_token::processor` end-to-end against the
//!   wire format.
//! - [`token_2022`] — SPL Token-2022. Delegates to `token` for
//!   the legacy overlap; surfaces a clear error for the
//!   extension surface (which is out of scope for Phase 1).
//! - [`ata`] — Associated Token Account program. Handles
//!   `Create` and `CreateIdempotent`.
//! - [`alt_program`] — Address Lookup Table program. Handles
//!   `Create`, `Freeze`, `Extend`, `Deactivate`, `Close`.
//! - [`config_program`] — Config program. Single `Store`
//!   instruction.
//! - [`stake_program`] — Stake program. Lifecycle slice —
//!   `Initialize`, `Authorize`, `DelegateStake`, `Withdraw`,
//!   `Deactivate`.
//! - [`vote_program`] — Vote program. Administrative slice —
//!   `InitializeAccount`, `Authorize`, `Withdraw`,
//!   `UpdateValidatorIdentity`, `UpdateCommission`.
//!
//! Programs that need surface beyond the slice can either fall
//! back to BPF by registering the real `.so` via
//! [`crate::HopperSvm::add_program`], or open a feature
//! request — Hopper accepts contributions for the niche
//! variants.

pub mod alt_program;
pub mod ata;
pub mod config_program;
pub mod stake_program;
pub mod token;
pub mod token_2022;
pub mod vote_program;
