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
    invoke_context::{BuiltinFunctionWithContext, EnvironmentConfig, InvokeContext},
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

    /// Lower-level builtin registration. Wraps `function` in a
    /// fresh `ProgramCacheEntry::new_builtin` so callers can
    /// register a `BuiltinFunctionWithContext` without
    /// constructing the cache entry themselves.
    ///
    /// `account_size` is the byte size of the program's on-chain
    /// account (Agave's runtime reads it for compute-cost
    /// metering); a typical builtin program account is 4 KiB
    /// stub-shaped, matching mainnet.
    pub fn add_builtin_function(
        &self,
        id: Pubkey,
        account_size: usize,
        function: BuiltinFunctionWithContext,
    ) {
        let entry =
            Arc::new(ProgramCacheEntry::new_builtin(0, account_size, function));
        self.add_builtin(id, entry);
    }

    /// Register Agave's real System Program processor. After
    /// this call, instructions targeting `solana_sdk::system_program::id()`
    /// dispatch through `solana_system_program::system_processor`'s
    /// canonical implementation. This is the headline "match
    /// mainnet semantics" win Tier 3 unlocks.
    pub fn install_system_program(&self) {
        self.add_builtin_function(
            solana_sdk::system_program::id(),
            14, // account_size: matches solana-runtime's BUILTIN_PROGRAM_DESCRIPTORS for system
            solana_system_program::system_processor::Entrypoint::vm,
        );
    }

    /// Register a pre-built `ProgramCacheEntry`. Lower-level than
    /// [`load_bpf_program`]: callers that already have a built
    /// entry (because they're sharing it across engines, or
    /// stitching tests together) drop it in directly.
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

    /// Load a BPF `.so` program from raw bytes through Agave's
    /// real ELF loader. Wraps
    /// [`solana_bpf_loader_program::load_program_from_bytes`] with
    /// a freshly-built program-runtime environment for the harness's
    /// feature set + execution budget.
    ///
    /// `loader_key` selects between Loader v2 (legacy, where program
    /// data lives in the program account itself) and Loader v3
    /// (upgradeable, where data lives in a separate ProgramData
    /// account). The two map to `solana_sdk::bpf_loader::id()` and
    /// `solana_sdk::bpf_loader_upgradeable::id()` respectively.
    /// Use [`AgaveProgramKind::BpfV2`] / [`AgaveProgramKind::BpfV3`]
    /// helpers for the canonical pair.
    ///
    /// `account_size` is the byte size of the on-chain program
    /// account. For test fixtures, the ELF length itself is fine
    /// (the runtime uses this for compute-cost metering only).
    ///
    /// Returns `Err(InstructionError::InvalidAccountData)` when the
    /// ELF fails to parse or fails verification (the same error
    /// path Agave's deploy ix produces). Successful loads land in
    /// the engine's `program_cache` keyed by `id`.
    pub fn load_bpf_program(
        &self,
        id: Pubkey,
        kind: AgaveProgramKind,
        loader_key: &Pubkey,
        elf: &[u8],
        account_size: usize,
    ) -> Result<(), AgaveEngineError> {
        let exec_budget = SVMTransactionExecutionBudget::default();
        let runtime_env =
            solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1(
                &self.feature_set,
                &exec_budget,
                /* reject_deployment_of_broken_elfs */ false,
                /* debugging_features */ false,
            )
            .map_err(|err| {
                AgaveEngineError::Instruction(
                    solana_sdk::instruction::InstructionError::Custom(0xE000),
                )
                .with_context(format!("create_program_runtime_environment_v1 failed: {err}"))
            })?;

        let mut metrics = solana_program_runtime::loaded_programs::LoadProgramMetrics::default();
        let entry = solana_bpf_loader_program::load_program_from_bytes(
            None,
            &mut metrics,
            elf,
            loader_key,
            account_size,
            /* deployment_slot */ 0,
            Arc::new(runtime_env),
            /* reloading */ false,
        )
        .map_err(AgaveEngineError::Instruction)?;

        self.program_cache
            .write()
            .expect("program_cache write")
            .replenish(id, Arc::new(entry));
        self.kinds.write().expect("kinds write").insert(id, kind);
        Ok(())
    }

    /// Look up the registration kind for a program id.
    pub fn kind_for(&self, id: &Pubkey) -> Option<AgaveProgramKind> {
        self.kinds.read().expect("kinds read").get(id).copied()
    }

    /// Build an Agave [`SysvarCache`] from Hopper's sysvar shape.
    /// Hopper's `Clock` / `Rent` / etc. are local types that mirror
    /// the solana-sdk wire shape but are nominally distinct. Convert
    /// each to its solana-sdk twin before stashing in the cache.
    pub fn build_sysvar_cache(svm_sysvars: &crate::sysvar::Sysvars) -> SysvarCache {
        let mut cache = SysvarCache::default();

        let clock = solana_sdk::clock::Clock {
            slot: svm_sysvars.clock.slot,
            epoch_start_timestamp: svm_sysvars.clock.epoch_start_timestamp,
            epoch: svm_sysvars.clock.epoch,
            leader_schedule_epoch: svm_sysvars.clock.leader_schedule_epoch,
            unix_timestamp: svm_sysvars.clock.unix_timestamp,
        };
        cache.set_sysvar_for_tests(&clock);

        let rent = solana_sdk::rent::Rent {
            lamports_per_byte_year: svm_sysvars.rent.lamports_per_byte_year,
            exemption_threshold: svm_sysvars.rent.exemption_threshold,
            burn_percent: svm_sysvars.rent.burn_percent,
        };
        cache.set_sysvar_for_tests(&rent);

        let epoch_schedule = solana_sdk::epoch_schedule::EpochSchedule {
            slots_per_epoch: svm_sysvars.epoch_schedule.slots_per_epoch,
            leader_schedule_slot_offset: svm_sysvars
                .epoch_schedule
                .leader_schedule_slot_offset,
            warmup: svm_sysvars.epoch_schedule.warmup,
            first_normal_epoch: svm_sysvars.epoch_schedule.first_normal_epoch,
            first_normal_slot: svm_sysvars.epoch_schedule.first_normal_slot,
        };
        cache.set_sysvar_for_tests(&epoch_schedule);

        let last_restart_slot = solana_sdk::sysvar::last_restart_slot::LastRestartSlot {
            last_restart_slot: svm_sysvars.last_restart_slot.last_restart_slot,
        };
        cache.set_sysvar_for_tests(&last_restart_slot);

        let epoch_rewards = solana_sdk::sysvar::epoch_rewards::EpochRewards {
            distribution_starting_block_height: svm_sysvars
                .epoch_rewards
                .distribution_starting_block_height,
            num_partitions: svm_sysvars.epoch_rewards.num_partitions,
            parent_blockhash: solana_sdk::hash::Hash::new_from_array(
                svm_sysvars.epoch_rewards.parent_blockhash,
            ),
            total_points: svm_sysvars.epoch_rewards.total_points,
            total_rewards: svm_sysvars.epoch_rewards.total_rewards,
            distributed_rewards: svm_sysvars.epoch_rewards.distributed_rewards,
            active: svm_sysvars.epoch_rewards.active,
        };
        cache.set_sysvar_for_tests(&epoch_rewards);

        cache
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

/// Errors surfaced by [`AgaveEngine::process_instruction_raw`] and
/// [`AgaveEngine::load_bpf_program`].
#[derive(Debug)]
pub enum AgaveEngineError {
    /// The caller passed an instruction referencing an account
    /// that did not appear in the transaction's account vector.
    UnknownAccount(Pubkey),
    /// The Agave runtime rejected the instruction.
    Instruction(solana_sdk::instruction::InstructionError),
    /// A non-instruction failure with attached context (typically
    /// from the BPF loader path).
    Context(String),
}

impl AgaveEngineError {
    /// Wrap `self` with an extra human-readable explanation. Used by
    /// the loader path to attach the upstream error message that
    /// `InstructionError::Custom` alone would lose.
    pub fn with_context(self, ctx: impl Into<String>) -> Self {
        match self {
            Self::Context(existing) => Self::Context(format!("{ctx}: {existing}", ctx = ctx.into())),
            other => Self::Context(format!("{}: {other}", ctx.into())),
        }
    }
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
            Self::Context(msg) => write!(f, "agave-engine: {msg}"),
        }
    }
}

impl std::error::Error for AgaveEngineError {}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::{
        account::{Account, ReadableAccount},
        system_instruction,
    };

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

    /// `install_system_program` registers the system program at the
    /// canonical id. The kind tracker reports it as a builtin.
    #[test]
    fn install_system_program_registers_builtin() {
        let eng = AgaveEngine::new();
        eng.install_system_program();
        assert!(eng.is_registered(&solana_sdk::system_program::id()));
        assert_eq!(
            eng.kind_for(&solana_sdk::system_program::id()),
            Some(AgaveProgramKind::Builtin),
        );
    }

    /// `load_bpf_program` against malformed bytes surfaces the
    /// `Instruction(InvalidAccountData)` Agave's loader produces.
    /// Smoke-tests the load path: the runtime environment is built,
    /// `solana-bpf-loader-program::load_program_from_bytes` runs,
    /// the bytes fail verification, the error is propagated.
    /// Real-ELF coverage lives in the SPL Token CPI integration
    /// test (`tests/agave_spl_token_cpi.rs`) which is gated on a
    /// caller-supplied `.so` (see `crates/hopper-svm/programs/README.md`).
    #[test]
    fn load_bpf_program_rejects_malformed_bytes() {
        let eng = AgaveEngine::new();
        let id = Pubkey::new_unique();
        let garbage = vec![0u8; 64];
        let err = eng
            .load_bpf_program(
                id,
                AgaveProgramKind::BpfV2,
                &solana_sdk::bpf_loader::id(),
                &garbage,
                garbage.len(),
            )
            .expect_err("garbage bytes must not load");
        // Loader maps every parse / verify failure to `InvalidAccountData`.
        match err {
            AgaveEngineError::Instruction(_) => {}
            other => panic!("expected InstructionError, got: {other}"),
        }
        assert!(!eng.is_registered(&id));
    }

    /// **Headline Tier 3 test**: a system-program transfer dispatched
    /// through Agave's real runtime. Pre-state has alice with
    /// 1_000_000 lamports and bob with 0; post-state has 750k / 250k
    /// after a `system_instruction::transfer`. This proves the full
    /// Agave path runs end-to-end: `InvokeContext::process_instruction`
    /// drives `solana_system_program::system_processor::Entrypoint`
    /// against a real `TransactionContext` and account state flows
    /// back through the `AccountSharedData` shape.
    #[test]
    fn system_transfer_through_agave_runtime() {
        let eng = AgaveEngine::new();
        eng.install_system_program();
        let svm_sysvars = crate::sysvar::Sysvars::default();
        let sysvar_cache = AgaveEngine::build_sysvar_cache(&svm_sysvars);

        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let system_id = solana_sdk::system_program::id();

        // Account vector. The system program account is a placeholder;
        // its bytes are not read by the builtin path (the runtime
        // routes through the cached `BuiltinFunctionWithContext`),
        // but it has to exist in the transaction's account list at
        // a known index so `program_indices` can refer to it.
        let mut alice_acct = Account::default();
        alice_acct.lamports = 1_000_000;
        alice_acct.owner = system_id;

        let mut bob_acct = Account::default();
        bob_acct.lamports = 0;
        bob_acct.owner = system_id;

        let mut sys_acct = Account::default();
        sys_acct.executable = true;
        sys_acct.owner = solana_sdk::native_loader::id();

        let accounts = vec![
            (alice, alice_acct.into()),
            (bob, bob_acct.into()),
            (system_id, sys_acct.into()),
        ];
        let program_indices = vec![2u16];

        let ix = system_instruction::transfer(&alice, &bob, 250_000);

        let (cu, post) = eng
            .process_instruction_raw(
                &ix,
                accounts,
                program_indices,
                &sysvar_cache,
                SVMTransactionExecutionBudget::default(),
                SVMTransactionExecutionCost::default(),
                solana_sdk::rent::Rent::default(),
            )
            .expect("agave system transfer ok");

        // System program declares `DEFAULT_COMPUTE_UNITS = 150`.
        assert!(cu >= 150, "expected >= 150 CU consumed, got {cu}");

        let alice_post = post
            .iter()
            .find(|(k, _)| k == &alice)
            .map(|(_, a)| a)
            .expect("alice in post-state");
        let bob_post = post
            .iter()
            .find(|(k, _)| k == &bob)
            .map(|(_, a)| a)
            .expect("bob in post-state");
        assert_eq!(alice_post.lamports(), 750_000);
        assert_eq!(bob_post.lamports(), 250_000);
    }
}
