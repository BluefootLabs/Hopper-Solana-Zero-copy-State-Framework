//! `BpfContext` — the runtime context handed to `solana-sbpf`'s VM.
//!
//! The `solana_sbpf::vm::ContextObject` trait is the seam where the
//! BPF interpreter calls back into the host: every executed
//! instruction can `trace`, every syscall calls `consume` to charge
//! its CU cost, and `get_remaining` lets the host inspect the
//! meter.
//!
//! `BpfContext` is Hopper's impl. It carries:
//!
//! - The Hopper log buffer — syscall handlers
//!   (`sol_log_*`, `sol_panic_`) push lines into it, the harness
//!   reads it back into [`crate::HopperExecutionResult::logs`] after
//!   execution.
//! - The CU meter, mirrored from [`crate::ComputeBudget`] at
//!   instruction-start. After the VM returns we copy the residual
//!   meter back into the harness budget so chained instructions see
//!   the up-to-date count.
//! - The program ID being invoked, captured so syscalls that need
//!   self-reference (`sol_log_compute_units`'s framing line) can
//!   format correctly.
//! - An optional `return_data` slot — `sol_set_return_data` writes
//!   into it; the harness reads it into [`crate::engine::ExecutionOutcome::return_data`].
//!
//! ## Trait shape assumption
//!
//! `solana-sbpf 0.20`'s `ContextObject` trait is structured as
//! `trace` + `consume` + `get_remaining`. If a future minor bumps
//! the trait surface, this is the file that needs updating; the
//! rest of the engine consumes `BpfContext` only through `consume`
//! and the public fields.

use crate::account::KeyedAccount;
use crate::engine::ExecutionOutcome;
use crate::log::LogCapture;
use crate::sysvar::Sysvars;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

/// Maximum CPI invocation depth — mainnet caps the call stack at
/// 4 nested instructions (the original transaction is depth 1; a
/// CPI from there is depth 2; etc). We mirror the cap so a Hopper
/// test that recurses too deeply sees the same structured
/// failure mainnet would produce.
pub const MAX_CPI_DEPTH: u32 = 4;

/// Closure type for the CPI dispatcher — populated by the engine
/// at instruction start, called by [`crate::bpf::cpi`] when a BPF
/// program issues `sol_invoke_signed_c`. The closure captures a
/// clone of the harness so the recursion can run without
/// thread-local global state.
///
/// The third argument is the **outer** instruction's log buffer —
/// the inner program's `Program <id> invoke [N+1]` / `Program log:`
/// / `Program <id> success` lines are appended directly to it,
/// so the test sees one coherent transcript across the call
/// boundary. `LogCapture::invoke` and `success` already track
/// depth; sharing the buffer is what makes the depth tracking
/// useful.
pub type CpiDispatcher =
    Arc<dyn Fn(&Instruction, Vec<KeyedAccount>, &mut LogCapture) -> ExecutionOutcome + Send + Sync>;

/// Per-execution context for the BPF VM.
///
/// One `BpfContext` is constructed per instruction, lives for the
/// duration of that instruction's VM run, and is consumed back into
/// the harness when the VM returns. Not reused across instructions
/// — each fresh dispatch builds a new one.
#[derive(Debug)]
pub struct BpfContext {
    /// Remaining CU budget. Starts at the harness's
    /// `ComputeBudget::limit() - ComputeBudget::consumed()`,
    /// decrements on every syscall via [`consume`].
    pub remaining_units: u64,
    /// Captured logs. Syscall handlers append; the harness lifts
    /// the line vector into the public result.
    pub logs: LogCapture,
    /// The program ID this context is executing under. Read by
    /// log-framing syscalls.
    pub program_id: Pubkey,
    /// Most recent `sol_set_return_data` payload, if the program
    /// called it. The first member is the program ID that set the
    /// data (used by callers to verify provenance).
    pub return_data: Option<(Pubkey, Vec<u8>)>,
    /// Optional panic message captured from a `sol_panic_` syscall.
    /// When `Some`, the engine maps it to a [`crate::HopperSvmError::BuiltinError`]
    /// so the test sees a structured failure instead of a generic
    /// VM trap.
    pub panic_message: Option<String>,
    /// Snapshot of harness sysvar state at instruction start.
    /// `sol_get_*_sysvar` syscalls read from here. Cloned in so
    /// programs see a consistent view even if outer test code
    /// mutates the harness sysvars between calls.
    pub sysvars: Sysvars,
    /// Heap bump-allocator cursor — bytes consumed since the
    /// start of the heap region. `sol_alloc_free_` reads + writes
    /// this to hand out monotonically-increasing addresses inside
    /// the VM's heap memory region. Starts at 0; once it exceeds
    /// the heap size, subsequent allocations return null. Free
    /// is a no-op (matches upstream's bump-allocator semantics).
    pub heap_cursor: u64,
    /// Recursive CPI dispatcher. `None` when the engine wasn't
    /// configured for CPI (Phase 1 path); `Some` for any BPF
    /// execution that has access to the harness. The closure
    /// dispatches a fully-formed inner instruction back through
    /// `HopperSvm::dispatch_one` and returns the resulting
    /// outcome.
    pub cpi_dispatcher: Option<CpiDispatcher>,
    /// Current CPI nesting depth. Outermost program runs at depth
    /// 1; each `sol_invoke_signed_*` increments. The
    /// [`MAX_CPI_DEPTH`] limit is enforced before the recursive
    /// dispatch so we can't run away.
    pub cpi_depth: u32,
    /// Cross-Program Invocations recorded during this VM run, in
    /// dispatch order. `bpf::cpi::dispatch_cpi` appends an entry
    /// for every successful inner call. The engine takes the
    /// vector by `core::mem::take` at unwind time and lifts it
    /// into [`crate::engine::ExecutionOutcome::inner_instructions`].
    pub inner_instructions: Vec<crate::engine::InnerInstruction>,
}

impl BpfContext {
    /// Build a fresh context for one instruction.
    pub fn new(program_id: Pubkey, remaining_units: u64) -> Self {
        Self::new_with_sysvars(program_id, remaining_units, Sysvars::default())
    }

    /// Build a fresh context with explicit sysvar state — used by
    /// the engine so each VM run sees the harness's configured
    /// `Sysvars` snapshot.
    pub fn new_with_sysvars(program_id: Pubkey, remaining_units: u64, sysvars: Sysvars) -> Self {
        Self {
            remaining_units,
            logs: LogCapture::default(),
            program_id,
            return_data: None,
            panic_message: None,
            sysvars,
            heap_cursor: 0,
            cpi_dispatcher: None,
            cpi_depth: 1,
            inner_instructions: Vec::new(),
        }
    }

    /// Configure the CPI dispatcher and depth. The engine calls
    /// this after constructing the base context so syscall handlers
    /// see a fully-wired CPI path.
    pub fn with_cpi(mut self, dispatcher: CpiDispatcher, depth: u32) -> Self {
        self.cpi_dispatcher = Some(dispatcher);
        self.cpi_depth = depth;
        self
    }

    /// Total CUs consumed so far. Computed as
    /// `initial_budget - remaining_units` by the engine, which is
    /// the only site that knows the initial value; we don't track
    /// it on the context itself to keep the struct small.
    pub fn consumed_relative_to(&self, initial_budget: u64) -> u64 {
        initial_budget.saturating_sub(self.remaining_units)
    }
}

// ---------------------------------------------------------------------------
// `solana_sbpf::vm::ContextObject` impl
// ---------------------------------------------------------------------------
//
// The impl is feature-gated through the parent `bpf` module which
// is itself behind `#[cfg(feature = "bpf-execution")]`, so the
// `solana_sbpf` types are always in scope when this code compiles.
//
// Trait methods (per `solana_sbpf::vm::ContextObject` 0.20):
//
//   fn trace(&mut self, state: &[u64; 12]);
//   fn consume(&mut self, amount: u64);
//   fn get_remaining(&self) -> u64;
//
// `trace` is called once per executed instruction *only* when the
// VM is configured with tracing enabled (off by default for our
// test harness). `consume` is called by every syscall and by the
// per-instruction meter when the VM is configured to charge
// per-instruction CUs. `get_remaining` is the host's read access
// to the meter.

impl solana_sbpf::vm::ContextObject for BpfContext {
    fn trace(&mut self, _state: &[u64; 12]) {
        // Tracing is off by default. If a debug feature ever turns
        // it on we'd push a formatted register-state line into
        // `logs`, but it's not part of Phase 2.0 — instructions in
        // a Hopper test would emit thousands of trace lines and
        // overwhelm the snapshot stability the log buffer is for.
    }

    fn consume(&mut self, amount: u64) {
        self.remaining_units = self.remaining_units.saturating_sub(amount);
    }

    fn get_remaining(&self) -> u64 {
        self.remaining_units
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sbpf::vm::ContextObject;

    /// `consume` decrements the meter, saturating at zero so
    /// downstream `get_remaining` calls never underflow.
    #[test]
    fn consume_decrements_and_saturates() {
        let pid = Pubkey::new_unique();
        let mut ctx = BpfContext::new(pid, 100);
        ctx.consume(30);
        assert_eq!(ctx.get_remaining(), 70);
        ctx.consume(80);
        // Saturated at 0 — no underflow even though 80 > 70.
        assert_eq!(ctx.get_remaining(), 0);
        ctx.consume(1);
        assert_eq!(ctx.get_remaining(), 0);
    }

    /// `consumed_relative_to` is the inverse — given the initial
    /// budget the engine started with, report how much we've
    /// burned. Pin the arithmetic.
    #[test]
    fn consumed_relative_to_inverts_remaining() {
        let pid = Pubkey::new_unique();
        let mut ctx = BpfContext::new(pid, 200_000);
        ctx.consume(1_500);
        assert_eq!(ctx.consumed_relative_to(200_000), 1_500);
    }

    /// `trace` is a no-op in Phase 2.0 — assert that calling it
    /// doesn't perturb the meter. Prevents a future change to
    /// trace from accidentally double-charging through the meter.
    #[test]
    fn trace_does_not_perturb_meter() {
        let pid = Pubkey::new_unique();
        let mut ctx = BpfContext::new(pid, 100);
        let dummy_state = [0u64; 12];
        ctx.trace(&dummy_state);
        assert_eq!(ctx.get_remaining(), 100);
    }
}
