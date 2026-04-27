//! Pre/post account-state validation — the bug-revealing layer.
//!
//! Solana's runtime enforces a handful of structural invariants
//! between the pre- and post-state of every instruction. A
//! program that mutates an account it doesn't own, or breaks
//! lamport conservation, or silently flips an account's
//! `executable` flag, **passes most homemade test harnesses**
//! but is hard-rejected on mainnet. The result: tests pass
//! locally, the program ships, the production transaction
//! reverts on first invocation, and the bug looks like a
//! runtime mystery.
//!
//! Hopper's SVM runs these same checks after each successful
//! instruction so the bug surfaces locally with a structured
//! error pointing at the exact account + rule that broke. Cost:
//! O(n) per instruction over the meta-list, with no allocation
//! beyond a small per-account address-resolution probe.
//!
//! ## Rules enforced (Phase 1)
//!
//! 1. **Read-only accounts cannot change.** If `meta.is_writable`
//!    is false, the account's `lamports`, `data`, `owner`, and
//!    `executable` must equal the pre-state.
//! 2. **Lamport conservation.** The sum of `lamports` across all
//!    metas must equal the pre-sum. Lamports can move between
//!    accounts, but cannot be created or destroyed.
//! 3. **Data writes require ownership.** An account's `data` may
//!    only change if `pre.owner == program_id` (the running
//!    program owns the account), OR the account had empty data
//!    and was just allocated (creation case — only the system
//!    program triggers this path).
//! 4. **Owner reassignment requires ownership.** An account's
//!    `owner` may only change if `pre.owner == program_id`. This
//!    is the rule that backs `system_instruction::assign` —
//!    only the current owner can hand the account to a new
//!    program.
//! 5. **Executable flag is immutable.** Programs cannot toggle
//!    `executable` on or off. Setting it requires the BPF
//!    loader's `Finalize` instruction, which Phase 1 doesn't
//!    simulate.
//! 6. **Lamport debits require ownership.** An account's
//!    `lamports` may only DECREASE if `pre.owner == program_id`.
//!    Anyone can ADD lamports to any account, but to subtract
//!    you must own the account. This rule combines with rule 2
//!    to make the "fake out lamports" attack vector impossible:
//!    a program can't debit a non-owned account because rule 6
//!    rejects it; can't credit lamports out of thin air because
//!    rule 2 rejects the conservation break.
//!
//! ## Policy
//!
//! [`ValidationPolicy::Strict`] is the default — every rule
//! checked, failure aborts the instruction. [`ValidationPolicy::Lax`]
//! disables validation for tests that intentionally exercise
//! the abstract behaviour without runtime-shape conformance
//! (e.g. fast unit tests of pure business logic). Toggle via
//! [`crate::HopperSvm::with_lax_validation`].
//!
//! ## Built-in exceptions
//!
//! - The **system program** is exempt from rule 3 (data writes
//!   require ownership): `CreateAccount`, `Allocate`, etc.
//!   legitimately allocate data on accounts the system program
//!   doesn't end up owning. Allocating goes through the system
//!   program's own validation path; we trust it.
//! - The **system program** is exempt from rule 4 only when
//!   handling `Allocate` against an empty account, which is
//!   a creation case (the new account's owner becomes whatever
//!   the caller requested, not the system program).
//!
//! Both exceptions reduce to "the system program is allowed to
//! initialise the post-creation owner field" — a single test
//! pinned in the rules below.

use crate::account::KeyedAccount;
use crate::error::HopperSvmError;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_program;

/// Validation policy. `Strict` is the default; `Lax` disables
/// every rule — useful for fast unit tests where the program
/// is a hand-written simulator and the structural invariants
/// don't apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ValidationPolicy {
    /// All rules enforced.
    #[default]
    Strict,
    /// All rules skipped. Use sparingly — the strict policy is
    /// what catches mainnet-only bugs early.
    Lax,
}

/// Run post-instruction validation. Returns `Ok(())` if every
/// rule holds, or [`HopperSvmError::AccountValidationFailed`]
/// with the offending account and a human-readable reason.
///
/// `program_id` is the program that just ran. `metas` is the
/// instruction's account metas (so we know which slots were
/// declared writable). `pre` and `post` are the account states
/// before and after execution. The pre/post lists may have
/// different orderings; we resolve by address.
pub fn validate_post_state(
    program_id: &Pubkey,
    metas: &[AccountMeta],
    pre: &[KeyedAccount],
    post: &[KeyedAccount],
    policy: ValidationPolicy,
) -> Result<(), HopperSvmError> {
    if matches!(policy, ValidationPolicy::Lax) {
        return Ok(());
    }

    // Lookup helpers — addresses can repeat (duplicate metas);
    // we always use the FIRST occurrence in pre/post so a
    // duplicate writable account is checked once.
    let pre_of = |addr: &Pubkey| pre.iter().find(|a| &a.address == addr);
    let post_of = |addr: &Pubkey| post.iter().find(|a| &a.address == addr);

    // Track sum of lamports for the conservation check.
    let mut sum_pre: u128 = 0;
    let mut sum_post: u128 = 0;
    let mut seen: Vec<Pubkey> = Vec::with_capacity(metas.len());

    for meta in metas {
        // Skip duplicates so we only count each account once in
        // the conservation sum.
        if seen.contains(&meta.pubkey) {
            continue;
        }
        seen.push(meta.pubkey);

        let p = match pre_of(&meta.pubkey) {
            Some(p) => p,
            None => continue, // Account didn't exist before; only
                              // way it appears in post is via
                              // creation, which is allowed by
                              // rule 3 path.
        };
        let n = match post_of(&meta.pubkey) {
            Some(n) => n,
            None => continue, // Removed from post (close path);
                              // lamports already accounted via
                              // pre-side sum.
        };

        sum_pre = sum_pre.saturating_add(p.lamports as u128);
        sum_post = sum_post.saturating_add(n.lamports as u128);

        // Rule 1: read-only accounts cannot change.
        if !meta.is_writable {
            if p.lamports != n.lamports
                || p.data != n.data
                || p.owner != n.owner
                || p.executable != n.executable
            {
                return Err(HopperSvmError::AccountValidationFailed {
                    account: meta.pubkey,
                    reason: "read-only account modified".to_string(),
                });
            }
            continue;
        }

        // Rule 5: executable flag is immutable.
        if p.executable != n.executable {
            return Err(HopperSvmError::AccountValidationFailed {
                account: meta.pubkey,
                reason: format!(
                    "executable flag toggled ({} -> {})",
                    p.executable, n.executable
                ),
            });
        }

        // Rule 6: lamport debits require ownership. Anyone can
        // credit any account; only the owner can debit. This
        // rule, combined with rule 2 (conservation), makes
        // "lamport theft" impossible without ownership.
        if n.lamports < p.lamports && p.owner != *program_id {
            return Err(HopperSvmError::AccountValidationFailed {
                account: meta.pubkey,
                reason: format!(
                    "lamports debited by non-owner program (pre.owner={}, program_id={}, debit={})",
                    p.owner,
                    program_id,
                    p.lamports - n.lamports
                ),
            });
        }

        // Rule 4: owner reassignment requires pre.owner == program_id.
        // Exception: the system program legitimately sets the
        // owner of a freshly-created (empty) account.
        if p.owner != n.owner {
            let is_creation =
                p.lamports == 0 && p.data.is_empty() && p.owner == system_program::id();
            let owner_change_allowed =
                p.owner == *program_id || (is_creation && *program_id == system_program::id());
            if !owner_change_allowed {
                return Err(HopperSvmError::AccountValidationFailed {
                    account: meta.pubkey,
                    reason: format!(
                        "owner reassigned by non-owner program (pre.owner={}, program_id={})",
                        p.owner, program_id
                    ),
                });
            }
        }

        // Rule 3: data writes require pre.owner == program_id.
        // Exception 1: the system program creating an account
        // (empty pre, system_program owner). Exception 2: any
        // program creating a brand-new account (system program
        // doesn't always own the data after creation; the
        // CreateAccount path sets the owner to the requested
        // program, then writes zeroed data of the requested
        // size).
        if p.data != n.data {
            let was_empty_system_owned =
                p.lamports == 0 && p.data.is_empty() && p.owner == system_program::id();
            let data_change_allowed = p.owner == *program_id || was_empty_system_owned;
            if !data_change_allowed {
                return Err(HopperSvmError::AccountValidationFailed {
                    account: meta.pubkey,
                    reason: format!(
                        "data mutated by non-owner program (pre.owner={}, program_id={})",
                        p.owner, program_id
                    ),
                });
            }
        }
    }

    // Rule 2: lamport conservation across all involved accounts.
    // Exception: accounts being created are exempt from the
    // conservation tally because their lamports come from outside
    // the instruction's account slice (the system program funds
    // them from a payer that may not appear in the pre/post pair
    // in unit tests). Detecting "this is a creation" follows the
    // same `was_empty_system_owned` predicate the data-write rule
    // uses.
    let creation_credit: u128 = pre
        .iter()
        .zip(post.iter())
        .filter_map(|(p, n)| {
            // A real "creation" event flips at least one of the
            // structural fields (owner, data) on top of the lamport
            // change. A pure incoming-lamport transfer to a brand-new
            // address does NOT count: the conservation rule must still
            // catch the corresponding debit elsewhere in the slice.
            let was_empty_system_owned =
                p.lamports == 0 && p.data.is_empty() && p.owner == system_program::id();
            let owner_changed = p.owner != n.owner;
            let data_grew = p.data.is_empty() && !n.data.is_empty();
            if was_empty_system_owned && (owner_changed || data_grew) {
                Some(n.lamports as u128)
            } else {
                None
            }
        })
        .sum();
    let adjusted_pre = sum_pre.saturating_add(creation_credit);
    if adjusted_pre != sum_post {
        return Err(HopperSvmError::AccountValidationFailed {
            account: Pubkey::default(),
            reason: format!(
                "lamport conservation broken (pre={sum_pre}+{creation_credit}, post={sum_post}, delta={})",
                sum_post as i128 - adjusted_pre as i128
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pk: Pubkey, signer: bool, writable: bool) -> AccountMeta {
        AccountMeta {
            pubkey: pk,
            is_signer: signer,
            is_writable: writable,
        }
    }

    fn ka(addr: Pubkey, lamports: u64, owner: Pubkey, data: Vec<u8>) -> KeyedAccount {
        KeyedAccount::new(addr, lamports, owner, data, false)
    }

    /// Identity case: pre == post. Always passes.
    #[test]
    fn identity_passes() {
        let prog = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, prog, vec![1, 2, 3])];
        let post = pre.clone();
        let metas = vec![meta(alice, false, true)];
        validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict)
            .expect("identity ok");
    }

    /// Rule 1: a read-only account whose lamports changed is
    /// rejected.
    #[test]
    fn readonly_lamport_change_rejected() {
        let prog = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, prog, vec![])];
        let post = vec![ka(alice, 200, prog, vec![])]; // different
        let metas = vec![meta(alice, false, false)]; // read-only
        let err =
            validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict).unwrap_err();
        match err {
            HopperSvmError::AccountValidationFailed { reason, account } => {
                assert_eq!(account, alice);
                assert!(reason.contains("read-only"), "{reason}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Rule 2: lamport conservation. Sum changes break.
    #[test]
    fn lamport_conservation_rejected() {
        let prog = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, prog, vec![]), ka(bob, 50, prog, vec![])];
        // Lamports created out of thin air.
        let post = vec![ka(alice, 100, prog, vec![]), ka(bob, 100, prog, vec![])];
        let metas = vec![meta(alice, false, true), meta(bob, false, true)];
        let err =
            validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict).unwrap_err();
        match err {
            HopperSvmError::AccountValidationFailed { reason, .. } => {
                assert!(reason.contains("conservation"), "{reason}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Rule 2: a transfer between two accounts of the same
    /// program preserves the sum and passes.
    #[test]
    fn lamport_transfer_passes() {
        let prog = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, prog, vec![]), ka(bob, 50, prog, vec![])];
        let post = vec![ka(alice, 60, prog, vec![]), ka(bob, 90, prog, vec![])];
        let metas = vec![meta(alice, false, true), meta(bob, false, true)];
        validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict)
            .expect("transfer ok");
    }

    /// Rule 3: data write by a program that doesn't own the
    /// account is rejected.
    #[test]
    fn non_owner_data_write_rejected() {
        let prog = Pubkey::new_unique();
        let other_owner = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, other_owner, vec![1, 2, 3])];
        let post = vec![ka(alice, 100, other_owner, vec![1, 2, 9])]; // last byte changed
        let metas = vec![meta(alice, false, true)];
        let err =
            validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict).unwrap_err();
        match err {
            HopperSvmError::AccountValidationFailed { reason, .. } => {
                assert!(reason.contains("data mutated by non-owner"), "{reason}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Rule 3 exception: data write on a freshly-created (empty,
    /// system-program-owned) account is allowed regardless of
    /// who's running.
    #[test]
    fn data_write_on_creation_allowed() {
        let prog = system_program::id();
        let alice = Pubkey::new_unique();
        let pre = vec![ka(alice, 0, system_program::id(), vec![])]; // empty
        let post = vec![ka(alice, 1_000, prog, vec![0u8; 100])]; // allocated
        let metas = vec![meta(alice, true, true)];
        validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict)
            .expect("creation ok");
    }

    /// Rule 4: owner reassignment by a non-owner program is
    /// rejected.
    #[test]
    fn non_owner_assign_rejected() {
        let prog = Pubkey::new_unique();
        let other_owner = Pubkey::new_unique();
        let new_owner = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, other_owner, vec![1])];
        let post = vec![ka(alice, 100, new_owner, vec![1])];
        let metas = vec![meta(alice, false, true)];
        let err =
            validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict).unwrap_err();
        match err {
            HopperSvmError::AccountValidationFailed { reason, .. } => {
                assert!(reason.contains("owner reassigned"), "{reason}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Rule 4 OK case: the program that owns the account
    /// reassigns it to a new program.
    #[test]
    fn owner_assign_by_owner_passes() {
        let prog = Pubkey::new_unique();
        let new_owner = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, prog, vec![])];
        let post = vec![ka(alice, 100, new_owner, vec![])];
        let metas = vec![meta(alice, true, true)];
        validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict)
            .expect("self-assign ok");
    }

    /// Rule 5: executable flag immutable.
    #[test]
    fn executable_toggle_rejected() {
        let prog = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let pre_a = ka(alice, 100, prog, vec![]);
        let mut post_a = pre_a.clone();
        post_a.executable = !pre_a.executable; // toggle
        let metas = vec![meta(alice, false, true)];
        let err = validate_post_state(&prog, &metas, &[pre_a], &[post_a], ValidationPolicy::Strict)
            .unwrap_err();
        match err {
            HopperSvmError::AccountValidationFailed { reason, .. } => {
                assert!(reason.contains("executable"), "{reason}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Rule 6: a non-owner program that debits an account's
    /// lamports is rejected. The account is writable but not
    /// owned by the running program — debit is forbidden.
    #[test]
    fn non_owner_lamport_debit_rejected() {
        let prog = Pubkey::new_unique();
        let other_owner = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        // Pre: alice has 100 lamports owned by other_owner, recipient has 0.
        let pre = vec![
            ka(alice, 100, other_owner, vec![]),
            ka(recipient, 0, prog, vec![]),
        ];
        // Post: prog tried to siphon lamports from alice to recipient.
        // Conservation holds (100 + 0 = 60 + 40), but rule 6
        // catches the unauthorized debit.
        let post = vec![
            ka(alice, 60, other_owner, vec![]),
            ka(recipient, 40, prog, vec![]),
        ];
        let metas = vec![meta(alice, false, true), meta(recipient, false, true)];
        let err =
            validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict).unwrap_err();
        match err {
            HopperSvmError::AccountValidationFailed { account, reason } => {
                assert_eq!(account, alice);
                assert!(reason.contains("debited by non-owner"), "{reason}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Rule 6 OK case: a program crediting an account it
    /// doesn't own is allowed (anyone can credit). Pin against
    /// over-strict checks that would reject legitimate flows.
    #[test]
    fn non_owner_lamport_credit_allowed() {
        let prog = Pubkey::new_unique();
        let other_owner = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let donor = Pubkey::new_unique();
        let pre = vec![
            ka(alice, 50, other_owner, vec![]),
            ka(donor, 100, prog, vec![]),
        ];
        // donor (which we own) sends 30 lamports to alice (which
        // we don't own) — credit-only on alice, conservation holds.
        let post = vec![
            ka(alice, 80, other_owner, vec![]),
            ka(donor, 70, prog, vec![]),
        ];
        let metas = vec![meta(alice, false, true), meta(donor, true, true)];
        validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Strict)
            .expect("non-owner credit ok");
    }

    /// Lax policy lets anything through.
    #[test]
    fn lax_policy_disables_all_checks() {
        let prog = Pubkey::new_unique();
        let other_owner = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let pre = vec![ka(alice, 100, other_owner, vec![1, 2])];
        // Multiple rule violations in one shot:
        //   - non-owner data write
        //   - lamport non-conservation
        //   - non-owner assign
        let post = vec![ka(alice, 999, Pubkey::new_unique(), vec![9, 9])];
        let metas = vec![meta(alice, false, true)];
        validate_post_state(&prog, &metas, &pre, &post, ValidationPolicy::Lax)
            .expect("lax skips everything");
    }
}
