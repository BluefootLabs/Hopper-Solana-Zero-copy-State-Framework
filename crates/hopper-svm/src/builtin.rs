//! Built-in program registry — Hopper-native.
//!
//! `BuiltinProgram` is the trait every built-in implements. The
//! [`crate::HopperSvm::with_builtin`] builder registers an instance
//! against a program ID; subsequent `process_instruction` calls
//! whose `program_id` matches that ID dispatch into the built-in's
//! [`invoke`] method.
//!
//! ## Why expose this?
//!
//! Three reasons:
//!
//! 1. **System program**: Solana's system program is shipped as a
//!    built-in, not a BPF program. Phase 1 of the harness ships its
//!    own implementation in [`crate::system_program`] so transfers,
//!    account creation, etc. work without needing the BPF executor.
//! 2. **Test seams**: Hopper authors writing unit tests sometimes
//!    want a hand-written Rust simulator of their program — fast
//!    iteration, no `cargo build-sbf` cycle. Implementing
//!    `BuiltinProgram` for a test simulator and registering it under
//!    your program's ID gives you that, with no special-casing in
//!    the harness.
//! 3. **Custom built-ins for fault injection**: tests that need to
//!    simulate edge cases the real program can't easily produce
//!    (insufficient lamports, wrong owner, …) can register a custom
//!    built-in for the duration of one test.
//!
//! ## Contract
//!
//! `invoke` receives:
//!
//! - The instruction data (after the program ID is matched).
//! - A mutable view of the accounts in the order the instruction's
//!   `account_metas` listed them.
//! - An [`InvokeContext`] that carries the sysvar state, log buffer,
//!   and (Phase 2) a CPI dispatcher.
//!
//! On success, the built-in returns `Ok(())` and the harness reads
//! the post-state from the mutated account vector. On failure, the
//! returned error is propagated into [`crate::HopperExecutionResult`].

use crate::account::KeyedAccount;
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use crate::log::LogCapture;
use crate::sysvar::Sysvars;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;

/// Per-invocation context handed to a built-in. Read-only access to
/// the harness's sysvar state, mutable access to the log buffer,
/// and the resolved program ID for self-referential operations.
pub struct InvokeContext<'a> {
    /// The program ID of the built-in being invoked.
    pub program_id: &'a Pubkey,
    /// Account metas as supplied in the instruction. Index-aligned
    /// with the `accounts` slice the harness hands the built-in.
    pub account_metas: &'a [AccountMeta],
    /// Read-only sysvar state. Built-ins that need to read clock /
    /// rent see the values that were configured on the harness at
    /// the moment this instruction started.
    pub sysvars: &'a Sysvars,
    /// Log buffer — built-ins write `Program log:` lines through
    /// [`LogCapture::program_log`].
    pub logs: &'a mut LogCapture,
    /// Compute meter — built-ins charge their own CU cost via
    /// [`ComputeBudget::consume`].
    pub budget: &'a mut ComputeBudget,
}

impl InvokeContext<'_> {
    /// Lookup the metadata for a given account address, returning
    /// the matching `AccountMeta` if it was supplied. Useful for
    /// built-ins that need to check signer / writable flags before
    /// touching an account.
    pub fn meta_for(&self, addr: &Pubkey) -> Option<&AccountMeta> {
        self.account_metas.iter().find(|m| &m.pubkey == addr)
    }

    /// Convenience: assert an account was passed as a signer.
    /// Returns [`HopperSvmError::AccountNotSigner`] otherwise.
    pub fn require_signer(&self, addr: &Pubkey) -> Result<(), HopperSvmError> {
        match self.meta_for(addr) {
            Some(m) if m.is_signer => Ok(()),
            _ => Err(HopperSvmError::AccountNotSigner { account: *addr }),
        }
    }

    /// Convenience: assert an account was passed as writable.
    /// Returns [`HopperSvmError::AccountNotWritable`] otherwise.
    pub fn require_writable(&self, addr: &Pubkey) -> Result<(), HopperSvmError> {
        match self.meta_for(addr) {
            Some(m) if m.is_writable => Ok(()),
            _ => Err(HopperSvmError::AccountNotWritable { account: *addr }),
        }
    }

    /// Emit a log line at the canonical `Program log:` prefix.
    pub fn log(&mut self, msg: impl AsRef<str>) {
        self.logs.program_log(msg);
    }
}

/// Trait every built-in program implements.
///
/// The trait is `Send + Sync` so a `HopperSvm` can be cloned across
/// threads (each clone shares the same Arc'd registry).
pub trait BuiltinProgram: Send + Sync {
    /// Stable program name. Used in log framing — the runtime emits
    /// the program ID, not the name, so this is purely informational
    /// for Hopper's debug-log lines.
    fn name(&self) -> &'static str;

    /// CU cost charged at invoke time. Default is the harness's
    /// configured `default_builtin_cost` (150 CU). Override for
    /// programs whose cost should be fixed regardless of the
    /// caller's harness config.
    fn cost(&self, budget: &ComputeBudget) -> u64 {
        budget.default_builtin_cost()
    }

    /// Execute the instruction. The `accounts` slice is pre-resolved
    /// in the order the instruction's `account_metas` listed them
    /// — match the built-in's expectations to that order.
    fn invoke(
        &self,
        data: &[u8],
        accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError>;
}
