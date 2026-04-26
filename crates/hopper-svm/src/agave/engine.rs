//! Agave-runtime execution engine.
//!
//! See module docs in [`super`] for the architectural overview.
//! This file owns:
//!
//! - [`AgaveEngine`] — the harness-side wrapper around Agave's
//!   `InvokeContext` / `TransactionContext` / `ProgramCacheForTxBatch`.
//! - [`AgaveProgramKind`] — distinguishes built-in (host-Rust)
//!   programs from BPF (`.so`) programs at registration time.
//!
//! The engine is constructed empty (no programs) and built up via
//! [`AgaveEngine::add_builtin`] for built-in programs (e.g. the
//! system program, the Address Lookup Table program) and
//! [`AgaveEngine::add_bpf_program`] for `.so` artifacts loaded by
//! `solana-bpf-loader-program`.

use solana_program_runtime::{
    execution_budget::{SVMTransactionExecutionBudget, SVMTransactionExecutionCost},
    invoke_context::{EnvironmentConfig, InvokeContext},
    loaded_programs::{
        BlockRelation, ForkGraph, ProgramCacheEntry, ProgramCacheForTxBatch,
        ProgramRuntimeEnvironments,
    },
    sysvar_cache::SysvarCache,
};
use solana_sdk::{
    account::AccountSharedData,
    hash::Hash,
    instruction::Instruction,
    pubkey::Pubkey,
    slot_history::Slot,
    transaction_context::{InstructionAccount, TransactionContext},
};
use solana_svm_callback::InvokeContextCallback;
use solana_svm_feature_set::SVMFeatureSet;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Distinguishes how a registered program is dispatched.
///
/// Built-ins are host-side Rust closures Agave's runtime recognises
/// directly. BPF programs are `.so` ELFs loaded through the
/// canonical `solana-bpf-loader-program` path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgaveProgramKind {
    /// Host-Rust builtin (system, ALT, BPF loader, etc.).
    Builtin,
    /// `.so` BPF program loaded via Loader v2.
    BpfV2,
    /// `.so` BPF program loaded via Loader v3 (upgradeable).
    BpfV3,
}

/// No-op fork graph. The harness has no fork-tracking concept; every
/// program lives in the single shipping cache. Implementing the
/// trait is mandatory for `ProgramCacheForTxBatch` plumbing.
#[derive(Default)]
struct UnitForkGraph;

impl ForkGraph for UnitForkGraph {
    fn relationship(&self, _a: Slot, _b: Slot) -> BlockRelation {
        // The harness runs in a single, monotonic slot timeline.
        // Every slot is an ancestor of every later slot.
        BlockRelation::Equal
    }
}

/// No-op `InvokeContextCallback`. The harness doesn't model
/// epoch-stake or vote-account state; the trait's defaults
/// already return zero/empty for those queries, so an empty
/// implementation block is enough.
#[derive(Default)]
struct NoOpCallback;

impl InvokeContextCallback for NoOpCallback {}

/// Hopper's Agave-runtime engine. Holds the program cache, feature
/// set, and shared fork graph the runtime needs to dispatch a
/// real instruction.
///
/// Constructed via [`AgaveEngine::new`]. Built-in programs are
/// installed with [`AgaveEngine::add_builtin`] and BPF programs
/// with [`AgaveEngine::add_bpf_program`]. The `process_instruction`
/// surface (Phase 2) consumes a `TransactionContext` worth of
/// accounts and dispatches via `InvokeContext::process_instruction`.
#[derive(Clone)]
pub struct AgaveEngine {
    /// Solana feature set in effect for executions. Defaults to
    /// `SVMFeatureSet::all_enabled()` so the harness behaves like
    /// mainnet at the latest activation horizon.
    pub feature_set: Arc<SVMFeatureSet>,
    /// Per-batch program cache. Built-ins land here directly;
    /// BPF programs land here after loading.
    pub program_cache: Arc<RwLock<ProgramCacheForTxBatch>>,
    /// Per-program-id metadata so the engine knows which
    /// dispatch path each registered program takes.
    pub kinds: Arc<RwLock<HashMap<Pubkey, AgaveProgramKind>>>,
    /// Fork graph the runtime requires. Single-timeline impl.
    fork_graph: Arc<RwLock<UnitForkGraph>>,
    /// Default lamports-per-signature when constructing an
    /// `EnvironmentConfig`. Mainnet baseline is 5000; tests can
    /// override via [`set_lamports_per_signature`].
    pub lamports_per_signature: u64,
}

impl AgaveEngine {
    /// Build a fresh engine. No programs are registered; callers
    /// add them via [`add_builtin`] / [`add_bpf_program`] before
    /// dispatching.
    pub fn new() -> Self {
        let cache = ProgramCacheForTxBatch::new(
            0,
            ProgramRuntimeEnvironments::default(),
            None,
            0,
        );
        Self {
            feature_set: Arc::new(SVMFeatureSet::all_enabled()),
            program_cache: Arc::new(RwLock::new(cache)),
            kinds: Arc::new(RwLock::new(HashMap::new())),
            fork_graph: Arc::new(RwLock::new(UnitForkGraph)),
            lamports_per_signature: 5_000,
        }
    }

    /// Override the harness's lamports-per-signature. Mirrors
    /// `HopperSvm::set_fee_calculator` at the engine level.
    pub fn set_lamports_per_signature(&mut self, n: u64) {
        self.lamports_per_signature = n;
    }

    /// Register a host-Rust builtin program. Stores the cache
    /// entry under `id` so subsequent `process_instruction` calls
    /// targeting `id` route through the builtin's invoke handler.
    pub fn add_builtin(&self, id: Pubkey, entry: Arc<ProgramCacheEntry>) {
        self.program_cache
            .write()
            .expect("program_cache write")
            .replenish(id, entry);
        self.kinds
            .write()
            .expect("kinds write")
            .insert(id, AgaveProgramKind::Builtin);
    }

    /// Register a BPF `.so` program. The bytes are loaded through
    /// `solana-bpf-loader-program`'s `load_program_from_bytes` (or
    /// equivalent) and the resulting `ProgramCacheEntry` is stored.
    /// Future Phase-2 hookup wires the loader call; today this is
    /// the registration point so the API surface is fixed.
    pub fn add_bpf_program(
        &self,
        id: Pubkey,
        kind: AgaveProgramKind,
        entry: Arc<ProgramCacheEntry>,
    ) {
        self.program_cache
            .write()
            .expect("program_cache write")
            .replenish(id, entry);
        self.kinds
            .write()
            .expect("kinds write")
            .insert(id, kind);
    }

    /// Look up the registration kind for a program id.
    pub fn kind_for(&self, id: &Pubkey) -> Option<AgaveProgramKind> {
        self.kinds.read().expect("kinds read").get(id).copied()
    }

    /// Whether any program has been registered against `id` (built-in or BPF).
    pub fn is_registered(&self, id: &Pubkey) -> bool {
        self.kinds.read().expect("kinds read").contains_key(id)
    }

    /// Build an [`InvokeContext`] over a fresh
    /// [`TransactionContext`] populated from
    /// `(address, AccountSharedData)` pairs. The caller drives the
    /// resulting context through `process_instruction` (Phase 2
    /// of this engine's bring-up will fold this into a single
    /// `process` verb on the engine itself).
    ///
    /// Returns the `TransactionContext` separately so the caller
    /// can read post-state out after invocation. The
    /// `program_cache` argument is borrowed mutably because Agave
    /// drains it of program entries on dispatch.
    pub fn build_transaction_context(
        &self,
        accounts: Vec<(Pubkey, AccountSharedData)>,
        rent: solana_sdk::rent::Rent,
        stack_capacity: usize,
    ) -> TransactionContext {
        TransactionContext::new(accounts, rent, stack_capacity, stack_capacity)
    }

    /// Construct the `EnvironmentConfig` Agave's `InvokeContext`
    /// requires. Uses the harness's feature set + lamports/sig +
    /// the supplied sysvar cache. The caller's `blockhash` is
    /// typically `Hash::default()` for a unit test; tests that
    /// pin specific blockhashes can pass a non-default value.
    pub fn make_environment_config<'a>(
        &'a self,
        sysvar_cache: &'a SysvarCache,
        blockhash: Hash,
        epoch_stake_callback: &'a dyn InvokeContextCallback,
    ) -> EnvironmentConfig<'a> {
        EnvironmentConfig::new(
            blockhash,
            self.lamports_per_signature,
            epoch_stake_callback,
            &self.feature_set,
            sysvar_cache,
        )
    }

    /// One-shot top-level instruction dispatch through the
    /// real Agave runtime. Constructs a `TransactionContext`
    /// from `accounts`, wraps it in an `InvokeContext`, runs
    /// `InvokeContext::process_instruction`, and returns the
    /// `(success, compute_units_consumed, post_accounts)` triple.
    ///
    /// `program_indices` is the list of indices into the supplied
    /// account vector identifying the program account(s) the
    /// instruction targets — Agave's runtime requires this so it
    /// can route the dispatch through the cache entry attached to
    /// that index.
    ///
    /// This is the Phase-2 entry point. The
    /// `_compute_budget` arg is wired through to `InvokeContext`'s
    /// budget at construction; the caller's harness-level
    /// `ComputeBudget` is translated into the runtime shape.
    #[allow(clippy::too_many_arguments)]
    pub fn process_instruction_raw(
        &self,
        ix: &Instruction,
        accounts: Vec<(Pubkey, AccountSharedData)>,
        program_indices: Vec<u16>,
        sysvar_cache: &SysvarCache,
        execution_budget: SVMTransactionExecutionBudget,
        execution_cost: SVMTransactionExecutionCost,
        rent: solana_sdk::rent::Rent,
    ) -> Result<(u64, Vec<(Pubkey, AccountSharedData)>), AgaveEngineError> {
        let mut tx_context = self.build_transaction_context(accounts, rent, 5);

        let mut program_cache = self
            .program_cache
            .read()
            .expect("program_cache read")
            .clone();

        let callback = NoOpCallback;
        let env_cfg =
            self.make_environment_config(sysvar_cache, Hash::default(), &callback);
        let mut ctx = InvokeContext::new(
            &mut tx_context,
            &mut program_cache,
            env_cfg,
            None,
            execution_budget,
            execution_cost,
        );

        // Build instruction_accounts: each meta entry plus its
        // index in the transaction's account list. Agave's runtime
        // expects de-duplicated entries when an account appears
        // multiple times in the meta list.
        let mut instruction_accounts = Vec::<InstructionAccount>::with_capacity(ix.accounts.len());
        for (i, meta) in ix.accounts.iter().enumerate() {
            let index_in_tx = ctx
                .transaction_context
                .find_index_of_account(&meta.pubkey)
                .ok_or_else(|| AgaveEngineError::UnknownAccount(meta.pubkey))?;
            instruction_accounts.push(InstructionAccount {
                index_in_transaction: index_in_tx,
                index_in_caller: index_in_tx,
                index_in_callee: i as u16,
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            });
        }

        let mut compute_units_consumed: u64 = 0;
        let mut timings = solana_timings::ExecuteTimings::default();
        ctx.process_instruction(
            &ix.data,
            &instruction_accounts,
            &program_indices,
            &mut compute_units_consumed,
            &mut timings,
        )
        .map_err(AgaveEngineError::Instruction)?;

        let post = (0..tx_context.get_number_of_accounts())
            .filter_map(|i| {
                let key = *tx_context.get_key_of_account_at_index(i).ok()?;
                let acct = tx_context.accounts().try_borrow(i).ok()?.clone();
                Some((key, acct))
            })
            .collect();
        Ok((compute_units_consumed, post))
    }
}

impl Default for AgaveEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors surfaced by [`AgaveEngine::process_instruction_raw`].
#[derive(Debug)]
pub enum AgaveEngineError {
    /// The caller passed an instruction referencing an account
    /// that did not appear in the transaction's account vector.
    UnknownAccount(Pubkey),
    /// The Agave runtime rejected the instruction.
    Instruction(solana_sdk::instruction::InstructionError),
}

impl core::fmt::Display for AgaveEngineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownAccount(pk) => {
                write!(f, "agave-engine: unknown account in instruction meta: {pk}")
            }
            Self::Instruction(err) => {
                write!(f, "agave-engine: instruction failed: {err:?}")
            }
        }
    }
}

impl std::error::Error for AgaveEngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: a fresh `AgaveEngine` constructs without panicking
    /// and reports no programs registered. Anchors the dep tree
    /// (every Agave crate the engine touches must link).
    #[test]
    fn fresh_engine_is_empty() {
        let eng = AgaveEngine::new();
        assert!(!eng.is_registered(&Pubkey::new_unique()));
        assert_eq!(eng.lamports_per_signature, 5_000);
        assert!(eng.feature_set.lift_cpi_caller_restriction);
    }

    /// `set_lamports_per_signature` mutates the field; smoke check.
    #[test]
    fn lamports_per_signature_overridable() {
        let mut eng = AgaveEngine::new();
        eng.set_lamports_per_signature(10_000);
        assert_eq!(eng.lamports_per_signature, 10_000);
    }
}
