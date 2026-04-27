//! Agave-runtime execution engine (Tier 3 / `agave-runtime` feature).
//!
//! Replaces the direct `solana-sbpf` invocation in
//! [`crate::bpf::engine`] with the real validator stack:
//! `solana-program-runtime`, `solana-bpf-loader-program`,
//! `solana-compute-budget`. Syscall behaviour, CPI dispatch,
//! sysvar handling, and account input/output serialization match
//! mainnet exactly because they ARE the mainnet code path.
//!
//! ## Design
//!
//! The engine wraps three pieces of state:
//!
//! 1. **`ProgramCacheEntry` registry** — keyed by `Pubkey`, owns
//!    the loaded ELF + jit-compiled bytecode for each registered
//!    program. Built from `solana_bpf_loader_program::load_program_from_bytes`.
//! 2. **`SysvarCache`** — populated from the harness's [`crate::sysvar::Sysvars`]
//!    via `fill_missing_entries`-style copy.
//! 3. **`ComputeBudget`** — sourced from the harness's per-call
//!    budget; `solana-compute-budget`'s shape so it slots into
//!    `InvokeContext` directly.
//!
//! Each `process_instruction` call:
//!
//! - Builds a fresh `TransactionContext` from the supplied
//!   [`crate::account::KeyedAccount`] slice.
//! - Constructs an `InvokeContext` over that context with the
//!   harness's program cache + sysvar cache + compute budget.
//! - Invokes via `InvokeContext::process_instruction`.
//! - Reads back the post-state accounts from the transaction
//!   context, translates them into [`crate::account::KeyedAccount`],
//!   and ships them through the standard
//!   [`crate::engine::ExecutionOutcome`].
//!
//! The harness exposes the engine through
//! [`crate::HopperSvm::with_agave_runtime`], a builder verb that
//! flips the dispatch path from the inline `BuiltinProgram`
//! registry + `solana-sbpf` direct interpreter to this Agave path.
//! Both engines stay available so a single test binary can compare
//! results between them.

#![cfg(feature = "agave-runtime")]

mod engine;

pub use engine::{AgaveEngine, AgaveEngineError, AgaveProgramKind};
