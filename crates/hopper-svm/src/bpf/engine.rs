//! `BpfEngine` — Phase 2 execution engine.
//!
//! Loads `.so` files, sets up the `solana-sbpf` virtual machine,
//! routes syscalls through Hopper's [`super::syscalls`] surface,
//! and lifts the post-state out of the parameter buffer.
//!
//! ## Lifecycle
//!
//! ```text
//! HopperSvm::add_program(&id, "name")
//!   → reads target/deploy/<name>.so
//!   → BpfEngine::add_elf(&id, bytes)
//!
//! HopperSvm::process_instruction(&ix, &accounts)
//!   → dispatch_one
//!     → built-in registry miss
//!       → BpfEngine::try_execute(ix, accounts, budget, sysvars, logs)
//!         → serialize parameter buffer
//!         → set up MemoryMapping (input + heap + stack)
//!         → construct EbpfVm with BpfContext
//!         → run interpreter
//!         → deserialize parameter buffer
//!         → lift logs + return_data + post-state into ExecutionOutcome
//! ```
//!
//! ## Memory layout (matches the production runtime)
//!
//! - `MM_RODATA_START` (0x100000000) — program rodata segments
//!   (the executable manages these).
//! - `MM_STACK_START` (0x200000000) — VM stack.
//! - `MM_HEAP_START` (0x300000000) — VM heap.
//! - `MM_INPUT_START` (0x400000000) — parameter buffer (the
//!   serialized accounts + ix data + program ID).
//!
//! ## Phase 2.0 limitations
//!
//! - **No CPI.** `sol_invoke_signed_*` is not registered; programs
//!   that call it get a "syscall not found" error. Phase 2.1.
//! - **No sysvar reads.** `sol_get_*_sysvar` is not registered.
//!   Programs that need clock/rent should accept them as accounts
//!   in the meantime.
//! - **Interpreter only.** JIT is not enabled; the interpreter is
//!   correctness-equivalent and works on every platform sbpf
//!   supports.
//! - **Loader v3 / SBPFv3 default.** Older loaders work but are
//!   not the default — the engine constructs an `Executable` with
//!   the latest SBPF version sbpf 0.20 supports. For programs
//!   compiled against older loaders, set `BpfEngine::sbpf_version`
//!   before calling `try_execute`.

use crate::account::KeyedAccount;
use crate::bpf::adapters::*;
use crate::bpf::context::{BpfContext, CpiDispatcher};
use crate::bpf::parameter::{self, Parameters};
use crate::compute::ComputeBudget;
use crate::engine::ExecutionOutcome;
use crate::error::HopperSvmError;
use crate::log::LogCapture;
use crate::sysvar::Sysvars;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use solana_sbpf::elf::Executable;
use solana_sbpf::memory_region::{MemoryMapping, MemoryRegion};
use solana_sbpf::program::{BuiltinProgram, FunctionRegistry, SBPFVersion};
use solana_sbpf::vm::{Config, EbpfVm};

/// VM stack size. Matches the upstream runtime default
/// (`solana_program_runtime::compute_budget::DEFAULT_STACK_SIZE`)
/// so a Hopper test sees the same call-depth limits as production.
const STACK_SIZE: usize = 4_096 * 64; // 256 KiB

/// VM heap size. Matches the upstream default
/// (`solana_program_runtime::compute_budget::DEFAULT_HEAP_COST`'s
/// 32 KiB allowance). Programs that allocate beyond this through
/// `sol_alloc_free_` (Phase 2.1) get a heap-oom error.
const HEAP_SIZE: usize = 32 * 1024;

/// Which BPF loader owns a registered program.
///
/// Mainnet runs three generations:
///
/// - **V2** (`BPFLoader2111111111111111111111111111111111`): the
///   non-upgradeable loader. SPL Token (legacy), SPL Memo, SPL
///   Token-2022 are all deployed under V2. Account inputs are
///   serialized with the original (non-V3) layout, and the
///   program data lives in the program account itself.
/// - **V3 / Upgradeable** (`BPFLoaderUpgradeab1e11111111111111111111111`):
///   the upgradeable loader. Almost every newer program (Anchor
///   programs, custom protocols, Hopper programs) ships under V3.
///   Program data lives in a separate `ProgramData` account; the
///   program account stores a 36-byte header that points at it.
/// - **V4** (post-Agave): not yet shipped on mainnet at the
///   shipping date of this harness. When V4 lands the variant
///   will be added; existing call sites that pin V3 keep working.
///
/// The variant chosen at `add_program_with_loader` time controls
/// how the engine loads and presents the program to user code.
/// `Default::default()` is `V3` because that's the modern path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoaderKind {
    /// Non-upgradeable BPF Loader v2.
    V2,
    /// Upgradeable BPF Loader v3.
    V3,
}

impl Default for LoaderKind {
    fn default() -> Self {
        Self::V3
    }
}

/// Bundled program record: ELF bytes plus the loader they target.
#[derive(Clone)]
pub struct LoadedProgram {
    /// Raw ELF (`.so` contents).
    pub elf: Vec<u8>,
    /// Which loader the program is registered against. Drives
    /// account-input serialization quirks and the program-data
    /// fetch path during invocation.
    pub loader: LoaderKind,
}

/// Phase 2 BPF execution engine.
///
/// Holds a registry of program-id -> [`LoadedProgram`]. `add_elf`
/// populates it (typically driven by [`crate::HopperSvm::add_program`]);
/// `try_execute` looks up the program by `ix.program_id` and runs it.
#[derive(Clone, Default)]
pub struct BpfEngine {
    elfs: Arc<Mutex<HashMap<Pubkey, LoadedProgram>>>,
    /// SBPF version to assume when loading ELFs. Defaults to the
    /// latest sbpf 0.20 supports; override per-engine for older
    /// loaders.
    pub sbpf_version: SBPFVersion,
}

impl BpfEngine {
    /// Build a fresh engine with no programs loaded.
    pub fn new() -> Self {
        Self {
            elfs: Arc::new(Mutex::new(HashMap::new())),
            sbpf_version: SBPFVersion::V3,
        }
    }

    /// Register the bytes of a `.so` against a program ID. The
    /// loader defaults to [`LoaderKind::V3`] (the modern upgradeable
    /// loader); use [`add_elf_with_loader`] to pin V2 instead.
    pub fn add_elf(&self, program_id: &Pubkey, elf: Vec<u8>) {
        self.add_elf_with_loader(program_id, elf, LoaderKind::default());
    }

    /// Register a `.so` against a program ID under a specific loader.
    /// Mirrors Quasar's `add_program(id, loader, elf)`.
    pub fn add_elf_with_loader(
        &self,
        program_id: &Pubkey,
        elf: Vec<u8>,
        loader: LoaderKind,
    ) {
        self.elfs
            .lock()
            .expect("elfs lock")
            .insert(*program_id, LoadedProgram { elf, loader });
    }

    /// Read back the loader kind for a registered program. Used by
    /// the engine's invocation path to decide which account-input
    /// serialization to apply. Returns `None` when the program ID
    /// has no registered ELF.
    pub fn loader_for(&self, program_id: &Pubkey) -> Option<LoaderKind> {
        self.elfs
            .lock()
            .expect("elfs lock")
            .get(program_id)
            .map(|p| p.loader)
    }

    /// Try to execute an instruction. Returns `None` when this
    /// engine does not have an ELF for `ix.program_id`, letting
    /// the harness fall through to a different engine or surface
    /// `UnknownProgram`.
    ///
    /// `cpi_dispatcher` is the closure the BPF program will use
    /// for `sol_invoke_signed_*` recursion. `None` disables CPI
    /// (programs that issue a CPI under that path get a clean
    /// "FAILED" return code).
    pub fn try_execute(
        &self,
        ix: &Instruction,
        accounts: &[KeyedAccount],
        budget: &mut ComputeBudget,
        sysvars: &Sysvars,
        logs: &mut LogCapture,
        cpi_dispatcher: Option<CpiDispatcher>,
        cpi_depth: u32,
    ) -> Option<ExecutionOutcome> {
        let loaded = self
            .elfs
            .lock()
            .expect("elfs lock")
            .get(&ix.program_id)
            .cloned()?;
        let _loader = loaded.loader; // reserved for V2/V3 input-serialisation divergence
        Some(self.execute_loaded(
            loaded.elf,
            ix,
            accounts,
            budget,
            sysvars,
            logs,
            cpi_dispatcher,
            cpi_depth,
        ))
    }

    /// Run a specific ELF against the supplied state. Public so
    /// users that want to drive an ELF directly (without the
    /// program-id registry) can.
    pub fn execute_loaded(
        &self,
        elf: Vec<u8>,
        ix: &Instruction,
        accounts: &[KeyedAccount],
        budget: &mut ComputeBudget,
        sysvars: &Sysvars,
        logs: &mut LogCapture,
        cpi_dispatcher: Option<CpiDispatcher>,
        cpi_depth: u32,
    ) -> ExecutionOutcome {
        budget.reset();
        logs.invoke(&ix.program_id);

        // Serialize parameter buffer up front — if any meta
        // references an unknown account we fail fast without
        // touching the VM.
        let mut params = match parameter::serialize_parameters(
            &ix.accounts,
            accounts,
            &ix.data,
            &ix.program_id,
        ) {
            Ok(p) => p,
            Err(missing) => {
                let err = HopperSvmError::UnknownAccount(missing);
                logs.failure(
                    &ix.program_id,
                    budget.consumed(),
                    budget.limit(),
                    &err,
                );
                return ExecutionOutcome {
                    resulting_accounts: accounts.to_vec(),
                    compute_units_consumed: 0,
                    return_data: Vec::new(),
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: Some(err),
                };
            }
        };

        // Set up the syscall registry. Each adapter is registered
        // by name so the program's `extern "C" sol_log_(...)`
        // declarations resolve into the matching `BuiltinFunction`
        // entry.
        let loader = match build_loader(self.sbpf_version) {
            Ok(l) => l,
            Err(err) => return engine_error(ix, accounts, budget, logs, err),
        };

        // Load the ELF.
        let executable = match Executable::<BpfContext>::from_elf(&elf, loader) {
            Ok(e) => e,
            Err(err) => {
                let msg = format!("ELF parse failed: {err}");
                return engine_error(ix, accounts, budget, logs, msg);
            }
        };

        // Set up VM memory regions:
        //   stack  — fresh zeroed buffer
        //   heap   — fresh zeroed buffer
        //   input  — the serialized parameter buffer (writable so
        //            programs can mutate accounts in-place)
        let mut stack = vec![0u8; STACK_SIZE];
        let mut heap = vec![0u8; HEAP_SIZE];

        let regions = vec![
            executable.get_ro_region(),
            MemoryRegion::new_writable(&mut stack, solana_sbpf::ebpf::MM_STACK_START),
            MemoryRegion::new_writable(&mut heap, solana_sbpf::ebpf::MM_HEAP_START),
            MemoryRegion::new_writable(
                &mut params.buffer,
                solana_sbpf::ebpf::MM_INPUT_START,
            ),
        ];

        let memory_mapping = match MemoryMapping::new(
            regions,
            executable.get_config(),
            executable.get_sbpf_version(),
        ) {
            Ok(m) => m,
            Err(err) => {
                return engine_error(
                    ix,
                    accounts,
                    budget,
                    logs,
                    format!("memory mapping construction failed: {err}"),
                )
            }
        };

        // Build the per-instruction context with the harness's
        // remaining CU budget + a snapshot of the sysvar state
        // that `sol_get_*_sysvar` will read from.
        let mut context = BpfContext::new_with_sysvars(
            ix.program_id,
            budget.limit().saturating_sub(budget.consumed()),
            sysvars.clone(),
        );
        if let Some(dispatcher) = cpi_dispatcher {
            context = context.with_cpi(dispatcher, cpi_depth);
        }

        // Construct the VM and run the interpreter.
        let mut vm = EbpfVm::new(
            executable.get_loader().clone(),
            executable.get_sbpf_version(),
            &mut context,
            memory_mapping,
            STACK_SIZE,
        );

        let (instruction_count, result) =
            vm.execute_program(&executable, /* interpreter */ true);

        // Charge the cycles we burned (1 CU per instruction
        // matches Solana's per-instruction metering rate). The
        // remaining_units on the context already reflects syscall
        // charges; this catches the per-instruction CU consumption
        // the VM accumulated.
        let pre_charge = budget.consumed();
        if let Err(err) = budget.consume(instruction_count) {
            // Out of CUs at the per-instruction tally — surface
            // the structured error.
            for line in core::mem::take(&mut context.logs).into_lines() {
                logs.line(line);
            }
            logs.failure(
                &ix.program_id,
                budget.consumed(),
                budget.limit(),
                &err,
            );
            return ExecutionOutcome {
                resulting_accounts: accounts.to_vec(),
                compute_units_consumed: budget.consumed(),
                return_data: Vec::new(),
                inner_instructions: core::mem::take(&mut context.inner_instructions),
                execution_time_us: 0,
                error: Some(err),
            };
        }
        let _ = pre_charge;

        // Lift logs from the context into the harness buffer.
        for line in core::mem::take(&mut context.logs).into_lines() {
            logs.line(line);
        }

        match result {
            Ok(_return_value) => {
                logs.success(&ix.program_id, budget.consumed(), budget.limit());
                let post_state =
                    parameter::deserialize_parameters(&params, &ix.accounts, accounts);
                let return_data = context
                    .return_data
                    .map(|(_, data)| data)
                    .unwrap_or_default();
                ExecutionOutcome {
                    resulting_accounts: merge_accounts(accounts, &post_state),
                    compute_units_consumed: budget.consumed(),
                    return_data,
                    inner_instructions: core::mem::take(
                        &mut context.inner_instructions,
                    ),
                    execution_time_us: 0,
                    error: None,
                }
            }
            Err(vm_err) => {
                let err = if let Some(panic) = context.panic_message.clone() {
                    HopperSvmError::BuiltinError {
                        program_id: ix.program_id,
                        message: panic,
                    }
                } else {
                    HopperSvmError::BuiltinError {
                        program_id: ix.program_id,
                        message: format!("VM error: {vm_err}"),
                    }
                };
                logs.failure(
                    &ix.program_id,
                    budget.consumed(),
                    budget.limit(),
                    &err,
                );
                ExecutionOutcome {
                    // Roll back partial mutations on failure —
                    // matches Phase 1 / on-chain semantics.
                    resulting_accounts: accounts.to_vec(),
                    compute_units_consumed: budget.consumed(),
                    return_data: Vec::new(),
                    inner_instructions: core::mem::take(
                        &mut context.inner_instructions,
                    ),
                    execution_time_us: 0,
                    error: Some(err),
                }
            }
        }
    }
}

/// Wrap a free-form engine error into an [`ExecutionOutcome`] +
/// the matching log line. Used by paths that fail before VM
/// execution (ELF parse, memory mapping, …) so callers always get
/// a well-formed outcome.
fn engine_error(
    ix: &Instruction,
    accounts: &[KeyedAccount],
    budget: &ComputeBudget,
    logs: &mut LogCapture,
    message: impl Into<String>,
) -> ExecutionOutcome {
    let err = HopperSvmError::BuiltinError {
        program_id: ix.program_id,
        message: message.into(),
    };
    logs.failure(&ix.program_id, budget.consumed(), budget.limit(), &err);
    ExecutionOutcome {
        resulting_accounts: accounts.to_vec(),
        compute_units_consumed: budget.consumed(),
        return_data: Vec::new(),
        // Pre-VM-construction failure path — no CPIs ran yet.
        inner_instructions: Vec::new(),
        execution_time_us: 0,
        error: Some(err),
    }
}

/// Merge a BPF program's deserialized post-state back into the
/// full account list. Mirrors `engine::merge_accounts` but is
/// re-implemented locally because the engine module is
/// feature-independent.
fn merge_accounts(
    original: &[KeyedAccount],
    working: &[KeyedAccount],
) -> Vec<KeyedAccount> {
    let mut out: Vec<KeyedAccount> = original.to_vec();
    for w in working {
        match out.iter_mut().find(|a| a.address == w.address) {
            Some(slot) => *slot = w.clone(),
            None => out.push(w.clone()),
        }
    }
    out
}

/// Build the syscall loader — a `BuiltinProgram<BpfContext>` with
/// every Phase 2.0 syscall registered by name. Phase 2.1 will add
/// `sol_invoke_signed_*` and the `sol_get_*_sysvar` family;
/// missing-syscall errors at runtime are the cleanest way to
/// surface "you need Phase 2.1" until that work lands.
fn build_loader(
    sbpf_version: SBPFVersion,
) -> Result<Arc<BuiltinProgram<BpfContext>>, String> {
    let mut function_registry = FunctionRegistry::<BuiltinFunctionPointer>::default();
    let mut register = |name: &str, f: BuiltinFunctionPointer| -> Result<(), String> {
        function_registry
            .register_function_hashed(name.as_bytes(), f)
            .map(|_| ())
            .map_err(|e| format!("syscall registration `{name}` failed: {e}"))
    };

    register("sol_log_", SyscallSolLog::vm)?;
    register("sol_log_64_", SyscallSolLog64::vm)?;
    register("sol_log_pubkey", SyscallSolLogPubkey::vm)?;
    register("sol_log_compute_units_", SyscallSolLogComputeUnits::vm)?;
    register("sol_panic_", SyscallSolPanic::vm)?;
    register("sol_memcpy_", SyscallSolMemcpy::vm)?;
    register("sol_memset_", SyscallSolMemset::vm)?;
    register("sol_memcmp_", SyscallSolMemcmp::vm)?;
    register("sol_memmove_", SyscallSolMemmove::vm)?;
    register("sol_set_return_data", SyscallSolSetReturnData::vm)?;
    register("sol_get_return_data", SyscallSolGetReturnData::vm)?;
    register("sol_log_data", SyscallSolLogData::vm)?;
    // ── Phase 2.1: PDA derivation ─────────────────────────────────
    register(
        "sol_create_program_address",
        SyscallSolCreateProgramAddress::vm,
    )?;
    register(
        "sol_try_find_program_address",
        SyscallSolTryFindProgramAddress::vm,
    )?;
    // ── Phase 2.1: sysvar fetches ─────────────────────────────────
    register("sol_get_clock_sysvar", SyscallSolGetClockSysvar::vm)?;
    register("sol_get_rent_sysvar", SyscallSolGetRentSysvar::vm)?;
    register(
        "sol_get_epoch_schedule_sysvar",
        SyscallSolGetEpochScheduleSysvar::vm,
    )?;
    register(
        "sol_get_last_restart_slot_sysvar",
        SyscallSolGetLastRestartSlotSysvar::vm,
    )?;
    register(
        "sol_get_epoch_rewards_sysvar",
        SyscallSolGetEpochRewardsSysvar::vm,
    )?;
    // ── Phase 2.1: heap allocator ─────────────────────────────────
    register("sol_alloc_free_", SyscallSolAllocFree::vm)?;
    // ── Phase 2.1: crypto ─────────────────────────────────────────
    register("sol_keccak256_", SyscallSolKeccak256::vm)?;
    register("sol_blake3", SyscallSolBlake3::vm)?;
    register("sol_secp256k1_recover_", SyscallSolSecp256k1Recover::vm)?;
    // ── Phase 2.1: CPI ────────────────────────────────────────────
    register("sol_invoke_signed_c", SyscallSolInvokeSignedC::vm)?;
    register("sol_invoke_signed_rust", SyscallSolInvokeSignedRust::vm)?;
    // ── Tier 3: introspection ─────────────────────────────────────
    register("sol_get_stack_height", SyscallSolGetStackHeight::vm)?;
    register(
        "sol_remaining_compute_units",
        SyscallSolRemainingComputeUnits::vm,
    )?;
    register(
        "sol_get_processed_sibling_instruction",
        SyscallSolGetProcessedSiblingInstruction::vm,
    )?;
    // ── Tier 3: generic + obsolete sysvars ────────────────────────
    register("sol_get_sysvar", SyscallSolGetSysvar::vm)?;
    register(
        "sol_get_slothashes_sysvar",
        SyscallSolGetSlotHashesSysvar::vm,
    )?;
    register(
        "sol_get_slothistory_sysvar",
        SyscallSolGetSlotHistorySysvar::vm,
    )?;
    register(
        "sol_get_stakehistory_sysvar",
        SyscallSolGetStakeHistorySysvar::vm,
    )?;
    // ── Tier 3: curve25519 ────────────────────────────────────────
    register(
        "sol_curve_validate_point",
        SyscallSolCurveValidatePoint::vm,
    )?;
    register("sol_curve_group_op", SyscallSolCurveGroupOp::vm)?;
    // ── Tier 3: heavy-crypto stubs (Tier 4 work) ─────────────────
    register("sol_poseidon", SyscallSolPoseidon::vm)?;
    register("sol_big_mod_exp", SyscallSolBigModExp::vm)?;
    register("sol_alt_bn128_group_op", SyscallSolAltBn128GroupOp::vm)?;
    register(
        "sol_alt_bn128_compression",
        SyscallSolAltBn128Compression::vm,
    )?;

    let config = Config {
        max_call_depth: 64,
        stack_frame_size: 4096,
        enable_address_translation: true,
        enable_stack_frame_gaps: true,
        instruction_meter_checkpoint_distance: 10000,
        enable_instruction_meter: true,
        enable_instruction_tracing: false,
        enable_symbol_and_section_labels: false,
        reject_broken_elfs: true,
        noop_instruction_rate: 256,
        sanitize_user_provided_values: true,
        external_internal_function_hash_collision: false,
        reject_callx_r10: true,
        enable_sbpf_v0: matches!(sbpf_version, SBPFVersion::V0),
        enable_sbpf_v1: matches!(sbpf_version, SBPFVersion::V1),
        enable_sbpf_v2: matches!(sbpf_version, SBPFVersion::V2),
        enable_sbpf_v3: matches!(sbpf_version, SBPFVersion::V3),
        ..Config::default()
    };
    Ok(Arc::new(BuiltinProgram::new_loader(config, function_registry)))
}

/// Function-pointer alias matching what `declare_builtin_function!`
/// emits for the `vm` adapter (the entry point sbpf calls).
type BuiltinFunctionPointer = solana_sbpf::program::BuiltinFunction<BpfContext>;

#[cfg(test)]
mod tests {
    use super::*;

    /// `merge_accounts` carries through untouched accounts and
    /// applies modified accounts. Pin the merge semantics that
    /// the rest of the engine relies on.
    #[test]
    fn merge_accounts_replaces_and_appends() {
        let a1 = KeyedAccount::new(
            Pubkey::new_unique(),
            10,
            Pubkey::default(),
            vec![1],
            false,
        );
        let a2 = KeyedAccount::new(
            Pubkey::new_unique(),
            20,
            Pubkey::default(),
            vec![2],
            false,
        );
        let original = vec![a1.clone(), a2.clone()];
        // Modified: a1's lamports change. New: a3 didn't exist before.
        let mut a1_mut = a1.clone();
        a1_mut.lamports = 99;
        let a3 = KeyedAccount::new(
            Pubkey::new_unique(),
            30,
            Pubkey::default(),
            vec![3],
            false,
        );
        let working = vec![a1_mut.clone(), a3.clone()];
        let merged = merge_accounts(&original, &working);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].lamports, 99);
        assert_eq!(merged[1].lamports, 20);
        assert_eq!(merged[2].lamports, 30);
    }

    /// `BpfEngine::try_execute` returns `None` for a program ID
    /// it doesn't have an ELF for. The harness uses this signal
    /// to surface `UnknownProgram` rather than an ELF-parse
    /// error.
    #[test]
    fn try_execute_returns_none_when_elf_missing() {
        let engine = BpfEngine::new();
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let ix = Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![],
        };
        let result = engine.try_execute(
            &ix,
            &[],
            &mut budget,
            &sysvars,
            &mut logs,
            None,
            1,
        );
        assert!(result.is_none());
    }
}
