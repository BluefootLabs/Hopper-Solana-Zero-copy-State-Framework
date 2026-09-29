//! Runtime lab: the parts of Hopper that only mean something on the real
//! VM, each behind one instruction.
//!
//! - The heap allocator, with a declared 256 KiB heap: one large
//!   allocation, a vector grown in place, and a loop that gives the heap
//!   back with a checkpoint.
//! - The panic handler: with the `report-panics` feature the runtime logs
//!   the message and `file:line:column`; without it the program aborts
//!   and logs nothing.
//! - The SlotHashes lookup: the hash of a slot a given distance behind the
//!   newest one, with the reason when there is none and the number of
//!   sysvar reads it took.
//!
//! Every instruction answers through return data, so a test or a devnet
//! runner checks values, not only success.
#![cfg_attr(target_os = "solana", no_std)]
#![cfg_attr(not(target_os = "solana"), allow(dead_code))]

extern crate alloc;

use alloc::vec::Vec;
use hopper::prelude::*;
use hopper::sysvar::{slot_hash_lookup, slot_hashes_latest, SlotHashStatus};

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::default_allocator!(heap = 256 * 1024);
    hopper::nostd_panic_handler!();
}

hopper::hopper_error! {
    base = 6800;
    AllocationRefused,
    PatternMismatch,
    HeapNotReleased,
}

#[derive(Accounts)]
pub struct Caller<'info> {
    pub caller: Signer<'info>,
}

/// Status bytes of the `slot_hash` answers.
pub const STATUS_FOUND: u8 = 0;
pub const STATUS_SKIPPED: u8 = 1;
pub const STATUS_TOO_OLD: u8 = 2;
pub const STATUS_AHEAD: u8 = 3;

fn status_byte(status: &SlotHashStatus) -> u8 {
    match status {
        SlotHashStatus::Found(_) => STATUS_FOUND,
        SlotHashStatus::Skipped => STATUS_SKIPPED,
        SlotHashStatus::TooOld => STATUS_TOO_OLD,
        SlotHashStatus::Ahead => STATUS_AHEAD,
    }
}

#[program]
mod runtime_lab {
    use super::*;

    /// Allocate `bytes` in one block, fill it, write a marker on every
    /// page, and read the markers back. Returns `[used: u64][pages: u64]`.
    /// A block past 32 KiB needs the transaction to request the heap frame.
    #[instruction(0)]
    pub fn allocate(_ctx: Ctx<Caller>, bytes: u32) -> ProgramResult {
        const PAGE: usize = 4096;
        let bytes = bytes as usize;
        let mut block: Vec<u8> = Vec::new();
        block
            .try_reserve_exact(bytes)
            .map_err(|_| ProgramError::from(AllocationRefused))?;
        // One fill touches every byte of the block, the last included.
        block.resize(bytes, 0x5a);
        let mut pages = 0u64;
        let mut at = 0usize;
        while at < bytes {
            block[at] = (at / PAGE) as u8;
            at += PAGE;
            pages += 1;
        }
        let mut at = 0usize;
        while at < bytes {
            hopper::hopper_require!(block[at] == (at / PAGE) as u8, PatternMismatch);
            at += PAGE;
        }
        if let Some(last) = block.last() {
            hopper::hopper_require!(
                *last == 0x5a || (bytes - 1).is_multiple_of(PAGE),
                PatternMismatch
            );
        }
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&(hopper::heap::used() as u64).to_le_bytes());
        out[8..].copy_from_slice(&pages.to_le_bytes());
        hopper::return_data::set_return_data(&out);
        Ok(())
    }

    /// Grow one vector to `kib` KiB a kilobyte at a time. The vector is
    /// the most recent allocation, so every growth happens in place and
    /// the heap used at the end is the vector's capacity, not the sum of
    /// the sizes it passed through. Returns `[used: u64][capacity: u64]`.
    #[instruction(1)]
    pub fn grow(_ctx: Ctx<Caller>, kib: u16) -> ProgramResult {
        let mut buffer: Vec<u8> = Vec::new();
        let mut step = 0u16;
        while step < kib {
            buffer
                .try_reserve_exact(1024)
                .map_err(|_| ProgramError::from(AllocationRefused))?;
            buffer.extend_from_slice(&[step as u8; 1024]);
            step += 1;
        }
        if let (Some(first), Some(last)) = (buffer.first(), buffer.last()) {
            hopper::hopper_require!(*first == 0, PatternMismatch);
            hopper::hopper_require!(*last == (kib - 1) as u8, PatternMismatch);
        }
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&(hopper::heap::used() as u64).to_le_bytes());
        out[8..].copy_from_slice(&(buffer.capacity() as u64).to_le_bytes());
        hopper::return_data::set_return_data(&out);
        Ok(())
    }

    /// Allocate `bytes` and drop it, `rounds` times, releasing the heap
    /// to a checkpoint after each round. Without the checkpoint the loop
    /// would exhaust the heap after a few rounds. Returns
    /// `[used_after: u64][peak: u64]`.
    #[instruction(2)]
    pub fn checkpoint_loop(_ctx: Ctx<Caller>, bytes: u32, rounds: u16) -> ProgramResult {
        let mark = hopper::heap::mark();
        let before = hopper::heap::used();
        let mut peak = 0usize;
        let mut round = 0u16;
        while round < rounds {
            {
                let mut scratch: Vec<u8> = Vec::new();
                scratch
                    .try_reserve_exact(bytes as usize)
                    .map_err(|_| ProgramError::from(AllocationRefused))?;
                scratch.resize(bytes as usize, round as u8);
                hopper::hopper_require!(
                    scratch.last().copied().unwrap_or(round as u8) == round as u8,
                    PatternMismatch
                );
                let used = hopper::heap::used();
                if used > peak {
                    peak = used;
                }
            }
            // SAFETY: `scratch` was dropped at the end of the block above;
            // nothing allocated since `mark` is alive.
            unsafe { hopper::heap::release_to(mark) };
            round += 1;
        }
        hopper::hopper_require!(hopper::heap::used() == before, HeapNotReleased);
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&(hopper::heap::used() as u64).to_le_bytes());
        out[8..].copy_from_slice(&(peak as u64).to_le_bytes());
        hopper::return_data::set_return_data(&out);
        Ok(())
    }

    /// Panic with a message that carries `code`.
    #[instruction(3)]
    pub fn panic_now(_ctx: Ctx<Caller>, code: u8) -> ProgramResult {
        panic!("runtime lab panic, code {}", code);
    }

    /// Look up the slot `back` slots behind the newest SlotHashes entry.
    /// Returns `[newest: u64][target: u64][status][reads][hash: 32]`.
    #[instruction(4)]
    pub fn slot_hash_back(_ctx: Ctx<Caller>, back: u64) -> ProgramResult {
        let newest = match slot_hashes_latest()? {
            Some(entry) => entry.slot,
            None => return Err(ProgramError::UnsupportedSysvar),
        };
        answer(newest, newest.saturating_sub(back))
    }

    /// Look up `slot` itself. Same answer as [`slot_hash_back`].
    #[instruction(5)]
    pub fn slot_hash_at(_ctx: Ctx<Caller>, slot: u64) -> ProgramResult {
        let newest = match slot_hashes_latest()? {
            Some(entry) => entry.slot,
            None => return Err(ProgramError::UnsupportedSysvar),
        };
        answer(newest, slot)
    }
}

fn answer(newest: u64, target: u64) -> ProgramResult {
    let lookup = slot_hash_lookup(target)?;
    let mut out = [0u8; 50];
    out[..8].copy_from_slice(&newest.to_le_bytes());
    out[8..16].copy_from_slice(&target.to_le_bytes());
    out[16] = status_byte(&lookup.status);
    out[17] = lookup.reads;
    if let Some(hash) = lookup.hash() {
        out[18..].copy_from_slice(&hash);
    }
    hopper::return_data::set_return_data(&out);
    Ok(())
}
