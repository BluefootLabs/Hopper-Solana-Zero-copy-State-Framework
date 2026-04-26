//! Phase 2: real `.so` execution via `solana-sbpf`.
//!
//! This module is **feature-gated** behind the `bpf-execution`
//! feature. Default Hopper SVM users get the Phase 1 built-in
//! execution path; Phase 2 activates when callers explicitly opt
//! in:
//!
//! ```toml
//! [dev-dependencies]
//! hopper-svm = { workspace = true, features = ["bpf-execution"] }
//! ```
//!
//! ## Module map
//!
//! - [`parameter`] — serialise account state + ix data + program ID
//!   into the canonical Solana parameter buffer the BPF program
//!   reads through `solana_program::entrypoint::deserialize`, and
//!   read the post-state back out after the VM returns.
//! - [`context`] — `BpfContext` impls `solana_sbpf::vm::ContextObject`,
//!   tying the harness compute meter and log buffer into the VM's
//!   tracing/metering hooks.
//! - [`syscalls`] — pure-Rust syscall logic functions
//!   (`do_sol_log`, `do_sol_memcpy`, …).
//! - [`adapters`] — `declare_builtin_function!` adapters that
//!   bridge the pure-Rust logic into `solana-sbpf`'s
//!   `BuiltinFunction` shape. The only file that touches the sbpf
//!   macro/translation API surface, so any drift between sbpf
//!   minor versions concentrates here.
//! - [`engine`] — `BpfEngine` for ELF loading + memory regions +
//!   VM lifecycle + execute + parameter-buffer round-trip. The
//!   harness's [`crate::HopperSvm::dispatch_one`] falls through to
//!   it after the built-in registry misses.
//!
//! ## Syscall surface in 2.1
//!
//! Adds PDA derivation: `sol_create_program_address`,
//! `sol_try_find_program_address`. Phase 2.0's syscall set
//! (`sol_log_*`, `sol_mem*`, `sol_panic_`, return-data) keeps
//! working unchanged. Sysvars, heap alloc, crypto, and CPI ship
//! in subsequent passes.

pub mod adapters;
pub mod context;
pub mod cpi;
pub mod cpi_rust;
pub mod crypto_syscalls;
pub mod engine;
pub mod parameter;
pub mod syscalls;
pub mod sysvar_syscalls;
pub mod tier3_syscalls;

pub use engine::BpfEngine;
