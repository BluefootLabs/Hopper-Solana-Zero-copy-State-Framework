//! Transaction-level fee accounting.
//!
//! Solana charges a transaction-wide fee at the start of every
//! transaction, deducted from a designated **fee payer** (the
//! first account of the transaction's account list, by
//! convention). The fee is deducted **even if the transaction
//! fails** — this is the runtime's anti-spam mechanism.
//!
//! ## Fee formula
//!
//! ```text
//! total_fee = base_fee + priority_fee
//! base_fee     = lamports_per_signature × num_unique_signers
//! priority_fee = compute_unit_limit × micro_lamports_per_cu / 1_000_000
//! ```
//!
//! Where:
//!
//! - `lamports_per_signature` defaults to **5000** on mainnet
//!   (configurable on genesis; we ship the mainnet default).
//! - `num_unique_signers` is the count of distinct pubkeys
//!   that appear with `is_signer = true` across all
//!   instruction account-metas in the transaction.
//! - `micro_lamports_per_cu` is set by
//!   `ComputeBudgetInstruction::set_compute_unit_price(N)` —
//!   updated by Hopper's compute-budget program simulator and
//!   read here.
//! - `compute_unit_limit` is the per-transaction CU ceiling
//!   (200 000 default; raised by
//!   `SetComputeUnitLimit`).
//!
//! ## Hopper API shape
//!
//! - `process_transaction(ixs, accounts, fee_payer)` — runs
//!   the chain with fee accounting. Deducts the fee from the
//!   fee payer up front, runs every instruction, returns the
//!   final outcome with `transaction_fee_paid` populated.
//! - `process_instruction_chain` keeps the no-fee semantics
//!   for fast unit tests that don't care about transaction
//!   wiring.

use solana_sdk::pubkey::Pubkey;

/// Default lamports per signature on mainnet. Hard-coded since
/// it hasn't changed since genesis. Can be overridden via
/// [`crate::HopperSvm::set_fee_calculator`].
pub const DEFAULT_LAMPORTS_PER_SIGNATURE: u64 = 5_000;

/// Fee-related state for a Hopper SVM harness. Mirrors the
/// shape `solana_sdk::fee_calculator::FeeCalculator` had before
/// it was deprecated, plus the priority-fee surcharge tracking
/// that replaces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeCalculator {
    /// Base fee charged per transaction signature.
    pub lamports_per_signature: u64,
}

impl Default for FeeCalculator {
    fn default() -> Self {
        Self {
            lamports_per_signature: DEFAULT_LAMPORTS_PER_SIGNATURE,
        }
    }
}

impl FeeCalculator {
    /// Compute the base fee for a given number of unique
    /// signers. Saturating multiply — programs with absurd
    /// signer counts get the cap rather than overflow.
    pub fn base_fee(&self, num_signers: u64) -> u64 {
        self.lamports_per_signature.saturating_mul(num_signers)
    }
}

/// Compute the priority fee for a given compute budget +
/// price. Returns 0 if either is zero.
///
/// `priority_fee = compute_unit_limit × micro_lamports_per_cu / 1_000_000`
///
/// Saturating arithmetic prevents overflow; division-by-1M is
/// integer division so any sub-lamport remainder is truncated
/// (matches mainnet exactly — fees can't be fractional).
pub fn priority_fee(compute_unit_limit: u64, micro_lamports_per_cu: u64) -> u64 {
    let scaled = compute_unit_limit.saturating_mul(micro_lamports_per_cu);
    scaled / 1_000_000
}

/// Count the number of unique signers across a list of
/// instruction account-metas. The transaction-fee formula
/// charges per unique signer pubkey, not per signer slot.
pub fn count_unique_signers(metas_lists: &[&[solana_sdk::instruction::AccountMeta]]) -> u64 {
    let mut seen: Vec<Pubkey> = Vec::new();
    for metas in metas_lists {
        for m in metas.iter() {
            if m.is_signer && !seen.contains(&m.pubkey) {
                seen.push(m.pubkey);
            }
        }
    }
    seen.len() as u64
}

/// Compute the total transaction fee for a given chain.
/// Convenience wrapper that combines [`FeeCalculator::base_fee`]
/// + [`priority_fee`].
pub fn total_fee(
    fee_calculator: &FeeCalculator,
    num_unique_signers: u64,
    compute_unit_limit: u64,
    micro_lamports_per_cu: u64,
) -> u64 {
    fee_calculator
        .base_fee(num_unique_signers)
        .saturating_add(priority_fee(compute_unit_limit, micro_lamports_per_cu))
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::instruction::AccountMeta;

    /// Default mainnet rate is 5000 lamports/signature.
    #[test]
    fn default_fee_calculator_matches_mainnet() {
        let f = FeeCalculator::default();
        assert_eq!(f.lamports_per_signature, 5_000);
        assert_eq!(f.base_fee(1), 5_000);
        assert_eq!(f.base_fee(3), 15_000);
        assert_eq!(f.base_fee(0), 0);
    }

    /// Priority fee is integer division — no fractional
    /// remainders. Pin against silent rounding.
    #[test]
    fn priority_fee_is_integer_division() {
        // 1M μlamports/CU × 200K CU = 200_000_000_000 μlamports
        // = 200_000 lamports.
        assert_eq!(priority_fee(200_000, 1_000_000), 200_000);
        // Sub-lamport: 999 μlamports/CU × 1 CU = 999 μlamports
        // = 0 lamports (truncated).
        assert_eq!(priority_fee(1, 999), 0);
        // No price → no priority fee.
        assert_eq!(priority_fee(200_000, 0), 0);
        // No CUs → no priority fee.
        assert_eq!(priority_fee(0, 1_000), 0);
    }

    /// Unique-signer counting dedupes across instructions: if
    /// alice signs twice (once per instruction), she counts
    /// once toward the fee.
    #[test]
    fn unique_signer_dedupe_across_instructions() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let carol = Pubkey::new_unique();
        let metas_a = vec![
            AccountMeta::new(alice, true),
            AccountMeta::new(bob, true),
        ];
        let metas_b = vec![
            AccountMeta::new(alice, true), // signs again — no double-count
            AccountMeta::new(carol, true),
        ];
        let n = count_unique_signers(&[&metas_a, &metas_b]);
        assert_eq!(n, 3); // alice + bob + carol
    }

    /// Total fee combines base + priority.
    #[test]
    fn total_fee_combines_base_and_priority() {
        let fee = total_fee(&FeeCalculator::default(), 2, 200_000, 1_000_000);
        // base = 2 × 5000 = 10_000
        // priority = 200_000 × 1_000_000 / 1_000_000 = 200_000
        // total = 210_000
        assert_eq!(fee, 210_000);
    }
}
