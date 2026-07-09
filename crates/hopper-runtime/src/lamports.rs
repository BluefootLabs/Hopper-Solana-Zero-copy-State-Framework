//! Gate-aware lamport movement (BLD-MUT).
//!
//! [`transfer_lamports`] is **the** lamport transfer for programs whose
//! contexts declare `strict_writes` + `lamports(...)`: it performs the
//! exact arithmetic of the substrate helper
//! (`hopper_native::batch::transfer_lamports` — insufficient-funds
//! checked before overflow, all-or-nothing), but every balance write
//! crosses the runtime's lamport funnel
//! ([`native_boundary::try_set_lamports`](crate::native_boundary::try_set_lamports)),
//! so an installed lamport gate ([`write_policy`](crate::write_policy),
//! BLD-MUT) sees the move. The substrate helper writes balances
//! directly at the native layer and bypasses the gate by design (it is
//! the cheap no-CPI path); this module closes that gap for gated
//! programs without force-routing anyone else.

use crate::account::AccountView;
use crate::address::address_eq;
use crate::error::ProgramError;
use crate::ProgramResult;

/// Transfer `amount` lamports between two accounts without CPI, through
/// the runtime's **gated** lamport funnel.
///
/// This is the lamport transfer for `strict_writes` + `lamports(...)`
/// programs (the mutation-complete contract, BLD-MUT): both sides are
/// checked against the installed lamport gate **before any balance
/// changes**, so a refusal — `Custom(0xD000 | account_index)` on the
/// first undeclared side — can never half-apply the move. On a
/// `lamports(...)` bound context the generated
/// `ctx.transfer_lamports(from, to, amount)` method delegates here.
///
/// # Semantics
///
/// Identical arithmetic to the substrate helper
/// `hopper_native::batch::transfer_lamports`: insufficient funds is
/// checked before credit overflow ([`ProgramError::InsufficientFunds`]
/// wins when both would fail), both post-balances are computed before
/// either is applied (an arithmetic refusal also cannot half-apply),
/// and — like the substrate helper — no writability pre-check is added
/// (Sealevel's `writable` flag is still enforced underneath).
///
/// The one behavioral divergence is deliberate: a **self-transfer**
/// (`from` and `to` carry the same address, i.e. the same underlying
/// account) is handled explicitly as a balance-checked net zero,
/// mirroring the host System-transfer emulation and the real System
/// program. The substrate helper's debit-then-credit sequence would
/// credit the pre-debit balance and mint `amount` out of thin air.
///
/// # Cost
///
/// When no gate is installed, the only work added over the substrate
/// helper is the self-transfer address compare and the gate's existing
/// no-gate fast path; the per-account address comparisons against the
/// declared set happen only while a gate is actually installed.
///
/// # Errors
///
/// - `Custom(0xD000 | index)` — an installed lamport gate refuses
///   `from` or `to` (checked in that order), before any mutation.
/// - [`ProgramError::InsufficientFunds`] — `from` holds fewer than
///   `amount` lamports.
/// - [`ProgramError::ArithmeticOverflow`] — crediting `to` would
///   overflow `u64`.
#[inline]
pub fn transfer_lamports(
    from: &AccountView<'_>,
    to: &AccountView<'_>,
    amount: u64,
) -> ProgramResult {
    // BLD-MUT: pre-validate BOTH sides against the lamport gate before
    // any balance mutation. Relying on the per-account `try_set_lamports`
    // funnel alone would debit `from` and then have `to` refused at the
    // funnel, destroying lamports on the error path — a transfer must be
    // all-or-nothing. (Same pattern as the host System-transfer
    // emulation in `cpi.rs`.)
    crate::write_policy::check_lamport_mutation(from.address())?;
    crate::write_policy::check_lamport_mutation(to.address())?;

    // Self-transfer (same address = same underlying account): net zero.
    // Handled explicitly because the compute-both-then-apply sequence
    // below would otherwise credit from the pre-debit balance and mint
    // `amount` out of thin air.
    if address_eq(from.address(), to.address()) {
        if from.lamports() < amount {
            return Err(ProgramError::InsufficientFunds);
        }
        return Ok(());
    }

    // Compute both post-balances before applying either, so an
    // arithmetic refusal (insufficient funds, overflow) also cannot
    // half-apply the transfer. Check order matches the substrate
    // helper: insufficient funds before credit overflow.
    let debited = from
        .lamports()
        .checked_sub(amount)
        .ok_or(ProgramError::InsufficientFunds)?;
    let credited = to
        .lamports()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    from.try_set_lamports(debited)?;
    to.try_set_lamports(credited)?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_policy::{
        install_lamport_gate, write_policy_violation, WritePolicy, WriteRange,
    };
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make_backend(seed: u8, lamports: u64) -> (std::vec::Vec<u8>, NativeAccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + 32];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: the test owns `backing`, writes one valid RuntimeAccount
        // header, and keeps the buffer alive for the returned view.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([seed; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports,
                data_len: 32,
            });
        }
        // SAFETY: `raw` points at the RuntimeAccount just initialized above.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, backend)
    }

    fn make_account(seed: u8, lamports: u64) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let (backing, backend) = make_backend(seed, lamports);
        (backing, AccountView::from_backend(backend))
    }

    // ── (b) Ungated: byte-identical to the substrate helper ─────────

    /// Differential parity: with no gate installed, the runtime helper
    /// and `hopper_native::batch::transfer_lamports` must produce the
    /// same result and the same post-balances for every distinct-account
    /// case, including the error ordering (insufficient funds wins over
    /// overflow when both apply).
    #[test]
    fn ungated_behavior_matches_substrate_helper_exactly() {
        // (from_balance, to_balance, amount)
        let cases: [(u64, u64, u64); 6] = [
            (100, 50, 30),        // plain success
            (100, 50, 100),       // drain to exactly zero
            (100, 50, 0),         // zero amount is a no-op success
            (100, 50, 150),       // insufficient funds
            (100, u64::MAX, 1),   // credit overflow
            (100, u64::MAX, 150), // both would fail: sub is checked first
        ];

        for (i, &(from_bal, to_bal, amount)) in cases.iter().enumerate() {
            let seed = (10 + 4 * i) as u8;
            let (_rf, runtime_from) = make_account(seed, from_bal);
            let (_rt, runtime_to) = make_account(seed + 1, to_bal);
            let (_nf, native_from) = make_backend(seed + 2, from_bal);
            let (_nt, native_to) = make_backend(seed + 3, to_bal);

            let ours = transfer_lamports(&runtime_from, &runtime_to, amount);
            let theirs = hopper_native::batch::transfer_lamports(&native_from, &native_to, amount)
                .map_err(ProgramError::from);

            assert_eq!(ours, theirs, "case {i}: result diverged");
            assert_eq!(
                runtime_from.lamports(),
                native_from.lamports(),
                "case {i}: from balance diverged"
            );
            assert_eq!(
                runtime_to.lamports(),
                native_to.lamports(),
                "case {i}: to balance diverged"
            );
            // A refusal must leave both sides untouched.
            if ours.is_err() {
                assert_eq!(runtime_from.lamports(), from_bal, "case {i}");
                assert_eq!(runtime_to.lamports(), to_bal, "case {i}");
            }
        }
    }

    // ── (a) Gated ────────────────────────────────────────────────────

    #[test]
    fn gated_transfer_between_declared_accounts_moves_exact_balances() {
        let (_b0, from) = make_account(40, 1_000);
        let (_b1, to) = make_account(41, 250);
        let accounts = [from, to];
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[0, 1]);
        let _gate = install_lamport_gate(&accounts, &P);

        transfer_lamports(&accounts[0], &accounts[1], 400).unwrap();
        assert_eq!(accounts[0].lamports(), 600);
        assert_eq!(accounts[1].lamports(), 650);
    }

    #[test]
    fn gated_transfer_from_undeclared_account_is_refused_before_any_mutation() {
        // `from` (index 0) is NOT in the declared set; `to` (index 1) is.
        let (_b0, from) = make_account(42, 1_000);
        let (_b1, to) = make_account(43, 250);
        let accounts = [from, to];
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[1]);
        let _gate = install_lamport_gate(&accounts, &P);

        assert_eq!(
            transfer_lamports(&accounts[0], &accounts[1], 400),
            Err(write_policy_violation(0))
        );
        assert_eq!(accounts[0].lamports(), 1_000);
        assert_eq!(accounts[1].lamports(), 250);
    }

    #[test]
    fn gated_transfer_to_undeclared_account_is_refused_before_any_mutation() {
        // `from` (index 0) is declared; `to` (index 1) is NOT. Without
        // the both-sides pre-check the debit would land at the funnel
        // and the credit be refused, destroying 400 lamports.
        let (_b0, from) = make_account(44, 1_000);
        let (_b1, to) = make_account(45, 250);
        let accounts = [from, to];
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[0]);
        let _gate = install_lamport_gate(&accounts, &P);

        assert_eq!(
            transfer_lamports(&accounts[0], &accounts[1], 400),
            Err(write_policy_violation(1))
        );
        assert_eq!(accounts[0].lamports(), 1_000);
        assert_eq!(accounts[1].lamports(), 250);
    }

    #[test]
    fn gated_arithmetic_refusals_keep_indexed_gate_errors_out_of_the_way() {
        // Both sides declared: the gate admits the move, and the
        // arithmetic errors surface exactly as in the ungated path.
        let (_b0, from) = make_account(46, 100);
        let (_b1, to) = make_account(47, u64::MAX);
        let accounts = [from, to];
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[0, 1]);
        let _gate = install_lamport_gate(&accounts, &P);

        assert_eq!(
            transfer_lamports(&accounts[0], &accounts[1], 150),
            Err(ProgramError::InsufficientFunds)
        );
        assert_eq!(
            transfer_lamports(&accounts[0], &accounts[1], 1),
            Err(ProgramError::ArithmeticOverflow)
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1].lamports(), u64::MAX);
    }

    // ── (c) Self-transfer ────────────────────────────────────────────

    #[test]
    fn ungated_self_transfer_is_balance_checked_net_zero() {
        // Two views over the SAME underlying account (duplicate metas).
        let (_b, a) = make_account(50, 500);
        let alias = a.clone();

        // Balance-covered: net zero, no minting (the substrate helper
        // would set the balance to 500 + 200 here).
        transfer_lamports(&a, &alias, 200).unwrap();
        assert_eq!(a.lamports(), 500);

        // Over-balance: refused, balance untouched.
        assert_eq!(
            transfer_lamports(&a, &alias, 501),
            Err(ProgramError::InsufficientFunds)
        );
        assert_eq!(a.lamports(), 500);
    }

    #[test]
    fn gated_self_transfer_follows_the_declared_set() {
        // Declared: net zero succeeds under the gate.
        let (_b0, declared) = make_account(51, 500);
        let (_b1, foreign) = make_account(52, 500);
        let accounts = [declared];
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[0]);
        let _gate = install_lamport_gate(&accounts, &P);

        let alias = accounts[0].clone();
        transfer_lamports(&accounts[0], &alias, 200).unwrap();
        assert_eq!(accounts[0].lamports(), 500);
        assert_eq!(
            transfer_lamports(&accounts[0], &alias, 501),
            Err(ProgramError::InsufficientFunds)
        );

        // Undeclared (foreign to the gated slice): refused fail-closed
        // even though the move would net zero — the gate is consulted
        // before the self-transfer branch, mirroring the host
        // System-transfer emulation.
        let foreign_alias = foreign.clone();
        assert_eq!(
            transfer_lamports(&foreign, &foreign_alias, 1),
            Err(write_policy_violation(u8::MAX))
        );
        assert_eq!(foreign.lamports(), 500);
    }

    #[test]
    fn dropping_the_gate_restores_ungated_passthrough() {
        let (_b0, from) = make_account(53, 1_000);
        let (_b1, to) = make_account(54, 0);
        let accounts = [from, to];
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[]);
        {
            let _gate = install_lamport_gate(&accounts, &P);
            // Empty declared set: everything is refused while installed.
            assert_eq!(
                transfer_lamports(&accounts[0], &accounts[1], 1),
                Err(write_policy_violation(0))
            );
        }
        // Gate dropped: the same call goes through ungated.
        transfer_lamports(&accounts[0], &accounts[1], 1).unwrap();
        assert_eq!(accounts[0].lamports(), 999);
        assert_eq!(accounts[1].lamports(), 1);
    }

    /// A realistic mutation-complete policy carries data ranges AND the
    /// lamport set; the transfer consults only the lamport dimension.
    #[test]
    fn gated_transfer_composes_with_data_ranges() {
        let (_b0, from) = make_account(55, 10);
        let (_b1, to) = make_account(56, 10);
        let accounts = [from, to];
        static P: WritePolicy =
            WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0, 1]);
        let _gate = install_lamport_gate(&accounts, &P);
        transfer_lamports(&accounts[0], &accounts[1], 10).unwrap();
        assert_eq!(accounts[0].lamports(), 0);
        assert_eq!(accounts[1].lamports(), 20);
    }
}
