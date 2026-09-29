//! The hash of a given slot, read from the SlotHashes sysvar without
//! copying it.
//!
//! SlotHashes holds the last 512 `(slot, hash)` pairs, newest first:
//! 20,488 bytes. Copying it into a program costs a heap buffer larger than
//! most programs' whole heap, and passing it as an account costs a slot in
//! the transaction. Neither is needed: `sol_get_sysvar` copies any range of
//! a sysvar, and it charges the same for 40 bytes as for 2,400 (the syscall
//! base plus one memory-operation floor, 110 compute units).
//!
//! So the lookup reads windows of [`WINDOW`] entries.
//!
//! 1. One read takes the entry count and the newest window. A commit-reveal
//!    or replay guard asks about a slot a few slots old, and that read
//!    answers it: 110 compute units.
//! 2. For an older slot, the entry for slot `s` sits at index
//!    `newest - s` when no slot in between was skipped, and at a lower
//!    index when some were. The second read is the window that ends at
//!    that index, which holds the entry unless more than a window of slots
//!    were skipped in between: 220 compute units.
//! 3. Otherwise the search interpolates between the slots it has read on
//!    each side, and halves the range on every other probe so that slots
//!    spread unevenly cannot slow it down. A few more reads finish the job.
//!
//! The lookup says why a slot has no hash: it was skipped (no block was
//! produced), it is older than the sysvar reaches, or it is ahead of the
//! newest entry. A program that must tell "skipped" from "too old" does
//! not have to guess.
//!
//! The search is written over a reader function, so it runs in unit tests
//! against a synthetic sysvar image; [`slot_hash`] plugs in the syscall.

use crate::error::ProgramError;
use crate::sysvar::{get_sysvar_prefix_at, SLOT_HASHES_ID};

/// Entries read per syscall. Sixteen entries are 640 bytes on the stack
/// and cost what one entry costs.
pub const WINDOW: usize = 16;

/// Bytes of one `(slot, hash)` entry.
pub const ENTRY_LEN: usize = 40;

/// The most entries SlotHashes holds.
pub const MAX_ENTRIES: usize = 512;

const HEADER_LEN: usize = 8;

/// What the sysvar says about a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotHashStatus {
    /// The slot produced a block with this hash.
    Found([u8; 32]),
    /// The slot is inside the range the sysvar covers and has no entry: no
    /// block was produced in it.
    Skipped,
    /// The slot is older than the oldest entry.
    TooOld,
    /// The slot is newer than the newest entry (the current slot's hash is
    /// not known until the slot ends), or the sysvar is empty.
    Ahead,
}

/// The answer and what it cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotHashLookup {
    pub status: SlotHashStatus,
    /// Sysvar reads made, each 110 compute units on chain.
    pub reads: u8,
}

impl SlotHashLookup {
    /// The hash, when the slot has one.
    #[inline(always)]
    pub const fn hash(&self) -> Option<[u8; 32]> {
        match self.status {
            SlotHashStatus::Found(hash) => Some(hash),
            _ => None,
        }
    }
}

/// The hash of `slot`, or `None` when the sysvar has none for it.
/// [`slot_hash_lookup`] says which of the three reasons applies.
#[inline]
pub fn slot_hash(slot: u64) -> Result<Option<[u8; 32]>, ProgramError> {
    Ok(slot_hash_lookup(slot)?.hash())
}

/// Look `slot` up in the SlotHashes sysvar.
#[inline]
pub fn slot_hash_lookup(slot: u64) -> Result<SlotHashLookup, ProgramError> {
    slot_hash_lookup_with(slot, |offset, dst| {
        get_sysvar_prefix_at(&SLOT_HASHES_ID, offset, dst)
    })
}

#[inline(always)]
fn entry_slot(window: &[u8], index: usize) -> u64 {
    let at = index * ENTRY_LEN;
    u64::from_le_bytes([
        window[at],
        window[at + 1],
        window[at + 2],
        window[at + 3],
        window[at + 4],
        window[at + 5],
        window[at + 6],
        window[at + 7],
    ])
}

#[inline(always)]
fn entry_hash(window: &[u8], index: usize) -> [u8; 32] {
    let at = index * ENTRY_LEN + 8;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&window[at..at + 32]);
    hash
}

/// The search over any reader of the sysvar's bytes.
///
/// `read(offset, dst)` fills `dst` from the sysvar starting at `offset`
/// and returns `Ok(true)`, or returns `Ok(false)` when the range runs past
/// the sysvar's end, the contract of
/// [`get_sysvar_prefix_at`](crate::sysvar::get_sysvar_prefix_at).
pub fn slot_hash_lookup_with<R>(target: u64, mut read: R) -> Result<SlotHashLookup, ProgramError>
where
    R: FnMut(u64, &mut [u8]) -> Result<bool, ProgramError>,
{
    let mut reads = 0u8;
    let done = |status, reads| Ok(SlotHashLookup { status, reads });

    // The count and the newest window in one read. A sysvar shorter than
    // that (a fresh test validator) is read again at its real length.
    let mut head = [0u8; HEADER_LEN + WINDOW * ENTRY_LEN];
    reads += 1;
    let count = if read(0, &mut head)? {
        read_count(&head)?
    } else {
        reads += 1;
        if !read(0, &mut head[..HEADER_LEN])? {
            return Err(ProgramError::UnsupportedSysvar);
        }
        let count = read_count(&head)?;
        if count >= WINDOW {
            // The long read failed although the sysvar holds a full
            // window: the header does not describe the sysvar.
            return Err(ProgramError::UnsupportedSysvar);
        }
        if count > 0 {
            reads += 1;
            if !read(0, &mut head[..HEADER_LEN + count * ENTRY_LEN])? {
                return Err(ProgramError::UnsupportedSysvar);
            }
        }
        count
    };
    if count == 0 {
        return done(SlotHashStatus::Ahead, reads);
    }

    let have = if count < WINDOW { count } else { WINDOW };
    let window = &head[HEADER_LEN..HEADER_LEN + have * ENTRY_LEN];
    let newest = entry_slot(window, 0);
    if target > newest {
        return done(SlotHashStatus::Ahead, reads);
    }
    if let Some(status) = scan(window, have, target, true) {
        return done(status, reads);
    }
    // Every entry of the newest window is newer than the target.
    if have == count {
        return done(SlotHashStatus::TooOld, reads);
    }

    // Entries are strictly descending, so entry `i` has a slot of at most
    // `newest - i`: the target's entry, if any, is at index `newest -
    // target` or below it, and exactly there when no slot in between was
    // skipped.
    let last = count - 1;
    let bound = newest - target;
    // Entries above `hi` cannot be the target.
    let mut hi = if bound > last as u64 {
        last
    } else {
        bound as usize
    };
    // Invariant: every entry below `lo` is newer than the target.
    let mut lo = have;
    // The nearest entry on each side of the range that was actually read:
    // the search interpolates between their slots.
    let mut newer = (have - 1, entry_slot(window, have - 1));
    let mut older: Option<(usize, u64)> = None;
    let mut buffer = [0u8; WINDOW * ENTRY_LEN];
    let mut bisect = false;
    while lo <= hi {
        let span = hi - lo + 1;
        let width = if span < WINDOW { span } else { WINDOW };
        let highest_start = hi + 1 - width;
        let start = match older {
            // Nothing older has been read: the window that ends at the
            // bound, where the entry is when few slots were skipped.
            None => highest_start,
            // Every other probe halves the range, so a run of slots
            // spread unevenly cannot make the search walk.
            Some(_) if bisect => lo + (span - width) / 2,
            // Where the target falls between the two entries if the slots
            // between them are spread evenly.
            Some((older_index, older_slot)) => {
                let gap = (older_index - newer.0) as u128;
                let run = (newer.1 - older_slot) as u128;
                let guess = newer.0 + ((newer.1 - target) as u128 * gap / run) as usize;
                let centred = guess.saturating_sub(width / 2);
                if centred < lo {
                    lo
                } else if centred > highest_start {
                    highest_start
                } else {
                    centred
                }
            }
        };
        bisect = older.is_some() && !bisect;
        let window = &mut buffer[..width * ENTRY_LEN];
        reads = reads.saturating_add(1);
        let offset = (HEADER_LEN + start * ENTRY_LEN) as u64;
        if !read(offset, window)? {
            return Err(ProgramError::UnsupportedSysvar);
        }
        // The entry before this window is known to be newer than the
        // target only when the window starts at `lo`.
        if let Some(status) = scan(window, width, target, start == lo) {
            return done(status, reads);
        }
        let first_slot = entry_slot(window, 0);
        if first_slot < target {
            // The whole window is older than the target.
            older = Some((start, first_slot));
            hi = start - 1;
        } else {
            // The whole window is newer than the target.
            let end = start + width - 1;
            newer = (end, entry_slot(window, width - 1));
            lo = end + 1;
        }
    }
    // `lo` passed `hi`, so every entry that could be the target is newer
    // than it. If an older entry was read, the target's slot lies between
    // two entries and has none of its own. If none was, the range ran to
    // the last entry, and the target is older than the sysvar reaches.
    if older.is_some() {
        done(SlotHashStatus::Skipped, reads)
    } else {
        done(SlotHashStatus::TooOld, reads)
    }
}

#[inline(always)]
fn read_count(head: &[u8]) -> Result<usize, ProgramError> {
    let count = u64::from_le_bytes([
        head[0], head[1], head[2], head[3], head[4], head[5], head[6], head[7],
    ]);
    if count > MAX_ENTRIES as u64 {
        return Err(ProgramError::UnsupportedSysvar);
    }
    Ok(count as usize)
}

/// Look for `target` among the `len` entries of `window`. `Found` when it
/// is there; `Skipped` when the window shows an entry newer than the
/// target directly followed by an older one (the entry before the window
/// counts as newer when `newer_before` says so); `None` when the window
/// does not decide.
#[inline(always)]
fn scan(window: &[u8], len: usize, target: u64, newer_before: bool) -> Option<SlotHashStatus> {
    let mut i = 0;
    while i < len {
        let slot = entry_slot(window, i);
        if slot == target {
            return Some(SlotHashStatus::Found(entry_hash(window, i)));
        }
        if slot < target {
            return if i > 0 || newer_before {
                Some(SlotHashStatus::Skipped)
            } else {
                None
            };
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    fn hash_of(slot: u64) -> [u8; 32] {
        let mut hash = [0u8; 32];
        hash[..8].copy_from_slice(&slot.to_le_bytes());
        hash[8..16].copy_from_slice(&(!slot).to_le_bytes());
        hash[31] = 0x5a;
        hash
    }

    /// A sysvar image holding `slots`, which must be strictly descending.
    fn image(slots: &[u64]) -> Vec<u8> {
        let mut out = (slots.len() as u64).to_le_bytes().to_vec();
        for slot in slots {
            out.extend_from_slice(&slot.to_le_bytes());
            out.extend_from_slice(&hash_of(*slot));
        }
        out
    }

    fn lookup(image: &[u8], target: u64) -> SlotHashLookup {
        let mut reads = 0u8;
        let result = slot_hash_lookup_with(target, |offset, dst| {
            reads += 1;
            let start = offset as usize;
            match start.checked_add(dst.len()) {
                Some(end) if end <= image.len() => {
                    dst.copy_from_slice(&image[start..end]);
                    Ok(true)
                }
                _ => Ok(false),
            }
        })
        .unwrap();
        assert_eq!(result.reads, reads, "the lookup counts its own reads");
        result
    }

    /// The answer by definition, from the list itself.
    fn expected(slots: &[u64], target: u64) -> SlotHashStatus {
        match slots.first() {
            None => SlotHashStatus::Ahead,
            Some(newest) if target > *newest => SlotHashStatus::Ahead,
            _ if slots.contains(&target) => SlotHashStatus::Found(hash_of(target)),
            _ if target < *slots.last().unwrap() => SlotHashStatus::TooOld,
            _ => SlotHashStatus::Skipped,
        }
    }

    /// Descending slots from `newest`, `count` of them, skipping a slot
    /// wherever `skip` says so.
    fn chain(newest: u64, count: usize, mut skip: impl FnMut(u64) -> bool) -> Vec<u64> {
        let mut slots = Vec::new();
        let mut slot = newest;
        while slots.len() < count {
            if slots.is_empty() || !skip(slot) {
                slots.push(slot);
            }
            if slot == 0 {
                break;
            }
            slot -= 1;
        }
        slots
    }

    fn check_every_slot(slots: &[u64]) -> u8 {
        let sysvar = image(slots);
        let newest = slots.first().copied().unwrap_or(0);
        let oldest = slots.last().copied().unwrap_or(0);
        let mut worst = 0;
        let low = oldest.saturating_sub(3);
        for target in low..=newest + 3 {
            let got = lookup(&sysvar, target);
            assert_eq!(
                got.status,
                expected(slots, target),
                "target {target} in a list of {} from {newest} to {oldest}",
                slots.len()
            );
            worst = worst.max(got.reads);
        }
        worst
    }

    #[test]
    fn a_full_sysvar_with_no_skips_answers_in_two_reads() {
        let slots = chain(1_000_000, MAX_ENTRIES, |_| false);
        let sysvar = image(&slots);
        assert_eq!(sysvar.len(), 20_488);
        // The newest window: one read.
        for back in 0..WINDOW as u64 {
            let got = lookup(&sysvar, 1_000_000 - back);
            assert_eq!(got.hash(), Some(hash_of(1_000_000 - back)));
            assert_eq!(got.reads, 1);
        }
        // Anything older: the window that ends at the expected index.
        for back in WINDOW as u64..MAX_ENTRIES as u64 {
            let got = lookup(&sysvar, 1_000_000 - back);
            assert_eq!(got.hash(), Some(hash_of(1_000_000 - back)));
            assert_eq!(got.reads, 2, "{back} slots back");
        }
        assert_eq!(check_every_slot(&slots), 2);
    }

    #[test]
    fn skipped_slots_are_reported_and_cost_little() {
        // About one slot in twenty skipped, the rate of a busy cluster.
        let slots = chain(5_000_000, MAX_ENTRIES, |slot| slot % 19 == 7);
        let worst = check_every_slot(&slots);
        assert!(worst <= 4, "{worst} reads in the worst case");
        let sysvar = image(&slots);
        let skipped = (4_999_900..5_000_000u64).find(|s| s % 19 == 7).unwrap();
        assert_eq!(lookup(&sysvar, skipped).status, SlotHashStatus::Skipped);
        // Few skips in between: still two reads for a slot 100 back.
        assert_eq!(lookup(&sysvar, 5_000_000 - 100).reads, 2);
    }

    #[test]
    fn long_gaps_fall_back_to_the_search() {
        // A cluster that lost most of its slots: every third slot lands.
        let slots = chain(9_000, MAX_ENTRIES, |slot| slot % 3 != 0);
        let worst = check_every_slot(&slots);
        assert!(worst <= 8, "{worst} reads in the worst case");
        // One gap of 300 slots in the middle of the list.
        let slots = chain(80_000, MAX_ENTRIES, |slot| (79_500..79_800).contains(&slot));
        let worst = check_every_slot(&slots);
        assert!(worst <= 8, "{worst} reads in the worst case");
    }

    #[test]
    fn every_length_from_empty_to_full() {
        for count in 0..=40usize {
            let slots = chain(700, count, |slot| slot % 5 == 1);
            check_every_slot(&slots);
        }
        for count in [63, 64, 65, 255, 256, 257, 511, 512] {
            let slots = chain(90_000, count, |slot| slot % 11 == 3);
            check_every_slot(&slots);
        }
        // A list that reaches slot zero.
        let slots = chain(30, 31, |_| false);
        assert_eq!(*slots.last().unwrap(), 0);
        check_every_slot(&slots);
    }

    #[test]
    fn the_reasons_are_told_apart() {
        let slots = chain(1_000, 100, |slot| slot == 950);
        let sysvar = image(&slots);
        let oldest = *slots.last().unwrap();
        assert_eq!(lookup(&sysvar, 1_001).status, SlotHashStatus::Ahead);
        assert_eq!(lookup(&sysvar, u64::MAX).status, SlotHashStatus::Ahead);
        assert_eq!(lookup(&sysvar, 950).status, SlotHashStatus::Skipped);
        assert_eq!(lookup(&sysvar, oldest - 1).status, SlotHashStatus::TooOld);
        assert_eq!(lookup(&sysvar, 0).status, SlotHashStatus::TooOld);
        assert_eq!(lookup(&sysvar, oldest).hash(), Some(hash_of(oldest)));
        assert_eq!(lookup(&image(&[]), 5).status, SlotHashStatus::Ahead);
    }

    #[test]
    fn a_header_that_does_not_describe_the_sysvar_is_refused() {
        // A count above the sysvar's maximum.
        let mut sysvar = image(&chain(100, 20, |_| false));
        sysvar[..8].copy_from_slice(&513u64.to_le_bytes());
        let result = slot_hash_lookup_with(90, |offset, dst| {
            let start = offset as usize;
            if start + dst.len() > sysvar.len() {
                return Ok(false);
            }
            dst.copy_from_slice(&sysvar[start..start + dst.len()]);
            Ok(true)
        });
        assert_eq!(result.err(), Some(ProgramError::UnsupportedSysvar));

        // A count that promises more entries than the sysvar holds.
        let mut sysvar = image(&chain(100, 20, |_| false));
        sysvar[..8].copy_from_slice(&400u64.to_le_bytes());
        let result = slot_hash_lookup_with(60, |offset, dst| {
            let start = offset as usize;
            if start + dst.len() > sysvar.len() {
                return Ok(false);
            }
            dst.copy_from_slice(&sysvar[start..start + dst.len()]);
            Ok(true)
        });
        assert_eq!(result.err(), Some(ProgramError::UnsupportedSysvar));

        // A reader that fails is reported, not swallowed.
        let result = slot_hash_lookup_with(60, |_, _| Err(ProgramError::InvalidArgument));
        assert_eq!(result.err(), Some(ProgramError::InvalidArgument));
    }
}
