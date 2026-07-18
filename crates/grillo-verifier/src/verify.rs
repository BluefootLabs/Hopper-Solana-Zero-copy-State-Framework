//! The verdict algorithm: check a transaction's ACTUAL byte changes against
//! a Hopper instruction's published mutation contract.
//!
//! The invariant, honest by construction:
//!
//! > **changed ⊆ acquired ⊆ authorized**
//!
//! - `changed`    — bytes that actually differ between the pre and post
//!   account snapshots.
//! - `acquired`   — bytes covered by a WRITE touch record (what the
//!   instruction told the runtime it was mutating).
//! - `authorized` — bytes the manifest's `writeRanges` permit.
//!
//! Acquired-but-unchanged is LEGAL: access is not modification. A WRITE
//! record that changed none of its bytes is surfaced as a note on a PASS,
//! never a violation.

use grillo_manifest::{
    InstructionContract, MutationContractView, RangeContract, ResolveError,
    ResolvedInstructionContract,
};

use crate::touch_map::TouchMap;

/// A pre/post snapshot of one account in the instruction — its bytes and,
/// optionally, its lamport balance.
///
/// Lamports are explicitly marked observed by [`with_lamports`](Self::with_lamports),
/// so an omitted balance snapshot can never be confused with an observed
/// `0 -> 0` balance.
#[derive(Clone, Debug)]
pub struct AccountDelta<'a> {
    /// The account's index in the instruction's account list (matches a
    /// touch record `slot` and a [`RangeContract::account_index`]).
    pub account_index: u8,
    /// Account data before the instruction.
    pub pre: &'a [u8],
    /// Account data after the instruction.
    pub post: &'a [u8],
    /// Lamport balance before the instruction.
    pub pre_lamports: u64,
    /// Lamport balance after the instruction.
    pub post_lamports: u64,
    /// Whether both lamport balances came from real snapshots.
    pub lamports_observed: bool,
}

impl<'a> AccountDelta<'a> {
    /// A data-only delta (lamports untracked).
    pub fn new(account_index: u8, pre: &'a [u8], post: &'a [u8]) -> Self {
        Self {
            account_index,
            pre,
            post,
            pre_lamports: 0,
            post_lamports: 0,
            lamports_observed: false,
        }
    }

    /// Attach lamport balances so the lamport dimension is checked.
    pub fn with_lamports(mut self, pre_lamports: u64, post_lamports: u64) -> Self {
        self.pre_lamports = pre_lamports;
        self.post_lamports = post_lamports;
        self.lamports_observed = true;
        self
    }
}

/// A single contract violation, with byte-precise evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    /// A byte changed that no WRITE touch record covers: `changed ⊄
    /// acquired`. The instruction mutated a byte it never told the runtime
    /// it was touching.
    UntrackedWrite { account_index: u8, offset: u32 },
    /// A WRITE touch record acquired bytes outside the authorized set:
    /// `acquired ⊄ authorized`. The instruction acquired a write lease on
    /// bytes the manifest does not permit it to write.
    UnauthorizedAcquisition {
        account_index: u8,
        offset: u32,
        size: u32,
    },
    /// A lamport balance moved on an account the (mutation-complete)
    /// contract does not list as a lamport target.
    UnauthorizedLamportDelta {
        account_index: u8,
        pre_lamports: u64,
        post_lamports: u64,
    },
}

/// Why a verdict could not be a definitive PASS or VIOLATION.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InconclusiveReason {
    /// The instruction did not compile the byte-range write contract
    /// (`strict_writes = false`). Its `authorized` set carries no authority,
    /// so there is no byte contract to check `changed`/`acquired` against.
    NoByteContract,
    /// The manifest contains invocation-parametric exact-cell rules, but the
    /// caller used the unresolved verification API.  Verifying against the
    /// conservative static column envelope could produce a false PASS, so
    /// Grillo requires the raw argument payload (or an already-resolved
    /// contract) instead.
    ParametricArgumentsRequired,
    /// The touch map is partial (`overflowed` and/or `skipped`). It does not
    /// enumerate the instruction's complete effect set, so byte attribution
    /// is impossible — Grillo returns this rather than risk a false PASS.
    /// Rare by construction: the runtime coalesces exact unions under
    /// capacity pressure, so `overflowed` requires more than
    /// [`MAX_TOUCH_RECORDS`](crate::MAX_TOUCH_RECORDS) pairwise-unmergeable
    /// ranges in one instruction, and `skipped` requires an offset beyond
    /// 2³¹ (unreachable for real accounts, which cap at 10 MiB).
    PartialTouchMap { overflowed: bool, skipped: bool },
}

/// Evidence backing a PASS.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PassEvidence {
    /// Account indices for which pre/post data snapshots were inspected.
    /// An empty list is an empty-scope proof, not a transaction-complete one.
    pub observed_data_accounts: Vec<u8>,
    /// Account indices for which pre/post lamport snapshots were inspected.
    pub observed_lamport_accounts: Vec<u8>,
    /// Coalesced changed byte ranges across all accounts, in account/offset
    /// order.
    pub changed: Vec<RangeContract>,
    /// WRITE touch records that acquired bytes but changed NONE of them —
    /// legal (access is not modification), surfaced as notes.
    pub acquired_unchanged: Vec<RangeContract>,
    /// Total number of changed bytes across all accounts.
    pub changed_bytes: u64,
}

/// The verdict of a verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// `changed ⊆ acquired ⊆ authorized` held for the supplied snapshot
    /// scope (and observed lamports stayed within the declared set). Carries
    /// the byte-precise evidence and the exact observed account indices.
    Pass(PassEvidence),
    /// One or more contract violations, each with byte-precise evidence.
    Violation(Vec<Violation>),
    /// The inputs did not permit a definitive verdict; see the reason.
    Inconclusive(InconclusiveReason),
}

impl Verdict {
    /// Whether this verdict is a scoped PASS.
    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Pass(_))
    }

    /// A human-readable, multi-line rendering of the verdict with its
    /// byte-precise evidence.
    pub fn render(&self) -> String {
        match self {
            Verdict::Pass(ev) => {
                let mut s = String::from(
                    "GRILLO VERDICT: SCOPED PASS  (changed \u{2286} acquired \u{2286} authorized)\n",
                );
                s.push_str(&format!(
                    "  observed data accounts: {:?}\n",
                    ev.observed_data_accounts
                ));
                s.push_str(&format!(
                    "  observed lamport accounts: {:?}\n",
                    ev.observed_lamport_accounts
                ));
                s.push_str(&format!(
                    "  changed: {} byte(s) in {} range(s)\n",
                    ev.changed_bytes,
                    ev.changed.len()
                ));
                for r in &ev.changed {
                    s.push_str(&format!(
                        "    - account {} [{}..{})\n",
                        r.account_index,
                        r.offset,
                        r.end()
                    ));
                }
                if ev.acquired_unchanged.is_empty() {
                    s.push_str("  acquired-but-unchanged: none\n");
                } else {
                    s.push_str("  acquired-but-unchanged (access is not modification):\n");
                    for r in &ev.acquired_unchanged {
                        s.push_str(&format!(
                            "    - account {} [{}..{})\n",
                            r.account_index,
                            r.offset,
                            r.end()
                        ));
                    }
                }
                s
            }
            Verdict::Violation(violations) => {
                let mut s = format!(
                    "GRILLO VERDICT: VIOLATION ({} finding(s))\n",
                    violations.len()
                );
                for v in violations {
                    s.push_str("  - ");
                    match v {
                        Violation::UntrackedWrite {
                            account_index,
                            offset,
                        } => s.push_str(&format!(
                            "UntrackedWrite: account {account_index} byte {offset} changed but no \
                             WRITE touch record covers it (changed \u{2284} acquired)\n"
                        )),
                        Violation::UnauthorizedAcquisition {
                            account_index,
                            offset,
                            size,
                        } => s.push_str(&format!(
                            "UnauthorizedAcquisition: account {account_index} acquired a WRITE lease \
                             on [{offset}..{}) which the manifest does not authorize \
                             (acquired \u{2284} authorized)\n",
                            *offset as u64 + *size as u64
                        )),
                        Violation::UnauthorizedLamportDelta {
                            account_index,
                            pre_lamports,
                            post_lamports,
                        } => s.push_str(&format!(
                            "UnauthorizedLamportDelta: account {account_index} lamports \
                             {pre_lamports} -> {post_lamports}, not a declared lamport target\n"
                        )),
                    }
                }
                s
            }
            Verdict::Inconclusive(reason) => match reason {
                InconclusiveReason::NoByteContract => String::from(
                    "GRILLO VERDICT: INCONCLUSIVE - the instruction declares no byte-range write \
                     contract (strict_writes = false); nothing to verify against\n",
                ),
                InconclusiveReason::ParametricArgumentsRequired => String::from(
                    "GRILLO VERDICT: INCONCLUSIVE - this instruction has parametric exact-cell \
                     rules; resolve the concrete invocation arguments before verification\n",
                ),
                InconclusiveReason::PartialTouchMap {
                    overflowed,
                    skipped,
                } => format!(
                    "GRILLO VERDICT: INCONCLUSIVE_PARTIAL_MAP - the touch map is partial \
                     (overflowed={overflowed}, skipped={skipped}); byte attribution is \
                     impossible, so no PASS may be issued\n"
                ),
            },
        }
    }
}

/// Verify one instruction's effects against its mutation contract.
///
/// `deltas` must cover every account whose bytes (or lamports) the caller
/// wants attributed; an account with no delta is simply not inspected. The
/// `touch_map` is the instruction's decoded self-report; `contract` is the
/// instruction's entry from the [`MutationManifest`](grillo_manifest::MutationManifest).
pub fn verify(
    contract: &InstructionContract,
    deltas: &[AccountDelta<'_>],
    touch_map: &TouchMap,
) -> Verdict {
    if contract.requires_resolution() {
        return Verdict::Inconclusive(InconclusiveReason::ParametricArgumentsRequired);
    }
    verify_contract(contract, deltas, touch_map)
}

/// Resolve a parametric instruction from its real post-discriminator argument
/// payload, then verify its concrete effects.
///
/// Resolution errors are returned separately and can never be mistaken for a
/// PASS.  See [`InstructionContract::resolve_effects`] for the fail-closed wire
/// decoding rules.
pub fn verify_invocation(
    contract: &InstructionContract,
    argument_payload: &[u8],
    deltas: &[AccountDelta<'_>],
    touch_map: &TouchMap,
) -> Result<Verdict, ResolveError> {
    let resolved = contract.resolve_effects(argument_payload)?;
    Ok(verify_resolved(&resolved, deltas, touch_map))
}

/// Verify against an already invocation-resolved effect contract.
pub fn verify_resolved(
    contract: &ResolvedInstructionContract,
    deltas: &[AccountDelta<'_>],
    touch_map: &TouchMap,
) -> Verdict {
    verify_contract(contract, deltas, touch_map)
}

fn verify_contract<C: MutationContractView + ?Sized>(
    contract: &C,
    deltas: &[AccountDelta<'_>],
    touch_map: &TouchMap,
) -> Verdict {
    // A non-strict-writes instruction publishes no enforced byte contract,
    // so there is nothing to hold `changed`/`acquired` to.
    if !contract.strict_writes() {
        return Verdict::Inconclusive(InconclusiveReason::NoByteContract);
    }

    // Honesty rule: a partial touch map cannot enumerate the effect set, so
    // byte attribution is impossible — never a false PASS.
    if touch_map.is_partial() {
        return Verdict::Inconclusive(InconclusiveReason::PartialTouchMap {
            overflowed: touch_map.overflowed,
            skipped: touch_map.skipped,
        });
    }

    // Precompute the coalesced changed intervals for each supplied account.
    let changed_per_account: Vec<(u8, Vec<(u32, u32)>)> = deltas
        .iter()
        .map(|d| (d.account_index, changed_intervals(d.pre, d.post)))
        .collect();

    let mut violations = Vec::new();
    let mut changed_all: Vec<RangeContract> = Vec::new();
    let mut changed_bytes: u64 = 0;

    // ── changed ⊆ acquired ──────────────────────────────────────────────
    //
    // Every changed byte must fall inside some WRITE touch record for that
    // account. A gap is an UntrackedWrite at the exact first uncovered byte.
    for (account_index, intervals) in &changed_per_account {
        let acquired: Vec<(u64, u64)> = touch_map
            .records
            .iter()
            .filter(|r| r.write && r.slot == *account_index)
            .map(|r| (r.offset as u64, r.size as u64))
            .collect();

        for &(offset, size) in intervals {
            changed_bytes += size as u64;
            changed_all.push(RangeContract {
                account_index: *account_index,
                offset,
                size,
            });
            if let Some(gap) = first_gap(offset as u64, offset as u64 + size as u64, &acquired) {
                violations.push(Violation::UntrackedWrite {
                    account_index: *account_index,
                    offset: gap as u32,
                });
            }
        }
    }

    // ── acquired ⊆ authorized ───────────────────────────────────────────
    //
    // Every WRITE touch record's bytes must lie within the union of the
    // manifest's authorized ranges for that account. (Reads are access, not
    // modification, and need no authorization.)
    for rec in touch_map.records.iter().filter(|r| r.write) {
        let authorized: Vec<(u64, u64)> = contract
            .authorized_ranges()
            .iter()
            .filter(|range| range.account_index == rec.slot)
            .map(|r| (r.offset as u64, r.size as u64))
            .collect();
        if first_gap(rec.offset as u64, rec.end(), &authorized).is_some() {
            violations.push(Violation::UnauthorizedAcquisition {
                account_index: rec.slot,
                offset: rec.offset,
                size: rec.size,
            });
        }
    }

    // ── Lamport dimension ───────────────────────────────────────────────
    //
    // Only meaningful when the contract is mutation-complete (otherwise the
    // lamport dimension is undeclared and stays a passthrough).
    if contract.mutation_complete() {
        for delta in deltas {
            if delta.lamports_observed
                && delta.pre_lamports != delta.post_lamports
                && !contract.authorizes_lamports(delta.account_index)
            {
                violations.push(Violation::UnauthorizedLamportDelta {
                    account_index: delta.account_index,
                    pre_lamports: delta.pre_lamports,
                    post_lamports: delta.post_lamports,
                });
            }
        }
    }

    if !violations.is_empty() {
        return Verdict::Violation(violations);
    }

    // ── PASS: gather acquired-but-unchanged notes ───────────────────────
    //
    // A WRITE record whose bytes did not change is legal — surface it. Only
    // records whose account we actually snapshotted can be attributed.
    let mut acquired_unchanged = Vec::new();
    for rec in touch_map.records.iter().filter(|r| r.write) {
        let mut observed = false;
        let mut any_changed = false;
        for (account_index, intervals) in &changed_per_account {
            if *account_index != rec.slot {
                continue;
            }
            observed = true;
            for &(offset, size) in intervals {
                let (s, e) = (offset as u64, offset as u64 + size as u64);
                if s < rec.end() && (rec.offset as u64) < e {
                    any_changed = true;
                }
            }
        }
        if observed && !any_changed {
            acquired_unchanged.push(RangeContract {
                account_index: rec.slot,
                offset: rec.offset,
                size: rec.size,
            });
        }
    }

    let mut observed_data_accounts: Vec<u8> =
        deltas.iter().map(|delta| delta.account_index).collect();
    observed_data_accounts.sort_unstable();
    observed_data_accounts.dedup();
    let mut observed_lamport_accounts: Vec<u8> = deltas
        .iter()
        .filter(|delta| delta.lamports_observed)
        .map(|delta| delta.account_index)
        .collect();
    observed_lamport_accounts.sort_unstable();
    observed_lamport_accounts.dedup();

    Verdict::Pass(PassEvidence {
        observed_data_accounts,
        observed_lamport_accounts,
        changed: changed_all,
        acquired_unchanged,
        changed_bytes,
    })
}

/// Coalesced changed-byte intervals `(offset, size)` between two snapshots.
///
/// A byte differs when the two sides disagree at that index; if one side is
/// shorter, the extra bytes on the longer side count as changed (a
/// grow/shrink is a change). Consecutive changed bytes coalesce into one
/// interval.
fn changed_intervals(pre: &[u8], post: &[u8]) -> Vec<(u32, u32)> {
    let n = pre.len().max(post.len());
    let mut out = Vec::new();
    let mut run_start: Option<usize> = None;
    for i in 0..n {
        let changed = pre.get(i) != post.get(i);
        match (changed, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(start)) => {
                out.push((start as u32, (i - start) as u32));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        out.push((start as u32, (n - start) as u32));
    }
    out
}

/// First byte offset in `[start, end)` NOT covered by the union of `cover`
/// (each entry `(offset, size)`), or `None` if `[start, end)` is fully
/// covered.
///
/// All arithmetic is `u64` so a `u32::MAX`-sized whole-account range cannot
/// overflow. This is a standard interval-union coverage sweep: repeatedly
/// jump the cursor to the far end of whichever covering interval reaches
/// furthest past it; if nothing covers the cursor, that is the gap.
fn first_gap(start: u64, end: u64, cover: &[(u64, u64)]) -> Option<u64> {
    let mut cursor = start;
    while cursor < end {
        let mut furthest = cursor;
        for &(offset, size) in cover {
            let cover_end = offset + size;
            if offset <= cursor && cursor < cover_end && cover_end > furthest {
                furthest = cover_end;
            }
        }
        if furthest == cursor {
            return Some(cursor);
        }
        cursor = furthest;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_intervals_coalesces_adjacent_bytes() {
        // paused (114) then revision low byte (115) both flip 0 -> 1: two
        // adjacent single-byte changes coalesce into one (114, 2) interval.
        let mut pre = vec![0u8; 200];
        let mut post = pre.clone();
        post[114] = 1;
        post[115] = 1;
        assert_eq!(changed_intervals(&pre, &post), vec![(114, 2)]);

        // Non-adjacent changes stay separate.
        pre = vec![0u8; 200];
        let mut post2 = pre.clone();
        post2[10] = 9;
        post2[50] = 9;
        assert_eq!(changed_intervals(&pre, &post2), vec![(10, 1), (50, 1)]);
    }

    #[test]
    fn changed_intervals_flags_length_growth() {
        let pre = vec![0u8; 4];
        let post = vec![0u8; 8];
        assert_eq!(changed_intervals(&pre, &post), vec![(4, 4)]);
    }

    #[test]
    fn first_gap_detects_coverage_and_gaps() {
        // [114..123) fully covered by paused(114,1) + revision(115,8).
        assert_eq!(first_gap(114, 123, &[(114, 1), (115, 8)]), None);
        // A byte at 16 with only a (114,1) cover is uncovered at 16.
        assert_eq!(first_gap(16, 17, &[(114, 1)]), Some(16));
        // Whole-account cover (0, u32::MAX) covers everything, no overflow.
        assert_eq!(first_gap(48, 56, &[(0, u32::MAX as u64)]), None);
        // A partial cover leaves the tail uncovered.
        assert_eq!(first_gap(0, 10, &[(0, 4)]), Some(4));
    }
}
