//! Error type for the Hopper SVM harness.
//!
//! `HopperSvmError` is the union of failure modes Phase 1 can emit.
//! Phase 2 will extend it with BPF-specific cases (verifier failure,
//! syscall trap, memory-region overflow, …); we deliberately don't
//! prefix the existing variants because the wire shape is part of
//! the public API and renaming variants is a breaking change.

use solana_sdk::pubkey::Pubkey;

/// A Hopper SVM execution failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HopperSvmError {
    /// The instruction's `program_id` is not registered as a
    /// built-in and Phase 2 BPF execution is not yet wired in.
    UnknownProgram(Pubkey),
    /// A built-in returned `Err` from its `invoke()` impl. The
    /// variant carries the program ID and the program's own error
    /// message so the failure reads as something the test author can
    /// act on.
    BuiltinError {
        /// The program that raised the error.
        program_id: Pubkey,
        /// The program's own error description.
        message: String,
    },
    /// The compute meter went below zero. Phase 1 charges a flat
    /// per-built-in cost (configurable via [`crate::ComputeBudget`]),
    /// Phase 2 will charge per-instruction.
    OutOfComputeUnits {
        /// CUs the program tried to consume past the limit.
        consumed: u64,
        /// The configured budget at the start of execution.
        limit: u64,
    },
    /// `process_instruction_chain` was called with an empty slice.
    EmptyChain,
    /// The instruction's account list referenced an account index
    /// the harness didn't recognise (e.g. a built-in told the
    /// harness to write to account 5 but only 3 were supplied).
    AccountIndexOutOfBounds {
        /// The index the program asked for.
        index: usize,
        /// How many accounts the harness actually has.
        len: usize,
    },
    /// The instruction's account list referenced a pubkey the
    /// harness didn't recognise (e.g. a built-in asked the system
    /// program to debit from a pubkey not in the account list).
    UnknownAccount(Pubkey),
    /// A built-in tried to take more lamports out of an account than
    /// it had. The error carries enough context to identify both the
    /// account and the requested amount.
    InsufficientFunds {
        /// The account the built-in tried to debit.
        account: Pubkey,
        /// The account's current balance.
        balance: u64,
        /// The amount the built-in tried to remove.
        requested: u64,
    },
    /// A built-in tried to mutate an account that wasn't passed as
    /// writable in the instruction's account metas.
    AccountNotWritable {
        /// The non-writable account the built-in tried to mutate.
        account: Pubkey,
    },
    /// A built-in tried to debit lamports from an account that
    /// wasn't passed as a signer in the instruction's account metas.
    AccountNotSigner {
        /// The non-signer account the built-in tried to debit.
        account: Pubkey,
    },
    /// A built-in raised a generic error not covered above. Used as
    /// a safety valve so a built-in that wants to surface a custom
    /// error code without adding a new variant can carry the error
    /// code through unchanged.
    Custom(u32),
    /// Post-instruction account-state validation tripped a rule.
    /// Mainnet's runtime enforces a handful of invariants between
    /// the program's effect on accounts and the metas + ownership
    /// declared on the instruction; a violation surfaces as a
    /// transaction failure on chain. Hopper's validator runs
    /// the same checks after each successful instruction so
    /// tests catch the bug locally instead of in production.
    AccountValidationFailed {
        /// The account whose post-state violated a rule.
        account: Pubkey,
        /// Short, human-readable description of which rule
        /// fired (e.g. `"data mutated by non-owner program"`,
        /// `"lamport conservation broken"`,
        /// `"executable flag toggled"`,
        /// `"read-only account modified"`,
        /// `"owner reassigned by non-owner program"`).
        reason: String,
    },
}

impl HopperSvmError {
    /// Short, human-readable description. Used by the result type's
    /// `assert_*` helpers when constructing panic messages.
    pub fn describe(&self) -> String {
        match self {
            Self::UnknownProgram(id) => format!("UnknownProgram({id})"),
            Self::BuiltinError {
                program_id,
                message,
            } => {
                format!("BuiltinError({program_id}): {message}")
            }
            Self::OutOfComputeUnits { consumed, limit } => {
                format!("OutOfComputeUnits(consumed={consumed}, limit={limit})")
            }
            Self::EmptyChain => "EmptyChain".to_string(),
            Self::AccountIndexOutOfBounds { index, len } => {
                format!("AccountIndexOutOfBounds(index={index}, len={len})")
            }
            Self::UnknownAccount(addr) => format!("UnknownAccount({addr})"),
            Self::InsufficientFunds {
                account,
                balance,
                requested,
            } => format!(
                "InsufficientFunds(account={account}, balance={balance}, requested={requested})"
            ),
            Self::AccountNotWritable { account } => {
                format!("AccountNotWritable({account})")
            }
            Self::AccountNotSigner { account } => {
                format!("AccountNotSigner({account})")
            }
            Self::Custom(code) => format!("Custom({code})"),
            Self::AccountValidationFailed { account, reason } => {
                format!("AccountValidationFailed(account={account}, reason={reason})")
            }
        }
    }
}

impl std::fmt::Display for HopperSvmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

impl std::error::Error for HopperSvmError {}
