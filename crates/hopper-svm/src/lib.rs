//! # `hopper-svm` — Hopper-native in-process Solana execution
//!
//! A **Hopper-owned** test harness. Every layer above the eBPF
//! interpreter — built-in program registry, syscall surface (Phase 2),
//! CPI dispatch (Phase 2), compute metering, log buffer, sysvar state,
//! account input/output serialization, and Hopper-aware result
//! decoding — is implemented here from scratch. There is no
//! `mollusk-svm` dependency, no `quasar-svm` dependency, no copy of
//! anyone else's design. The harness is shaped around how Hopper
//! programs actually want to be tested, with first-class hooks for
//! Hopper headers, layout fingerprints, segment maps, and receipts.
//!
//! ## Phase 1 (this release)
//!
//! Ships a complete, working **built-in program** execution path:
//!
//! - [`HopperSvm`] — harness with program registry, sysvars, compute
//!   budget, log buffer.
//! - [`builtin::BuiltinProgram`] trait — implement it on a Rust struct
//!   to register a built-in program. The system program is shipped as
//!   the first reference implementation
//!   ([`system_program::SystemProgram`]).
//! - [`HopperExecutionResult`] — result type with `assert_success`,
//!   `account()`, log capture, compute-units consumed, and Hopper-aware
//!   decoders ([`HopperExecutionResult::decode_header`],
//!   [`HopperExecutionResult::decoded_logs`]).
//! - [`token`] — same factory helpers as before
//!   (`create_keyed_system_account`, `create_keyed_mint_account`, …).
//!   Token-account *state construction* is fully native (we serialize
//!   the SPL wire shapes ourselves via `Pack`); running an SPL token
//!   *program* requires Phase 2.
//!
//! Phase 1 lets users write tests that exercise:
//!
//! - System program flows (transfers, account creation, allocation,
//!   reassignment of ownership) end-to-end.
//! - Custom built-in programs registered for unit testing — useful
//!   when a Hopper program has a small enough business core that a
//!   hand-written Rust simulator gives the same coverage as the
//!   compiled `.so` for unit tests.
//! - Anything that needs Hopper-aware decoding of post-state account
//!   bytes (the layout-aware decoders work the same whether the
//!   account state was produced by a built-in or a future BPF run).
//!
//! ## Phase 2 (planned)
//!
//! Wires [`solana-sbpf`] (Anza's canonical eBPF interpreter, the
//! foundation Mollusk and Agave both wrap) as the execution engine
//! for real `.so` files, plus the full Solana syscall surface
//! (`sol_log`, `sol_log_pubkey`, `sol_panic_`, `sol_memcpy_`,
//! `sol_memset_`, `sol_memcmp_`, `sol_memmove_`, `sol_alloc_free_`,
//! `sol_get_clock_sysvar`, `sol_get_rent_sysvar`,
//! `sol_create_program_address`, `sol_try_find_program_address`,
//! `sol_invoke_signed`, `sol_log_compute_units`,
//! `sol_log_data`), CPI dispatch back into the harness, account
//! input-buffer serialization, and the realloc + return-data
//! conventions. The seam is already in place — see
//! [`engine::Engine`] — so Phase 2 lands as one new
//! `BpfEngine` impl plus a single line in
//! [`HopperSvm::new`].
//!
//! ## Quick start
//!
//! ```ignore
//! use hopper_svm::{HopperSvm, KeyedAccount};
//! use hopper_svm::token::create_keyed_system_account;
//! use solana_sdk::pubkey::Pubkey;
//! use solana_sdk::system_instruction;
//!
//! let alice = Pubkey::new_unique();
//! let bob = Pubkey::new_unique();
//! let svm = HopperSvm::new();  // system program registered by default
//!
//! let accounts = vec![
//!     create_keyed_system_account(&alice, 5_000_000),
//!     create_keyed_system_account(&bob, 0),
//! ];
//! let ix = system_instruction::transfer(&alice, &bob, 1_000_000);
//!
//! let result = svm.process_instruction(&ix, &accounts);
//! result.assert_success();
//! assert_eq!(result.account(&bob).unwrap().lamports, 1_000_000);
//! ```
//!
//! [`solana-sbpf`]: https://crates.io/crates/solana-sbpf

#![forbid(unsafe_code)]

pub mod account;
#[cfg(feature = "agave-runtime")]
pub mod agave;
pub mod alt;
#[cfg(feature = "bpf-execution")]
pub mod bpf;
pub mod builtin;
pub mod compute;
pub mod compute_budget_program;
pub mod engine;
pub mod error;
pub mod fees;
pub mod log;
pub mod result;
pub mod spl;
pub mod system_program;
pub mod sysvar;
pub mod token;
pub mod validation;

// Re-exports — anyone holding `use hopper_svm::*;` should be able to
// write a test end-to-end without learning which submodule each
// type lives in.
pub use account::KeyedAccount;
pub use builtin::BuiltinProgram;
pub use compute::ComputeBudget;
pub use engine::{Engine, ExecutionOutcome};
pub use error::HopperSvmError;
pub use fees::FeeCalculator;
pub use log::LogCapture;
pub use result::HopperExecutionResult;
pub use system_program::SystemProgram;
pub use sysvar::{Clock, Rent, Sysvars};
pub use validation::ValidationPolicy;

pub use solana_sdk::account::Account;
pub use solana_sdk::instruction::{AccountMeta, Instruction};
pub use solana_sdk::pubkey::Pubkey;

// SPL constants — match the spelling Quasar uses but resolve to the
// upstream `spl-token` IDs (canonical, not borrowed).
pub use spl_associated_token_account::id as associated_token_program_id;
pub use spl_token::id as spl_token_program_id;
pub use spl_token_2022::id as spl_token_2022_program_id;

/// Convenience: SPL Token program ID.
pub const SPL_TOKEN_PROGRAM_ID: Pubkey = spl_token::ID;
/// Convenience: SPL Token-2022 program ID.
pub const SPL_TOKEN_2022_PROGRAM_ID: Pubkey = spl_token_2022::ID;
/// Convenience: SPL Associated Token Account program ID
/// (`ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL`).
///
/// Hard-coded as bytes rather than re-exporting
/// `spl_associated_token_account::ID` because that const is now
/// typed as `solana_address::Address` (post-spl-ata 8.0), which is
/// nominally distinct from the legacy `solana_sdk::pubkey::Pubkey`
/// the rest of this crate uses. The bytes are identical; the
/// re-typing is what changed.
pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    0x8c, 0x97, 0x25, 0x8f, 0x4e, 0x24, 0x89, 0xf1, 0xbb, 0x3d, 0x10, 0x29, 0x14, 0x8e, 0x0d, 0x83,
    0x0b, 0x5a, 0x13, 0x99, 0xda, 0xff, 0x10, 0x84, 0x04, 0x8e, 0x7b, 0xd8, 0xdb, 0xe9, 0xf8, 0x59,
]);

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// HopperSvm — top-level harness
// ---------------------------------------------------------------------------

/// In-process Solana execution harness, Hopper-native.
///
/// Construction: [`HopperSvm::new`] returns a harness with the
/// system program registered and reasonable defaults for the
/// compute budget and sysvars. Use [`with_builtin`] to add custom
/// built-in programs and [`with_sysvars`] / [`set_compute_budget`]
/// to customise execution conditions.
///
/// Thread safety: `HopperSvm` is `Clone` because the registry, log
/// buffer, and sysvar state are all `Arc<Mutex<…>>` internally —
/// the same harness can be passed across thread boundaries in a
/// test that uses `std::thread::scope`. Each call to
/// `process_instruction` takes a short-lived lock; concurrent
/// instruction execution against the same harness is *serialised*
/// (a real Solana validator runs instructions one at a time per
/// transaction, so this matches semantics).
/// Error surfaced by the Agave-backed BPF loader verbs
/// ([`HopperSvm::load_bpf_program_through_agave`],
/// [`HopperSvm::with_real_spl_token`], etc.).
#[cfg(feature = "agave-runtime")]
#[derive(Debug)]
pub enum HopperSvmAgaveLoadError {
    /// `with_agave_runtime()` was not called before attempting to
    /// load a BPF program.
    NoAgaveEngine,
    /// The Agave loader rejected the bytes (parse / verify failure)
    /// or the runtime environment construction failed.
    Engine(crate::agave::AgaveEngineError),
}

#[cfg(feature = "agave-runtime")]
impl core::fmt::Display for HopperSvmAgaveLoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoAgaveEngine => write!(
                f,
                "hopper-svm: Agave runtime not installed; call with_agave_runtime() first"
            ),
            Self::Engine(err) => write!(f, "hopper-svm: {err}"),
        }
    }
}

#[cfg(feature = "agave-runtime")]
impl std::error::Error for HopperSvmAgaveLoadError {}

#[derive(Clone)]
pub struct HopperSvm {
    /// Map from program ID → registered built-in program. Phase 1
    /// is a built-in-only execution path; Phase 2 falls through
    /// to a `BpfEngine` for IDs not in this map.
    pub(crate) registry: Arc<Mutex<HashMap<Pubkey, Arc<dyn BuiltinProgram>>>>,
    /// Sysvar state — clock, rent, etc. User-settable so tests can
    /// move the clock forward, change rent rates, etc.
    pub(crate) sysvars: Arc<Mutex<Sysvars>>,
    /// Compute meter budget. Decremented per built-in invocation;
    /// programs that exceed the budget abort with
    /// [`HopperSvmError::OutOfComputeUnits`].
    pub(crate) budget: Arc<Mutex<ComputeBudget>>,
    /// Phase 2 BPF engine — feature-gated. Holds the registry of
    /// program-id → ELF bytes that `add_program` populates. When
    /// the built-in registry misses, `dispatch_one` falls through
    /// here.
    #[cfg(feature = "bpf-execution")]
    pub(crate) bpf_engine: bpf::BpfEngine,
    /// Pending CU limit override — written by the compute-budget
    /// program's `SetComputeUnitLimit` handler and read at the
    /// start of every subsequent instruction in
    /// `process_instruction_chain`. `None` means "use the
    /// configured default budget"; `Some(N)` overrides for the
    /// rest of the chain.
    pub(crate) pending_cu_limit: Arc<Mutex<Option<u64>>>,
    /// Pre/post account-state validation policy. Strict (default)
    /// runs every rule on each successful instruction; Lax
    /// disables validation for fast unit tests where structural
    /// invariants don't apply. See [`ValidationPolicy`] for the
    /// rules enforced and [`crate::validation`] for the
    /// rationale.
    pub(crate) validation_policy: Arc<Mutex<ValidationPolicy>>,
    /// Fee calculator (lamports per signature). Mainnet default
    /// is 5000; configurable via [`set_fee_calculator`] for
    /// tests that want to verify fee-edge behaviour.
    pub(crate) fee_calculator: Arc<Mutex<FeeCalculator>>,
    /// Priority-fee surcharge in micro-lamports per CU.
    /// Mutated by the compute-budget program's
    /// `SetComputeUnitPrice` handler; read by
    /// `process_transaction` to compute the priority fee
    /// component.
    pub(crate) priority_fee_micro_lamports_per_cu: Arc<Mutex<u64>>,
    /// Stateful account overlay. Optional; `process_instruction`
    /// remains stateless w.r.t. accounts (caller passes them in by
    /// slice). The overlay enables Quasar-style fixture flows:
    /// `airdrop` / `create_account` / `set_token_balance` populate
    /// it, `process_instruction_with_store` reads from and writes
    /// back to it. The two paths can coexist in one test.
    pub(crate) account_store: Arc<Mutex<HashMap<Pubkey, KeyedAccount>>>,
    /// Optional Agave-runtime engine. When present, instructions
    /// whose `program_id` is registered in the engine's program
    /// cache route through real `solana-program-runtime` /
    /// `solana-bpf-loader-program` invocation instead of Hopper's
    /// inline built-in registry. Enabled with the `agave-runtime`
    /// feature and [`with_agave_runtime`] / [`set_agave_engine`].
    #[cfg(feature = "agave-runtime")]
    pub(crate) agave_engine: Arc<Mutex<Option<agave::AgaveEngine>>>,
}

impl HopperSvm {
    /// Build a new harness with the system program pre-registered
    /// and sensible defaults: 200,000 CU compute budget, slot 0,
    /// epoch 0, default rent rate.
    pub fn new() -> Self {
        let mut registry: HashMap<Pubkey, Arc<dyn BuiltinProgram>> = HashMap::new();
        registry.insert(solana_sdk::system_program::id(), Arc::new(SystemProgram));
        Self {
            registry: Arc::new(Mutex::new(registry)),
            sysvars: Arc::new(Mutex::new(Sysvars::default())),
            budget: Arc::new(Mutex::new(ComputeBudget::default())),
            #[cfg(feature = "bpf-execution")]
            bpf_engine: bpf::BpfEngine::new(),
            pending_cu_limit: Arc::new(Mutex::new(None)),
            validation_policy: Arc::new(Mutex::new(ValidationPolicy::default())),
            fee_calculator: Arc::new(Mutex::new(FeeCalculator::default())),
            priority_fee_micro_lamports_per_cu: Arc::new(Mutex::new(0)),
            account_store: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "agave-runtime")]
            agave_engine: Arc::new(Mutex::new(None)),
        }
    }

    /// Enable the Agave-runtime execution path. Constructs an
    /// [`agave::AgaveEngine`] with the system program pre-registered
    /// (so `system_instruction::transfer` etc. dispatch through
    /// `solana_system_program::system_processor::Entrypoint`, not
    /// the inline Hopper system program), and stashes it on the
    /// harness. Subsequent `process_instruction` calls route
    /// through real Agave when the program is present in the engine,
    /// fall through to the inline registry otherwise.
    ///
    /// This is the headline Tier 3 verb: behaviour now matches
    /// mainnet exactly because it IS the validator's code.
    #[cfg(feature = "agave-runtime")]
    pub fn with_agave_runtime(self) -> Self {
        let engine = agave::AgaveEngine::new();
        engine.install_system_program();
        *self.agave_engine.lock().expect("agave_engine") = Some(engine);
        self
    }

    /// Replace the harness's Agave engine with a pre-built one.
    /// Useful when the test needs to register custom builtins
    /// (e.g. a stake program, a vote program) before installing.
    #[cfg(feature = "agave-runtime")]
    pub fn set_agave_engine(&self, engine: agave::AgaveEngine) {
        *self.agave_engine.lock().expect("agave_engine") = Some(engine);
    }

    /// Borrow the active Agave engine, if any.
    #[cfg(feature = "agave-runtime")]
    pub fn agave_engine(&self) -> Option<agave::AgaveEngine> {
        self.agave_engine.lock().expect("agave_engine").clone()
    }

    /// Load a BPF `.so` program through the Agave engine (if
    /// installed). Returns `Err(NoAgaveEngine)` if `with_agave_runtime`
    /// has not been called. Wraps
    /// [`agave::AgaveEngine::load_bpf_program`] so callers don't
    /// have to pull the engine out by hand.
    #[cfg(feature = "agave-runtime")]
    pub fn load_bpf_program_through_agave(
        &self,
        id: Pubkey,
        loader_kind: agave::AgaveProgramKind,
        loader_key: &Pubkey,
        elf: &[u8],
        account_size: usize,
    ) -> Result<(), HopperSvmAgaveLoadError> {
        let engine = self
            .agave_engine
            .lock()
            .expect("agave_engine")
            .clone()
            .ok_or(HopperSvmAgaveLoadError::NoAgaveEngine)?;
        engine
            .load_bpf_program(id, loader_kind, loader_key, elf, account_size)
            .map_err(HopperSvmAgaveLoadError::Engine)
    }

    /// Convenience: load the SPL Token program ELF supplied by the
    /// caller into the Agave engine under Loader v2 (matches mainnet
    /// deployment). After this call, `process_instruction` calls
    /// targeting `SPL_TOKEN_PROGRAM_ID` route through the real Token
    /// program rather than the inline simulator.
    ///
    /// Hopper-svm does not check in SPL `.so` binaries (200-300 KB
    /// each, license / release-cadence concerns). The user passes
    /// the bytes via `include_bytes!`:
    ///
    /// ```ignore
    /// let svm = HopperSvm::new()
    ///     .with_agave_runtime()
    ///     .with_real_spl_token(include_bytes!("path/to/spl_token.so"))?;
    /// ```
    #[cfg(feature = "agave-runtime")]
    pub fn with_real_spl_token(self, elf: &[u8]) -> Result<Self, HopperSvmAgaveLoadError> {
        self.load_bpf_program_through_agave(
            SPL_TOKEN_PROGRAM_ID,
            agave::AgaveProgramKind::BpfV2,
            &solana_sdk::bpf_loader::id(),
            elf,
            elf.len(),
        )?;
        Ok(self)
    }

    /// SPL Token-2022 sibling of [`with_real_spl_token`]. Same
    /// pattern — Loader v2, registered under
    /// [`SPL_TOKEN_2022_PROGRAM_ID`].
    #[cfg(feature = "agave-runtime")]
    pub fn with_real_spl_token_2022(self, elf: &[u8]) -> Result<Self, HopperSvmAgaveLoadError> {
        self.load_bpf_program_through_agave(
            SPL_TOKEN_2022_PROGRAM_ID,
            agave::AgaveProgramKind::BpfV2,
            &solana_sdk::bpf_loader::id(),
            elf,
            elf.len(),
        )?;
        Ok(self)
    }

    /// SPL Associated Token Account sibling. Loader v2, registered
    /// under [`ASSOCIATED_TOKEN_PROGRAM_ID`].
    #[cfg(feature = "agave-runtime")]
    pub fn with_real_spl_associated_token(
        self,
        elf: &[u8],
    ) -> Result<Self, HopperSvmAgaveLoadError> {
        self.load_bpf_program_through_agave(
            ASSOCIATED_TOKEN_PROGRAM_ID,
            agave::AgaveProgramKind::BpfV2,
            &solana_sdk::bpf_loader::id(),
            elf,
            elf.len(),
        )?;
        Ok(self)
    }

    /// Override the harness's fee calculator. Mainnet default is
    /// 5000 lamports/signature; tests that need to verify
    /// fee-edge behaviour (e.g. genesis configurations with
    /// different rates) can override here.
    pub fn set_fee_calculator(&self, fc: FeeCalculator) {
        *self.fee_calculator.lock().expect("fee_calculator") = fc;
    }

    /// Read the current fee calculator.
    pub fn fee_calculator(&self) -> FeeCalculator {
        self.fee_calculator.lock().expect("fee_calculator").clone()
    }

    /// Set the priority-fee surcharge (μlamports per CU).
    /// Normally written by the compute-budget program's
    /// `SetComputeUnitPrice` handler; this setter is for tests
    /// that want to seed the value directly.
    pub fn set_priority_fee_micro_lamports_per_cu(&self, micro_lamports: u64) {
        *self
            .priority_fee_micro_lamports_per_cu
            .lock()
            .expect("priority_fee") = micro_lamports;
    }

    /// Read the current priority-fee surcharge.
    pub fn priority_fee_micro_lamports_per_cu(&self) -> u64 {
        *self
            .priority_fee_micro_lamports_per_cu
            .lock()
            .expect("priority_fee")
    }

    /// Disable post-instruction account validation for this
    /// harness. The default is [`ValidationPolicy::Strict`],
    /// which catches mainnet-only bugs (non-owner data writes,
    /// lamport non-conservation, executable-flag toggles, etc.)
    /// locally before they ship.
    ///
    /// Use lax for fast unit tests of pure business logic where
    /// the structural invariants don't apply — e.g. when the
    /// "program" is a hand-written Rust simulator that doesn't
    /// follow Solana's account-mutation rules.
    pub fn with_lax_validation(self) -> Self {
        *self.validation_policy.lock().expect("validation_policy") = ValidationPolicy::Lax;
        self
    }

    /// Override the validation policy. Most callers use
    /// [`with_lax_validation`] (the only non-default choice);
    /// this is the escape hatch for tests that toggle policy
    /// mid-run.
    pub fn set_validation_policy(&self, policy: ValidationPolicy) {
        *self.validation_policy.lock().expect("validation_policy") = policy;
    }

    /// Register the bundled Compute Budget program against
    /// [`compute_budget_program::COMPUTE_BUDGET_PROGRAM_ID`].
    /// Programs that include compute-budget instructions in
    /// their transaction (the common
    /// `ComputeBudgetInstruction::set_compute_unit_limit(N)`
    /// pattern) will see the new limit applied on subsequent
    /// instructions in the same chain. Without this builder
    /// registered, compute-budget instructions surface as
    /// `UnknownProgram`.
    pub fn with_compute_budget_program(self) -> Self {
        let pending = self.pending_cu_limit.clone();
        let priority_fee = self.priority_fee_micro_lamports_per_cu.clone();
        self.with_builtin(
            compute_budget_program::COMPUTE_BUDGET_PROGRAM_ID,
            compute_budget_program::ComputeBudgetProgramSimulator {
                pending_cu_limit: pending,
                priority_fee,
            },
        )
    }

    /// Phase 2: register a `.so` BPF program by ID. Reads
    /// `target/deploy/<name>.so` from the cargo workspace root
    /// (the standard `cargo build-sbf` output path) and stores
    /// the bytes against `id`. Subsequent `process_instruction`
    /// calls whose `program_id` matches dispatch into the BPF
    /// engine.
    ///
    /// Feature-gated: only available with `--features bpf-execution`.
    /// Phase 1 builds — the default — surface a clear "Phase 2 not
    /// enabled" error if a program is invoked without a built-in
    /// registered for it.
    #[cfg(feature = "bpf-execution")]
    pub fn add_program(&self, id: &Pubkey, name: &str) -> Result<(), std::io::Error> {
        let path = std::path::Path::new("target")
            .join("deploy")
            .join(format!("{name}.so"));
        let elf = std::fs::read(&path)?;
        self.bpf_engine.add_elf(id, elf);
        Ok(())
    }

    /// Phase 2: register a `.so` BPF program from in-memory bytes.
    /// Useful for tests that compile and embed the program via
    /// `include_bytes!` rather than reading from `target/deploy`.
    /// Defaults to [`bpf::engine::LoaderKind::V3`] (the modern
    /// upgradeable loader); use [`add_program_with_loader`] to
    /// pin the legacy V2 loader when registering an SPL program
    /// or any other v2-deployed binary.
    #[cfg(feature = "bpf-execution")]
    pub fn add_program_from_bytes(&self, id: &Pubkey, elf: Vec<u8>) {
        self.bpf_engine.add_elf(id, elf);
    }

    /// Phase 2: register a BPF program with an explicit loader kind.
    /// Mirrors `quasar-svm`'s `add_program(id, loader, elf)`.
    /// SPL Token, Memo, and Token-2022 are all V2-deployed on
    /// mainnet; everything Anchor-shipped or Hopper-shipped is V3.
    #[cfg(feature = "bpf-execution")]
    pub fn add_program_with_loader(
        &self,
        id: &Pubkey,
        loader: bpf::engine::LoaderKind,
        elf: Vec<u8>,
    ) {
        self.bpf_engine.add_elf_with_loader(id, elf, loader);
    }

    /// Builder-style sibling of [`add_program_from_bytes`].
    /// Returns the harness so registrations can chain. Defaults to
    /// V3 loader.
    #[cfg(feature = "bpf-execution")]
    pub fn with_program(self, id: Pubkey, elf: Vec<u8>) -> Self {
        self.bpf_engine.add_elf(&id, elf);
        self
    }

    /// Builder-style sibling of [`add_program_with_loader`]. Pin
    /// the loader explicitly. Mirrors Quasar's
    /// `with_program_loader(id, loader, elf)`.
    #[cfg(feature = "bpf-execution")]
    pub fn with_program_loader(
        self,
        id: Pubkey,
        loader: bpf::engine::LoaderKind,
        elf: Vec<u8>,
    ) -> Self {
        self.bpf_engine.add_elf_with_loader(&id, elf, loader);
        self
    }

    /// Register an SPL Token program ELF supplied by the caller.
    ///
    /// Hopper-svm intentionally does not check in SPL `.so`
    /// binaries (they're 200-300 KB each and the canonical bytes
    /// are under Anza's own license / release cadence). Callers
    /// embed the bytes in their tests:
    ///
    /// ```ignore
    /// let svm = HopperSvm::new()
    ///     .with_bundled_spl_token(include_bytes!("path/to/spl_token.so").to_vec());
    /// ```
    ///
    /// The ELF is registered against [`SPL_TOKEN_PROGRAM_ID`] under
    /// the V2 loader (matches the mainnet deployment). For tests
    /// that don't need the real Token program, the inline simulator
    /// at [`with_spl_token_simulator`] is faster and ships in the
    /// default feature set.
    #[cfg(feature = "bpf-execution")]
    pub fn with_bundled_spl_token(self, elf: Vec<u8>) -> Self {
        self.bpf_engine.add_elf_with_loader(
            &SPL_TOKEN_PROGRAM_ID,
            elf,
            bpf::engine::LoaderKind::V2,
        );
        self
    }

    /// Register an SPL Token-2022 program ELF supplied by the
    /// caller. Same pattern as [`with_bundled_spl_token`]; the ELF
    /// is registered against [`SPL_TOKEN_2022_PROGRAM_ID`] under
    /// the V2 loader (Token-2022's mainnet deployment is V2).
    #[cfg(feature = "bpf-execution")]
    pub fn with_bundled_spl_token_2022(self, elf: Vec<u8>) -> Self {
        self.bpf_engine.add_elf_with_loader(
            &SPL_TOKEN_2022_PROGRAM_ID,
            elf,
            bpf::engine::LoaderKind::V2,
        );
        self
    }

    /// Register an SPL Associated Token Account program ELF
    /// supplied by the caller. Registered against
    /// [`ASSOCIATED_TOKEN_PROGRAM_ID`] under the V2 loader.
    #[cfg(feature = "bpf-execution")]
    pub fn with_bundled_spl_associated_token(self, elf: Vec<u8>) -> Self {
        self.bpf_engine.add_elf_with_loader(
            &ASSOCIATED_TOKEN_PROGRAM_ID,
            elf,
            bpf::engine::LoaderKind::V2,
        );
        self
    }

    /// Register a built-in program by ID. Builder-style, so several
    /// can be chained. Replaces any previous registration for the
    /// same ID — tests that need to override the system program
    /// (or any other built-in) for fault injection can do so freely.
    pub fn with_builtin<P: BuiltinProgram + 'static>(self, id: Pubkey, program: P) -> Self {
        self.registry
            .lock()
            .expect("registry lock")
            .insert(id, Arc::new(program));
        self
    }

    /// Register the bundled SPL Token simulator against
    /// [`SPL_TOKEN_PROGRAM_ID`]. Pure-Rust implementation of
    /// the 8 most-used Token instructions (`InitializeMint`,
    /// `InitializeAccount`, `Transfer`, `Approve`, `Revoke`,
    /// `MintTo`, `Burn`, `CloseAccount`). Phase-1-execution
    /// path: 10-100× faster than going through BPF for the same
    /// instructions, no `.so` bytes to maintain.
    ///
    /// Other tags (`FreezeAccount`, `*Checked` variants,
    /// `SetAuthority`, etc.) return a structured "not yet
    /// supported by the bundled simulator" error so tests that
    /// hit them fail fast with an actionable message. A future
    /// release expands the set; in the meantime, programs that
    /// need an unsupported instruction should register the real
    /// SPL `.so` via [`add_program`] instead.
    pub fn with_spl_token_simulator(self) -> Self {
        self.with_builtin(SPL_TOKEN_PROGRAM_ID, spl::token::SplTokenSimulator)
    }

    /// Register the bundled SPL Token-2022 simulator against
    /// [`SPL_TOKEN_2022_PROGRAM_ID`]. Common (legacy-compatible)
    /// tags 0-9 delegate to the same logic as the Token
    /// simulator — the on-disk Mint/Account layout is identical
    /// for non-extension accounts. Extension tags (22+)
    /// produce a structured "Phase 1 simulator doesn't support
    /// this extension" error pointing at
    /// [`add_program`](Self::add_program) for the real `.so`
    /// fallback.
    pub fn with_spl_token_2022_simulator(self) -> Self {
        self.with_builtin(
            SPL_TOKEN_2022_PROGRAM_ID,
            spl::token_2022::SplToken2022Simulator,
        )
    }

    /// Register the bundled SPL Associated Token Account program
    /// simulator against [`ASSOCIATED_TOKEN_PROGRAM_ID`].
    /// Handles `Create` and `CreateIdempotent`. Validates the
    /// derived ATA address, allocates the account inline, and
    /// initialises it as a token account owned by the supplied
    /// token program (Token or Token-2022). No CPI dispatch is
    /// needed — the create + initialise flow is deterministic
    /// and expressible in terms of direct account-state mutation.
    pub fn with_spl_associated_token_simulator(self) -> Self {
        self.with_builtin(ASSOCIATED_TOKEN_PROGRAM_ID, spl::ata::SplAtaSimulator)
    }

    /// Convenience: register all three SPL simulators in one
    /// call. Equivalent to chaining
    /// `.with_spl_token_simulator()
    ///  .with_spl_token_2022_simulator()
    ///  .with_spl_associated_token_simulator()`. Useful for
    /// tests that exercise mixed token-program flows.
    pub fn with_spl_simulators(self) -> Self {
        self.with_spl_token_simulator()
            .with_spl_token_2022_simulator()
            .with_spl_associated_token_simulator()
    }

    /// Register the bundled Address Lookup Table program against
    /// `AddressLookupTab1e1111111111111111111111111111`. Handles
    /// `Create`, `Freeze`, `Extend`, `Deactivate`, `Close`. The
    /// 56-byte meta header + 32-byte-per-address data layout
    /// matches mainnet exactly so tables created here can be
    /// fed into [`resolve_address_table_lookup`] without
    /// translation.
    pub fn with_alt_program(self) -> Self {
        self.with_builtin(
            spl::alt_program::ALT_PROGRAM_ID,
            spl::alt_program::AltProgramSimulator,
        )
    }

    /// Register the Config program simulator at
    /// `Config1111111111111111111111111111111111111`. Handles the
    /// single `Store` instruction. Used by validator-side surface
    /// (stake-config etc.); rarely touched by application
    /// programs but bundled for mainnet parity.
    pub fn with_config_program(self) -> Self {
        self.with_builtin(
            spl::config_program::CONFIG_PROGRAM_ID,
            spl::config_program::ConfigProgramSimulator,
        )
    }

    /// Register the Stake program simulator at
    /// `Stake11111111111111111111111111111111111111`. Covers the
    /// lifecycle slice — `Initialize`, `Authorize`,
    /// `DelegateStake`, `Withdraw`, `Deactivate`. See
    /// [`spl::stake_program`] for the divergences from upstream
    /// (no validator-feature variants, no warmup/cooldown
    /// machinery).
    pub fn with_stake_program(self) -> Self {
        self.with_builtin(
            spl::stake_program::STAKE_PROGRAM_ID,
            spl::stake_program::StakeProgramSimulator,
        )
    }

    /// Register the Vote program simulator at
    /// `Vote111111111111111111111111111111111111111`. Covers the
    /// administrative slice — `InitializeAccount`, `Authorize`,
    /// `Withdraw`, `UpdateValidatorIdentity`, `UpdateCommission`.
    /// Vote-emitting variants (the TowerBFT lockout machinery)
    /// are out of scope for Phase 1 — see
    /// [`spl::vote_program`].
    pub fn with_vote_program(self) -> Self {
        self.with_builtin(
            spl::vote_program::VOTE_PROGRAM_ID,
            spl::vote_program::VoteProgramSimulator,
        )
    }

    /// Resolve a v0-transaction `MessageAddressTableLookup`-shaped
    /// reference into concrete `(pubkey, is_writable)` pairs.
    ///
    /// Reads the lookup table from the supplied account state,
    /// pulls out each indexed address, returns two vectors:
    /// `(writable_addrs, readonly_addrs)`. The convention is
    /// the same one mainnet uses: writable indexes resolve to
    /// writable account metas, readonly indexes to read-only.
    ///
    /// Errors if the table account is missing, isn't a valid
    /// lookup table, is currently deactivated, or any index is
    /// out of bounds.
    pub fn resolve_address_table_lookup(
        &self,
        accounts: &[KeyedAccount],
        table_address: &Pubkey,
        writable_indexes: &[u8],
        readonly_indexes: &[u8],
    ) -> Result<(Vec<Pubkey>, Vec<Pubkey>), HopperSvmError> {
        let table = accounts
            .iter()
            .find(|a| &a.address == table_address)
            .ok_or_else(|| HopperSvmError::UnknownAccount(*table_address))?;
        let meta = alt::read_meta(&table.data).ok_or_else(|| HopperSvmError::BuiltinError {
            program_id: spl::alt_program::ALT_PROGRAM_ID,
            message: format!("resolve: {table_address} not a valid lookup table"),
        })?;
        // Mainnet rejects lookups against tables whose
        // deactivation has fully cooled down (the table is
        // effectively closed even before Close runs). We mirror
        // that.
        if meta.is_closeable(self.sysvars().clock.slot) {
            return Err(HopperSvmError::BuiltinError {
                program_id: spl::alt_program::ALT_PROGRAM_ID,
                message: format!(
                    "resolve: {table_address} deactivated and cooled down — addresses no longer resolvable"
                ),
            });
        }
        alt::resolve_lookup(&table.data, writable_indexes, readonly_indexes).map_err(|err| {
            HopperSvmError::BuiltinError {
                program_id: spl::alt_program::ALT_PROGRAM_ID,
                message: format!("resolve {table_address}: {err}"),
            }
        })
    }

    /// Convenience: register the full Solana runtime surface in
    /// one call. Equivalent to chaining `.with_compute_budget_program()
    /// .with_spl_simulators()`. The system program is already
    /// registered by `new()`. Mirrors `quasar-svm`'s
    /// "SPL programs loaded by default on `QuasarSvm::new()`"
    /// behaviour without making `new()` itself heavyweight —
    /// authors who want a stripped harness for fault injection
    /// keep the bare `new()`, authors who want the full
    /// out-of-the-box runtime call `.with_solana_runtime()`.
    ///
    /// Concretely, after this call the harness has:
    /// - System program (registered by `new()`)
    /// - Compute Budget program
    /// - Address Lookup Table program
    /// - Config program
    /// - Stake program
    /// - Vote program
    /// - SPL Token simulator
    /// - SPL Token-2022 simulator
    /// - Associated Token Account simulator
    pub fn with_solana_runtime(self) -> Self {
        self.with_compute_budget_program()
            .with_alt_program()
            .with_config_program()
            .with_stake_program()
            .with_vote_program()
            .with_spl_simulators()
    }

    /// Replace the sysvar state. Useful for time-sensitive tests
    /// (clock-windowed instructions, rent-dependent realloc).
    pub fn with_sysvars(self, sysvars: Sysvars) -> Self {
        *self.sysvars.lock().expect("sysvars lock") = sysvars;
        self
    }

    /// Override the compute budget for subsequent executions.
    pub fn set_compute_budget(&self, units: u64) {
        self.budget.lock().expect("budget lock").set_limit(units);
    }

    /// Borrow the current sysvar state — useful for assertions
    /// against an instruction's effect on slot/timestamp.
    pub fn sysvars(&self) -> Sysvars {
        self.sysvars.lock().expect("sysvars lock").clone()
    }

    /// Replace the harness's clock sysvar. Mirrors `quasar-svm`'s
    /// `svm.set_clock(c)` for porting clock-sensitive tests.
    pub fn set_clock(&self, clock: Clock) {
        self.sysvars.lock().expect("sysvars lock").clock = clock;
    }

    /// Replace the harness's rent sysvar.
    pub fn set_rent(&self, rent: Rent) {
        self.sysvars.lock().expect("sysvars lock").rent = rent;
    }

    /// Replace the harness's epoch-schedule sysvar.
    pub fn set_epoch_schedule(&self, epoch_schedule: sysvar::EpochSchedule) {
        self.sysvars.lock().expect("sysvars lock").epoch_schedule = epoch_schedule;
    }

    /// Replace the harness's last-restart-slot sysvar.
    pub fn set_last_restart_slot(&self, last_restart_slot: sysvar::LastRestartSlot) {
        self.sysvars.lock().expect("sysvars lock").last_restart_slot = last_restart_slot;
    }

    /// Replace the harness's epoch-rewards sysvar.
    pub fn set_epoch_rewards(&self, epoch_rewards: sysvar::EpochRewards) {
        self.sysvars.lock().expect("sysvars lock").epoch_rewards = epoch_rewards;
    }

    /// Move the simulated clock to a specific slot. Updates
    /// `clock.slot`, the `unix_timestamp` (advances by 400 ms
    /// per slot — Solana's target slot duration), and the
    /// `epoch` (computed from the configured `EpochSchedule`).
    /// `epoch_start_timestamp` is left untouched so test
    /// epochs remain anchored to whatever the test set them to.
    /// Mirrors `quasar-svm`'s `svm.sysvars.warp_to_slot(N)`.
    pub fn warp_to_slot(&self, slot: u64) {
        let mut sv = self.sysvars.lock().expect("sysvars lock");
        let prev_slot = sv.clock.slot;
        let elapsed_slots = slot.saturating_sub(prev_slot) as i64;
        // 400 ms per slot, Solana's mainnet-target slot
        // duration. Tests that need a different cadence should
        // call `set_clock` directly with the precise values.
        sv.clock.unix_timestamp = sv
            .clock
            .unix_timestamp
            .saturating_add(elapsed_slots.saturating_mul(400) / 1000);
        sv.clock.slot = slot;
        // Epoch derivation: a slot lives in the first "warmup"
        // epochs while it's below `first_normal_slot`, then in
        // the steady-state epochs after. Match
        // `solana_sdk::epoch_schedule::EpochSchedule::get_epoch`.
        let es = &sv.epoch_schedule;
        sv.clock.epoch = if slot < es.first_normal_slot {
            // Warmup phase: epoch numbers track log_2 progression
            // from MINIMUM_SLOTS_PER_EPOCH (32). For test
            // simplicity we approximate as slot / 32; programs
            // that care about exact warmup math should use
            // `set_clock` with explicit values.
            slot / 32
        } else {
            es.first_normal_epoch + (slot - es.first_normal_slot) / es.slots_per_epoch
        };
    }

    /// Move the harness clock to a specific Unix timestamp and
    /// derive the matching slot from the elapsed wall-clock time.
    ///
    /// Solana's mainnet target is 400 ms per slot; this method
    /// uses the same cadence so a forward warp by `delta` seconds
    /// advances `clock.slot` by `delta * 1000 / 400` slots. The
    /// epoch is recomputed via the same `EpochSchedule` math
    /// [`warp_to_slot`] uses.
    ///
    /// Negative timestamps are clamped to zero. Going backwards in
    /// time is permitted (some tests want to assert behaviour at a
    /// specific epoch boundary regardless of monotonic clock
    /// invariants); the slot recomputes accordingly.
    ///
    /// Mirrors `quasar-svm`'s `svm.sysvars.warp_to_timestamp(t)`.
    pub fn warp_to_timestamp(&self, unix_timestamp: i64) {
        let mut sv = self.sysvars.lock().expect("sysvars lock");
        let prev_ts = sv.clock.unix_timestamp;
        let prev_slot = sv.clock.slot;
        let delta_ms = unix_timestamp.saturating_sub(prev_ts).saturating_mul(1000);
        let delta_slots = delta_ms / 400;
        let new_slot = if delta_slots >= 0 {
            prev_slot.saturating_add(delta_slots as u64)
        } else {
            prev_slot.saturating_sub((-delta_slots) as u64)
        };
        sv.clock.unix_timestamp = unix_timestamp.max(0);
        sv.clock.slot = new_slot;
        let es = &sv.epoch_schedule;
        sv.clock.epoch = if new_slot < es.first_normal_slot {
            new_slot / 32
        } else {
            es.first_normal_epoch + (new_slot - es.first_normal_slot) / es.slots_per_epoch
        };
    }

    /// Non-mutating dry run of [`process_instruction`]. Returns the
    /// same [`HopperExecutionResult`] but the caller's `accounts`
    /// slice is never observed-as-mutated, and the harness's own
    /// state (sysvars, fee config, compute budget) is untouched.
    ///
    /// Hopper-svm's stateless model means [`process_instruction`]
    /// already does not mutate the input slice, so the runtime
    /// behaviour of `simulate_instruction` is identical. The
    /// distinct verb exists for API parity with `quasar-svm` and
    /// `mollusk-svm` and to make the intent ("read-only, do not
    /// commit") explicit at the call site. Tests that rely on the
    /// stateful overlay (`store_*` helpers) should call
    /// [`simulate_instruction_with_store`] instead, which guarantees
    /// the harness's account store is rolled back to its pre-call
    /// snapshot regardless of the instruction's outcome.
    #[inline]
    pub fn simulate_instruction(
        &self,
        ix: &Instruction,
        accounts: &[KeyedAccount],
    ) -> HopperExecutionResult {
        self.process_instruction(ix, accounts)
    }

    /// Non-mutating dry run of [`process_instruction_chain`].
    /// Carries forward account state between instructions in the
    /// chain (matching the real chain's semantics) but does not
    /// expose those mutations to the caller; the returned outcome's
    /// `resulting_accounts` is the post-chain projection.
    ///
    /// Same caveat as [`simulate_instruction`]: this is identical
    /// to [`process_instruction_chain`] in the stateless model and
    /// distinct only in [`simulate_instruction_chain_with_store`].
    #[inline]
    pub fn simulate_instruction_chain(
        &self,
        ixs: &[Instruction],
        accounts: &[KeyedAccount],
    ) -> HopperExecutionResult {
        self.process_instruction_chain(ixs, accounts)
    }

    // -------------------------------------------------------------
    // Stateful account-store API (Quasar-style fixture flows)
    // -------------------------------------------------------------

    /// Insert or replace an account in the harness's stateful
    /// overlay. Subsequent calls to [`process_instruction_with_store`]
    /// read from this map; calls to the stateless
    /// [`process_instruction`] do not see it. Mirrors Quasar's
    /// `svm.set_account(account)`.
    pub fn set_account(&self, account: KeyedAccount) {
        self.account_store
            .lock()
            .expect("account_store lock")
            .insert(account.address, account);
    }

    /// Read an account from the stateful overlay. Returns `None` if
    /// the address has not been seeded with [`set_account`] / [`airdrop`]
    /// / [`create_account`] / [`set_token_balance`] /
    /// [`set_mint_supply`], or has not been written by a previous
    /// [`process_instruction_with_store`] call.
    pub fn get_account(&self, address: &Pubkey) -> Option<KeyedAccount> {
        self.account_store
            .lock()
            .expect("account_store lock")
            .get(address)
            .cloned()
    }

    /// Remove an account from the overlay and return it.
    pub fn remove_account(&self, address: &Pubkey) -> Option<KeyedAccount> {
        self.account_store
            .lock()
            .expect("account_store lock")
            .remove(address)
    }

    /// Drop every account from the overlay. Useful between test
    /// scenarios that share a `HopperSvm` to avoid cross-test
    /// contamination.
    pub fn clear_accounts(&self) {
        self.account_store
            .lock()
            .expect("account_store lock")
            .clear();
    }

    /// Snapshot the entire account overlay. Used by
    /// [`simulate_instruction_with_store`] /
    /// [`simulate_instruction_chain_with_store`] to roll back state
    /// after a non-mutating dry run.
    pub fn snapshot_accounts(&self) -> HashMap<Pubkey, KeyedAccount> {
        self.account_store
            .lock()
            .expect("account_store lock")
            .clone()
    }

    /// Replace the overlay with a previously taken snapshot. Pair
    /// with [`snapshot_accounts`] for "set up, run a probe, restore"
    /// patterns.
    pub fn restore_accounts(&self, snapshot: HashMap<Pubkey, KeyedAccount>) {
        *self.account_store.lock().expect("account_store lock") = snapshot;
    }

    /// Credit `lamports` to `address`, creating a system-owned
    /// account if it does not yet exist. Equivalent to a real
    /// validator's airdrop instruction without going through the
    /// system program. Mirrors Quasar's `svm.airdrop(&pk, n)`.
    pub fn airdrop(&self, address: &Pubkey, lamports: u64) {
        let mut store = self.account_store.lock().expect("account_store lock");
        match store.get_mut(address) {
            Some(existing) => {
                existing.lamports = existing.lamports.saturating_add(lamports);
            }
            None => {
                store.insert(
                    *address,
                    KeyedAccount {
                        address: *address,
                        lamports,
                        data: Vec::new(),
                        owner: solana_sdk::system_program::id(),
                        executable: false,
                        rent_epoch: 0,
                    },
                );
            }
        }
    }

    /// Create a fresh empty account with `space` zero-initialized
    /// data bytes, owned by `owner`, with a rent-exempt lamport
    /// balance computed from the configured rent sysvar.
    /// Replaces any existing account at the same address.
    /// Mirrors Quasar's `svm.create_account(&pk, space, &owner)`.
    pub fn create_account(&self, address: &Pubkey, space: usize, owner: &Pubkey) {
        let rent = self.sysvars.lock().expect("sysvars lock").rent.clone();
        let lamports = rent.minimum_balance(space);
        self.account_store
            .lock()
            .expect("account_store lock")
            .insert(
                *address,
                KeyedAccount {
                    address: *address,
                    lamports,
                    data: vec![0u8; space],
                    owner: *owner,
                    executable: false,
                    rent_epoch: 0,
                },
            );
    }

    /// Force a token account's balance to `amount`, packing the new
    /// SPL TokenAccount wire shape over the existing 165-byte slot.
    /// The account must already exist (via [`set_account`] or
    /// [`token::create_keyed_token_account`]); panics with a clear
    /// message if it does not, since silently creating a token
    /// account requires a mint and an owner that this verb cannot
    /// invent. Mirrors Quasar's `svm.set_token_balance(&pk, n)`.
    pub fn set_token_balance(&self, address: &Pubkey, amount: u64) {
        let mut store = self.account_store.lock().expect("account_store lock");
        let acct = store.get_mut(address).unwrap_or_else(|| {
            panic!(
                "hopper-svm: set_token_balance({address}, {amount}) on an account that has not been seeded; \
                 call set_account or token::create_keyed_token_account first"
            )
        });
        // SPL TokenAccount.amount is at bytes 64..72 (LE u64) for
        // both legacy SPL Token and Token-2022's vanilla layout.
        if acct.data.len() < 72 {
            panic!(
                "hopper-svm: set_token_balance({address}, {amount}) on a buffer that is {} bytes \
                 (expected at least 72 for SPL TokenAccount layout)",
                acct.data.len()
            );
        }
        acct.data[64..72].copy_from_slice(&amount.to_le_bytes());
    }

    /// Force a mint account's supply to `supply`, packing the new
    /// SPL Mint wire shape over the existing slot. Same caveat as
    /// [`set_token_balance`]: the account must exist.
    /// Mirrors Quasar's `svm.set_mint_supply(&pk, n)`.
    pub fn set_mint_supply(&self, address: &Pubkey, supply: u64) {
        let mut store = self.account_store.lock().expect("account_store lock");
        let acct = store.get_mut(address).unwrap_or_else(|| {
            panic!(
                "hopper-svm: set_mint_supply({address}, {supply}) on an account that has not been seeded; \
                 call set_account or token::create_keyed_mint_account first"
            )
        });
        // SPL Mint.supply is at bytes 36..44 (LE u64) on both
        // legacy SPL Token and Token-2022's vanilla layout.
        // Mint layout: [mint_authority_tag (4) | mint_authority (32) | supply (8) | decimals (1) | ...]
        if acct.data.len() < 44 {
            panic!(
                "hopper-svm: set_mint_supply({address}, {supply}) on a buffer that is {} bytes \
                 (expected at least 44 for SPL Mint layout)",
                acct.data.len()
            );
        }
        acct.data[36..44].copy_from_slice(&supply.to_le_bytes());
    }

    /// Stateful sibling of [`process_instruction`]. Reads the
    /// instruction's account inputs from the harness's overlay,
    /// dispatches, and writes any state changes back to the overlay.
    /// Returns the same [`HopperExecutionResult`] shape so existing
    /// assertion helpers work unchanged.
    ///
    /// Accounts referenced in `ix` that are not in the overlay are
    /// synthesised as zero-data system-owned accounts with zero
    /// lamports, matching how a real validator presents an unknown
    /// account to a program.
    pub fn process_instruction_with_store(&self, ix: &Instruction) -> HopperExecutionResult {
        let inputs = self.collect_inputs(ix);
        let result = self.process_instruction(ix, &inputs);
        if result.is_success() {
            self.commit(&result);
        }
        result
    }

    /// Stateful sibling of [`process_instruction_chain`]. Same
    /// semantics as the chain (later instructions see earlier
    /// instructions' writes); the overlay receives the final post-
    /// chain state on success.
    pub fn process_instruction_chain_with_store(
        &self,
        ixs: &[Instruction],
    ) -> HopperExecutionResult {
        // Gather the union of accounts referenced by any instruction
        // in the chain so the dispatcher sees one consistent input
        // slice. Order matters for some programs: keep first-seen
        // order from the chain, then fall back to current overlay
        // order for any account that wasn't referenced.
        let mut seen: Vec<Pubkey> = Vec::new();
        for ix in ixs {
            if !seen.iter().any(|p| p == &ix.program_id) {
                seen.push(ix.program_id);
            }
            for meta in &ix.accounts {
                if !seen.iter().any(|p| p == &meta.pubkey) {
                    seen.push(meta.pubkey);
                }
            }
        }
        let store = self.account_store.lock().expect("account_store lock");
        let inputs: Vec<KeyedAccount> = seen
            .into_iter()
            .map(|addr| {
                store.get(&addr).cloned().unwrap_or_else(|| KeyedAccount {
                    address: addr,
                    lamports: 0,
                    data: Vec::new(),
                    owner: solana_sdk::system_program::id(),
                    executable: false,
                    rent_epoch: 0,
                })
            })
            .collect();
        drop(store);
        let result = self.process_instruction_chain(ixs, &inputs);
        if result.is_success() {
            self.commit(&result);
        }
        result
    }

    /// Stateful, non-mutating dry run. Snapshots the overlay,
    /// runs [`process_instruction_with_store`], restores the
    /// overlay regardless of outcome, returns the result.
    pub fn simulate_instruction_with_store(&self, ix: &Instruction) -> HopperExecutionResult {
        let snapshot = self.snapshot_accounts();
        let result = self.process_instruction_with_store(ix);
        self.restore_accounts(snapshot);
        result
    }

    /// Stateful, non-mutating dry run for chains.
    pub fn simulate_instruction_chain_with_store(
        &self,
        ixs: &[Instruction],
    ) -> HopperExecutionResult {
        let snapshot = self.snapshot_accounts();
        let result = self.process_instruction_chain_with_store(ixs);
        self.restore_accounts(snapshot);
        result
    }

    /// Build the input slice for a single-instruction call from the
    /// overlay. Same defaulting rule as the chain version: unknown
    /// accounts become zero-lamport system-owned slots.
    fn collect_inputs(&self, ix: &Instruction) -> Vec<KeyedAccount> {
        let store = self.account_store.lock().expect("account_store lock");
        // Program account first (matches the runtime's
        // expectation that account index 0 maps to the program
        // when the program is provided as part of the instruction's
        // account list).
        let mut seen: Vec<Pubkey> = Vec::new();
        if !seen.iter().any(|p| p == &ix.program_id) {
            seen.push(ix.program_id);
        }
        for meta in &ix.accounts {
            if !seen.iter().any(|p| p == &meta.pubkey) {
                seen.push(meta.pubkey);
            }
        }
        seen.into_iter()
            .map(|addr| {
                store.get(&addr).cloned().unwrap_or_else(|| KeyedAccount {
                    address: addr,
                    lamports: 0,
                    data: Vec::new(),
                    owner: solana_sdk::system_program::id(),
                    executable: false,
                    rent_epoch: 0,
                })
            })
            .collect()
    }

    /// Apply an execution result's `resulting_accounts` to the
    /// overlay. Called automatically by the `*_with_store` verbs
    /// on success; exposed publicly so a test can stage a result
    /// from a stateless call into the overlay if needed.
    pub fn commit(&self, result: &HopperExecutionResult) {
        let mut store = self.account_store.lock().expect("account_store lock");
        for acct in result.resulting_accounts() {
            store.insert(acct.address, acct.clone());
        }
    }

    /// Process a single instruction atomically. Phase 1: dispatches
    /// to the registered built-in for `ix.program_id`. Returns an
    /// error result (not a panic) when the program ID is unknown,
    /// so tests of "did I forget to register the program" assertions
    /// can run against a `Result<T, _>`.
    pub fn process_instruction(
        &self,
        ix: &Instruction,
        accounts: &[KeyedAccount],
    ) -> HopperExecutionResult {
        let mut log_capture = LogCapture::default();
        let outcome = self.dispatch_one(ix, accounts, &mut log_capture);
        HopperExecutionResult::from_outcome(outcome, log_capture)
    }

    /// Process a slice of instructions as a full transaction —
    /// **fee-aware** chain dispatch. Deducts the transaction
    /// fee from `fee_payer` up front (matches mainnet's
    /// "fee charged even if tx fails" semantics), runs every
    /// instruction in order, returns the final outcome with
    /// `transaction_fee_paid` populated.
    ///
    /// Fee formula: `base_fee + priority_fee` where
    /// - `base_fee = lamports_per_signature × num_unique_signers`
    /// - `priority_fee = compute_unit_limit × micro_lamports_per_cu / 1_000_000`
    ///
    /// `compute_unit_limit` defaults to the harness's per-
    /// instruction limit (200 000); programs that ran a
    /// `SetComputeUnitLimit` instruction earlier in the chain
    /// have their value applied here too.
    ///
    /// If the fee payer can't afford the fee, the chain aborts
    /// before the first instruction with
    /// [`HopperSvmError::InsufficientFunds`]. If the chain
    /// runs but a later instruction fails, the fee is **still
    /// deducted** — matches mainnet's anti-spam rule.
    pub fn process_transaction(
        &self,
        ixs: &[Instruction],
        accounts: &[KeyedAccount],
        fee_payer: &Pubkey,
    ) -> HopperExecutionResult {
        let mut log_capture = LogCapture::default();
        // Compute fee.
        let metas_lists: Vec<&[solana_sdk::instruction::AccountMeta]> =
            ixs.iter().map(|ix| ix.accounts.as_slice()).collect();
        let num_signers = fees::count_unique_signers(&metas_lists);
        let cu_limit = self
            .pending_cu_limit
            .lock()
            .expect("pending_cu_limit")
            .unwrap_or_else(|| self.budget.lock().expect("budget").limit());
        let micro_lamports = self.priority_fee_micro_lamports_per_cu();
        let fc = self.fee_calculator();
        let total_fee = fees::total_fee(&fc, num_signers, cu_limit, micro_lamports);

        // Deduct fee up front.
        let mut state: Vec<KeyedAccount> = accounts.to_vec();
        let payer_idx = match state.iter().position(|a| &a.address == fee_payer) {
            Some(i) => i,
            None => {
                let outcome = ExecutionOutcome {
                    resulting_accounts: state,
                    compute_units_consumed: 0,
                    return_data: Vec::new(),
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: Some(HopperSvmError::UnknownAccount(*fee_payer)),
                };
                let mut result = HopperExecutionResult::from_outcome(outcome, log_capture);
                result.transaction_fee_paid = 0;
                return result;
            }
        };
        if state[payer_idx].lamports < total_fee {
            let bal = state[payer_idx].lamports;
            let outcome = ExecutionOutcome {
                resulting_accounts: state,
                compute_units_consumed: 0,
                return_data: Vec::new(),
                inner_instructions: Vec::new(),
                execution_time_us: 0,
                error: Some(HopperSvmError::InsufficientFunds {
                    account: *fee_payer,
                    balance: bal,
                    requested: total_fee,
                }),
            };
            let mut result = HopperExecutionResult::from_outcome(outcome, log_capture);
            result.transaction_fee_paid = 0;
            return result;
        }
        state[payer_idx].lamports -= total_fee;
        log_capture.line(format!(
            "Transaction: charged {total_fee} lamports from fee payer {fee_payer} ({num_signers} signers, {cu_limit} CU limit, {micro_lamports} μlamports/CU)"
        ));

        // Run the chain (fee already deducted; mainnet's anti-spam
        // rule says fees stay charged even if instructions fail).
        let mut final_outcome: Option<ExecutionOutcome> = None;
        for (i, ix) in ixs.iter().enumerate() {
            log_capture.section(&format!("ix[{i}] @ {}", ix.program_id));
            let outcome = self.dispatch_one(ix, &state, &mut log_capture);
            for upd in &outcome.resulting_accounts {
                if let Some(existing) = state.iter_mut().find(|a| a.address == upd.address) {
                    *existing = upd.clone();
                } else {
                    state.push(upd.clone());
                }
            }
            let failed = outcome.error.is_some();
            final_outcome = Some(outcome);
            if failed {
                break;
            }
        }
        let outcome = final_outcome.unwrap_or_else(|| ExecutionOutcome {
            resulting_accounts: state.clone(),
            compute_units_consumed: 0,
            return_data: Vec::new(),
            inner_instructions: Vec::new(),
            execution_time_us: 0,
            error: Some(HopperSvmError::EmptyChain),
        });
        let mut result = HopperExecutionResult::from_outcome(outcome, log_capture);
        result.transaction_fee_paid = total_fee;
        result
    }

    /// Process a slice of instructions atomically as a chain.
    /// Account state mutations carry forward between instructions,
    /// matching how a Solana transaction's instructions see each
    /// other's writes. The final outcome is the result of the last
    /// instruction; if any earlier instruction failed, the chain
    /// aborts and the result reflects that failure.
    pub fn process_instruction_chain(
        &self,
        ixs: &[Instruction],
        accounts: &[KeyedAccount],
    ) -> HopperExecutionResult {
        let mut log_capture = LogCapture::default();
        // Carry state forward: every successful step's resulting
        // accounts feed into the next step.
        let mut state: Vec<KeyedAccount> = accounts.to_vec();
        let mut final_outcome: Option<ExecutionOutcome> = None;
        for (i, ix) in ixs.iter().enumerate() {
            log_capture.section(&format!("ix[{i}] @ {}", ix.program_id));
            let outcome = self.dispatch_one(ix, &state, &mut log_capture);
            // Each step replaces the chain state with its post-state
            // accounts so the next step sees the writes. Accounts the
            // step didn't touch carry through unchanged.
            for upd in &outcome.resulting_accounts {
                if let Some(existing) = state.iter_mut().find(|a| a.address == upd.address) {
                    *existing = upd.clone();
                } else {
                    state.push(upd.clone());
                }
            }
            let failed = outcome.error.is_some();
            final_outcome = Some(outcome);
            if failed {
                break;
            }
        }
        let outcome = final_outcome.unwrap_or_else(|| ExecutionOutcome {
            resulting_accounts: state.clone(),
            compute_units_consumed: 0,
            return_data: Vec::new(),
            inner_instructions: Vec::new(),
            execution_time_us: 0,
            error: Some(HopperSvmError::EmptyChain),
        });
        HopperExecutionResult::from_outcome(outcome, log_capture)
    }

    /// Internal: dispatch a single instruction to the right
    /// engine. Phase 1 has only a built-in engine; Phase 2 adds a
    /// BPF engine fallback (feature-gated). Wall-clock timing is
    /// stamped onto the outcome's `execution_time_us` field; the
    /// inner `dispatch_one_inner` does the work, this wrapper
    /// just measures.
    fn dispatch_one(
        &self,
        ix: &Instruction,
        accounts: &[KeyedAccount],
        logs: &mut LogCapture,
    ) -> ExecutionOutcome {
        let start = std::time::Instant::now();
        let mut outcome = self.dispatch_one_inner(ix, accounts, logs);
        outcome.execution_time_us = start.elapsed().as_micros() as u64;
        outcome
    }

    /// Tier 3 dispatch: route the instruction through Agave's
    /// real runtime via the harness's installed [`agave::AgaveEngine`].
    /// Translates Hopper's [`KeyedAccount`] slice into Agave's
    /// `(Pubkey, AccountSharedData)` shape, runs the instruction,
    /// and translates the post-state back into [`KeyedAccount`]s.
    ///
    /// On success the returned [`ExecutionOutcome`] carries the
    /// post-state, return data, and consumed CU as if it had run
    /// through the inline engine — callers can't tell the difference
    /// at the API boundary, which is the whole point.
    #[cfg(feature = "agave-runtime")]
    fn dispatch_through_agave(
        &self,
        engine: &agave::AgaveEngine,
        ix: &Instruction,
        accounts: &[KeyedAccount],
        logs: &mut LogCapture,
    ) -> ExecutionOutcome {
        use solana_program_runtime::execution_budget::{
            SVMTransactionExecutionBudget, SVMTransactionExecutionCost,
        };
        use solana_sdk::account::{Account as SolanaAccount, ReadableAccount};

        // Translate KeyedAccount → (Pubkey, AccountSharedData). The
        // program account is appended at the end if not already in
        // the slice; Agave's runtime needs an account at
        // `program_indices[0]` whose owner is `native_loader::id`.
        let mut tx_accounts: Vec<(
            solana_sdk::pubkey::Pubkey,
            solana_sdk::account::AccountSharedData,
        )> = Vec::with_capacity(accounts.len() + 1);
        for ka in accounts {
            let acct = SolanaAccount {
                lamports: ka.lamports,
                data: ka.data.clone(),
                owner: ka.owner,
                executable: ka.executable,
                rent_epoch: ka.rent_epoch,
            };
            tx_accounts.push((ka.address, acct.into()));
        }

        // Find the program account index, or synthesise a stub.
        let program_index = match tx_accounts.iter().position(|(k, _)| k == &ix.program_id) {
            Some(i) => i as u16,
            None => {
                let mut stub = SolanaAccount::default();
                stub.executable = true;
                stub.owner = solana_sdk::native_loader::id();
                tx_accounts.push((ix.program_id, stub.into()));
                (tx_accounts.len() - 1) as u16
            }
        };

        let svm_sysvars = self.sysvars.lock().expect("sysvars lock").clone();
        let sysvar_cache = agave::AgaveEngine::build_sysvar_cache(&svm_sysvars);

        // Translate Hopper's CU budget into Agave's execution-budget
        // shape: only `compute_unit_limit` differs from the default.
        let mut exec_budget = SVMTransactionExecutionBudget::default();
        let cu_limit = self.budget.lock().expect("budget").limit();
        exec_budget.compute_unit_limit = cu_limit;

        let pre_addresses: Vec<solana_sdk::pubkey::Pubkey> =
            tx_accounts.iter().map(|(k, _)| *k).collect();

        match engine.process_instruction_raw(
            ix,
            tx_accounts,
            vec![program_index],
            &sysvar_cache,
            exec_budget,
            SVMTransactionExecutionCost::default(),
            solana_sdk::rent::Rent::default(),
        ) {
            Ok((cu, post)) => {
                logs.line(format!(
                    "Program {} consumed {cu} compute units (agave-runtime)",
                    ix.program_id
                ));
                let resulting_accounts = post
                    .into_iter()
                    .filter(|(k, _)| pre_addresses.contains(k))
                    .map(|(k, a)| KeyedAccount {
                        address: k,
                        lamports: a.lamports(),
                        data: a.data().to_vec(),
                        owner: *a.owner(),
                        executable: a.executable(),
                        rent_epoch: a.rent_epoch(),
                    })
                    .collect();
                ExecutionOutcome {
                    resulting_accounts,
                    compute_units_consumed: cu,
                    return_data: Vec::new(),
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: None,
                }
            }
            Err(err) => {
                logs.line(format!("agave-runtime: {err}"));
                ExecutionOutcome {
                    resulting_accounts: accounts.to_vec(),
                    compute_units_consumed: 0,
                    return_data: Vec::new(),
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: Some(crate::error::HopperSvmError::BuiltinError {
                        program_id: ix.program_id,
                        message: format!("{err}"),
                    }),
                }
            }
        }
    }

    /// Internal worker — same signature as the public dispatcher
    /// but without timing. Always returns an `ExecutionOutcome`
    /// with `execution_time_us = 0`; the wrapper stamps the real
    /// value.
    fn dispatch_one_inner(
        &self,
        ix: &Instruction,
        accounts: &[KeyedAccount],
        logs: &mut LogCapture,
    ) -> ExecutionOutcome {
        // Tier 3: when an Agave engine is installed and the program
        // is registered there, route through real `solana-program-runtime`
        // dispatch instead of the inline registry. The Agave path
        // matches mainnet semantics byte-for-byte; the inline registry
        // remains the fast path for built-in simulators that don't
        // need full validator fidelity.
        #[cfg(feature = "agave-runtime")]
        {
            let agave_clone = self.agave_engine.lock().expect("agave_engine").clone();
            if let Some(engine) = agave_clone {
                if engine.is_registered(&ix.program_id) {
                    return self.dispatch_through_agave(&engine, ix, accounts, logs);
                }
            }
        }

        // Try the built-in registry first. The system program and
        // any user-registered simulators live here.
        let registry = self.registry.lock().expect("registry lock");
        let program = registry.get(&ix.program_id).cloned();
        drop(registry);

        let mut budget = self.budget.lock().expect("budget lock").clone();
        let sysvars = self.sysvars.lock().expect("sysvars lock").clone();

        // Apply any pending CU-limit override written by an
        // earlier compute-budget instruction in the same chain.
        // This makes `ComputeBudgetInstruction::set_compute_unit_limit(N)`
        // take effect for the instructions that follow it,
        // matching mainnet's transaction-level budget semantics
        // within Hopper's instruction-by-instruction dispatch.
        if let Some(override_limit) = *self.pending_cu_limit.lock().expect("pending_cu_limit") {
            budget.set_limit(override_limit);
        }

        let policy = *self.validation_policy.lock().expect("validation_policy");

        if let Some(program) = program {
            let outcome = engine::BuiltinEngine.execute(
                program.as_ref(),
                ix,
                accounts,
                &mut budget,
                &sysvars,
                logs,
            );
            return self.apply_validation(ix, accounts, outcome, policy, logs);
        }

        // Phase 2 fall-through: BPF engine. Only present when the
        // `bpf-execution` feature is enabled. The CPI dispatcher
        // closure captures a clone of the harness so a BPF
        // program issuing `sol_invoke_signed_c` recursively
        // dispatches back through this same `dispatch_one` (one
        // depth deeper). The depth check inside `bpf::cpi`
        // bounds the recursion at `MAX_CPI_DEPTH`.
        //
        // The `inner_logs: &mut LogCapture` argument is the OUTER
        // instruction's log buffer — by passing it through to the
        // inner dispatch call, the inner program's
        // `invoke`/`success` framing and `Program log:` lines
        // append directly to the same transcript. The depth
        // counter on `LogCapture` already handles indentation
        // across the boundary.
        #[cfg(feature = "bpf-execution")]
        {
            let svm_for_cpi = self.clone();
            let dispatcher: bpf::context::CpiDispatcher = std::sync::Arc::new(
                move |inner_ix: &Instruction,
                      inner_accounts: Vec<KeyedAccount>,
                      inner_logs: &mut LogCapture| {
                    svm_for_cpi.dispatch_one(inner_ix, &inner_accounts, inner_logs)
                },
            );
            if let Some(outcome) = self.bpf_engine.try_execute(
                ix,
                accounts,
                &mut budget,
                &sysvars,
                logs,
                Some(dispatcher),
                1, // Outermost program at depth 1; CPI handler
                   // increments before recursing.
            ) {
                return self.apply_validation(ix, accounts, outcome, policy, logs);
            }
        }

        ExecutionOutcome {
            resulting_accounts: accounts.to_vec(),
            compute_units_consumed: 0,
            return_data: Vec::new(),
            inner_instructions: Vec::new(),
            execution_time_us: 0,
            error: Some(HopperSvmError::UnknownProgram(ix.program_id)),
        }
    }

    /// Run post-instruction account-state validation against an
    /// already-completed `ExecutionOutcome`. On rule violation,
    /// rewrites the outcome to roll back account mutations and
    /// surface a `HopperSvmError::AccountValidationFailed` so
    /// the test sees the failure with the offending account +
    /// reason. On success, the outcome passes through unchanged.
    ///
    /// Failed outcomes (the program errored) skip validation —
    /// the runtime already rolls back state on instruction
    /// failure, so there's no post-state to validate against.
    fn apply_validation(
        &self,
        ix: &Instruction,
        pre: &[KeyedAccount],
        mut outcome: ExecutionOutcome,
        policy: ValidationPolicy,
        logs: &mut LogCapture,
    ) -> ExecutionOutcome {
        if outcome.error.is_some() {
            return outcome;
        }
        match validation::validate_post_state(
            &ix.program_id,
            &ix.accounts,
            pre,
            &outcome.resulting_accounts,
            policy,
        ) {
            Ok(()) => outcome,
            Err(err) => {
                // Validation failed — log the reason on the
                // outer transcript so a snapshot test sees the
                // exact rule that fired, then roll back the
                // mutations and surface the structured error.
                logs.line(format!("Validation failed: {}", err.describe()));
                outcome.resulting_accounts = pre.to_vec();
                outcome.return_data = Vec::new();
                outcome.error = Some(err);
                outcome
            }
        }
    }
}

impl Default for HopperSvm {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Cross-cutting tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::system_instruction;

    /// SPL constants must equal the upstream IDs — guards against a
    /// silent drift of `spl-token::id()` that would point Hopper
    /// tests at a phantom program.
    #[test]
    fn spl_token_constants_match_upstream() {
        assert_eq!(SPL_TOKEN_PROGRAM_ID, spl_token::id());
        assert_eq!(SPL_TOKEN_2022_PROGRAM_ID, spl_token_2022::id());
        // spl_associated_token_account::id() is now typed as
        // `solana_address::Address`, distinct from the legacy
        // `solana_sdk::pubkey::Pubkey`. Compare bytes directly.
        assert_eq!(
            ASSOCIATED_TOKEN_PROGRAM_ID.to_bytes(),
            spl_associated_token_account::id().to_bytes(),
        );
    }

    /// End-to-end: a system-program transfer succeeds and leaves
    /// account balances correct. Exercises the full Phase 1 path —
    /// HopperSvm registry, BuiltinEngine, SystemProgram processor,
    /// account state carry-over, log capture.
    #[test]
    fn system_transfer_round_trip() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let svm = HopperSvm::new();
        let accounts = vec![
            token::create_keyed_system_account(&alice, 1_000_000),
            token::create_keyed_system_account(&bob, 0),
        ];
        let ix = system_instruction::transfer(&alice, &bob, 250_000);
        let result = svm.process_instruction(&ix, &accounts);

        assert!(result.is_success(), "logs: {}", result.all_logs());
        assert_eq!(result.account(&alice).unwrap().lamports, 750_000);
        assert_eq!(result.account(&bob).unwrap().lamports, 250_000);
        assert!(result.compute_units_consumed() > 0);
    }

    /// `simulate_instruction` returns the same outcome as
    /// `process_instruction` (stateless model means neither
    /// touches the input slice in place).
    #[test]
    fn simulate_instruction_matches_process_instruction() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let svm = HopperSvm::new();
        let accounts = vec![
            token::create_keyed_system_account(&alice, 1_000_000),
            token::create_keyed_system_account(&bob, 0),
        ];
        let ix = system_instruction::transfer(&alice, &bob, 250_000);

        let sim = svm.simulate_instruction(&ix, &accounts);
        let exec = svm.process_instruction(&ix, &accounts);

        assert_eq!(sim.is_success(), exec.is_success());
        assert_eq!(
            sim.account(&alice).unwrap().lamports,
            exec.account(&alice).unwrap().lamports,
        );
        assert_eq!(
            sim.account(&bob).unwrap().lamports,
            exec.account(&bob).unwrap().lamports,
        );
    }

    /// `warp_to_timestamp(t)` advances slot/epoch using the same
    /// 400ms-per-slot cadence that `warp_to_slot` reverses.
    #[test]
    fn warp_to_timestamp_advances_clock_and_slot() {
        let svm = HopperSvm::new();
        let initial = svm.sysvars().clock.unix_timestamp;
        // Forward by 60 seconds = 150 slots at 400 ms/slot.
        svm.warp_to_timestamp(initial + 60);
        let after = svm.sysvars().clock;
        assert_eq!(after.unix_timestamp, initial + 60);
        assert_eq!(after.slot, 150);
    }

    /// `airdrop` creates a fresh system-owned account and credits
    /// lamports; a second `airdrop` to the same address adds.
    #[test]
    fn airdrop_seeds_and_accumulates() {
        let svm = HopperSvm::new();
        let alice = Pubkey::new_unique();

        svm.airdrop(&alice, 5_000);
        let after_first = svm.get_account(&alice).expect("seeded");
        assert_eq!(after_first.lamports, 5_000);
        assert_eq!(after_first.owner, solana_sdk::system_program::id());
        assert_eq!(after_first.data.len(), 0);

        svm.airdrop(&alice, 2_500);
        assert_eq!(svm.get_account(&alice).unwrap().lamports, 7_500);
    }

    /// `create_account` allocates space, zero-initialises data, and
    /// derives the rent-exempt minimum from the configured rent
    /// sysvar. Replaces any existing entry at the same address.
    #[test]
    fn create_account_seeds_rent_exempt() {
        let svm = HopperSvm::new();
        let target = Pubkey::new_unique();
        let owner = Pubkey::new_unique();

        svm.create_account(&target, 165, &owner);
        let acct = svm.get_account(&target).expect("seeded");
        assert_eq!(acct.data.len(), 165);
        assert_eq!(acct.owner, owner);
        // The default rent sysvar charges a non-zero minimum for any
        // non-empty allocation; the exact value depends on the rate
        // but it must be greater than the zero-balance airdrop.
        assert!(acct.lamports > 0);
    }

    /// Stateful round trip: airdrop alice, set bob, run a transfer
    /// through the store, assert balances on the overlay.
    #[test]
    fn process_instruction_with_store_round_trip() {
        let svm = HopperSvm::new();
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();

        svm.airdrop(&alice, 1_000_000);
        svm.airdrop(&bob, 0);

        let ix = system_instruction::transfer(&alice, &bob, 250_000);
        let result = svm.process_instruction_with_store(&ix);
        assert!(result.is_success(), "logs: {}", result.all_logs());

        assert_eq!(svm.get_account(&alice).unwrap().lamports, 750_000);
        assert_eq!(svm.get_account(&bob).unwrap().lamports, 250_000);
    }

    /// **Tier 3 end-to-end**: a harness configured with
    /// `with_agave_runtime()` dispatches a system transfer through
    /// Agave's real `solana-program-runtime` rather than Hopper's
    /// inline system program. Same pre/post balances as the inline
    /// path so the verb is API-compatible.
    #[cfg(feature = "agave-runtime")]
    #[test]
    fn process_instruction_routes_through_agave_runtime() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let svm = HopperSvm::new().with_agave_runtime();
        let accounts = vec![
            token::create_keyed_system_account(&alice, 1_000_000),
            token::create_keyed_system_account(&bob, 0),
        ];
        let ix = system_instruction::transfer(&alice, &bob, 250_000);

        let result = svm.process_instruction(&ix, &accounts);
        assert!(result.is_success(), "logs: {}", result.all_logs());
        assert_eq!(result.account(&alice).unwrap().lamports, 750_000);
        assert_eq!(result.account(&bob).unwrap().lamports, 250_000);
        // Agave's system program declares a 150 CU baseline, distinct
        // from Hopper's inline simulator default.
        assert!(result.compute_units_consumed() >= 150);
        assert!(
            result.all_logs().contains("agave-runtime"),
            "expected agave-runtime tag in logs, got: {}",
            result.all_logs()
        );
    }

    /// `simulate_instruction_with_store` rolls back the overlay
    /// regardless of outcome, while still reporting the
    /// counterfactual result to the caller.
    #[test]
    fn simulate_with_store_does_not_commit() {
        let svm = HopperSvm::new();
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        svm.airdrop(&alice, 1_000_000);
        svm.airdrop(&bob, 0);

        let ix = system_instruction::transfer(&alice, &bob, 250_000);
        let probe = svm.simulate_instruction_with_store(&ix);
        assert!(probe.is_success());

        // Overlay is restored to pre-call state.
        assert_eq!(svm.get_account(&alice).unwrap().lamports, 1_000_000);
        assert_eq!(svm.get_account(&bob).unwrap().lamports, 0);
    }

    /// Unknown program IDs produce an error result, not a panic.
    /// This is the "did you forget to register your program" case
    /// authors hit in early test scaffolding; the failure mode
    /// should be a Result, not a process-killing panic.
    #[test]
    fn unknown_program_returns_error_not_panic() {
        let svm = HopperSvm::new();
        let bogus = Pubkey::new_unique();
        let ix = Instruction {
            program_id: bogus,
            accounts: vec![],
            data: vec![],
        };
        let result = svm.process_instruction(&ix, &[]);
        assert!(result.is_error());
        let msg = format!("{:?}", result.error().unwrap());
        assert!(msg.contains("UnknownProgram"), "got {msg}");
    }
}
