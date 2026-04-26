//! Solana program input-buffer serialization, Hopper-native.
//!
//! When `solana-bpf-loader-program` invokes a `.so` it serialises
//! the instruction's accounts + data + program ID into a single
//! contiguous buffer that lives in the VM's input region. The
//! program's `process_instruction` reads accounts back out of that
//! buffer through `solana_program::entrypoint::deserialize`. Any
//! mutation the program makes — lamports, data, owner, data length
//! (realloc) — is written back into the same buffer; the host
//! reads the post-state from it after the VM returns.
//!
//! This module owns both directions of that round-trip. **Pure bit
//! manipulation against a published spec, zero `solana-sbpf`
//! coupling**, so it can be unit-tested in isolation.
//!
//! ## Wire format (aligned variant, what `cargo build-sbf` emits)
//!
//! ```text
//! u64                    num_accounts
//! for each account:
//!   if duplicate of earlier slot j:
//!     u8                 dup_index = j (0..N)
//!     [u8; 7]            padding
//!   else:
//!     u8                 dup_marker = 0xff
//!     u8                 is_signer
//!     u8                 is_writable
//!     u8                 executable
//!     u32                original_data_len
//!     [u8; 32]           pubkey
//!     [u8; 32]           owner
//!     u64                lamports
//!     u64                data_len
//!     [u8; data_len]     data
//!     [u8; 10240]        realloc padding (MAX_PERMITTED_DATA_INCREASE)
//!     [u8; pad]          align to 8 bytes
//!     u64                rent_epoch
//! u64                    ix_data_len
//! [u8; ix_data_len]      ix_data
//! [u8; 32]               program_id
//! ```
//!
//! Constants:
//!
//! - `MAX_PERMITTED_DATA_INCREASE = 10240` — the realloc tail every
//!   account gets, so programs can `account.realloc(new_size)` up
//!   to `original_data_len + 10240` without the host having to grow
//!   the buffer mid-execution.
//! - `BPF_ALIGN_OF_U128 = 8` — alignment between an account's
//!   variable-length region and the trailing `rent_epoch` field.
//!
//! ## Implementation notes
//!
//! - Duplicate-account detection scans previous accounts in O(N²).
//!   For typical Solana instructions (≤ 12 accounts) this is fine;
//!   if a future test pushes thousands of accounts the loop would
//!   want a hash-map shortcut, but the ceiling on real instruction
//!   account counts makes the simpler loop the right call.
//! - We pre-compute the buffer length and allocate once to avoid
//!   resize copies. The length math is exact.
//! - Account *order* is preserved: the i-th metadata in the
//!   instruction is the i-th account in the buffer, no matter
//!   how many duplicates appear.

use crate::account::KeyedAccount;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;

/// Realloc tail size — the on-chain runtime guarantees programs
/// `MAX_PERMITTED_DATA_INCREASE` bytes of growable space after each
/// account's data, so a `realloc(new_size)` call can grow up to
/// that limit without host involvement.
pub const MAX_PERMITTED_DATA_INCREASE: usize = 10_240;

/// Alignment between the variable-length account region and the
/// trailing `rent_epoch` field. Matches `BPF_ALIGN_OF_U128` in the
/// upstream loader.
pub const BPF_ALIGN_OF_U128: usize = 8;

/// Marker byte indicating an account is unique (not a duplicate of
/// an earlier slot). Anything else (0..N) is the slot index of the
/// earlier occurrence.
pub const NON_DUP_MARKER: u8 = 0xff;

/// Per-account offsets into the serialized buffer. Used by
/// [`deserialize_parameters`] to read the post-state back out
/// without re-running the duplicate-detection scan.
#[derive(Debug, Clone)]
pub struct AccountOffset {
    /// Whether this slot is a duplicate of an earlier one. When
    /// `Some(i)`, the account state lives at offset
    /// `slot_offset[i].body_start`; this slot only contributes the
    /// 8-byte dup header.
    pub duplicate_of: Option<usize>,
    /// Byte offset where the unique account record begins. For a
    /// duplicate slot, this is the offset of the 8-byte dup header
    /// — not the slot it duplicates.
    pub record_start: usize,
    /// Byte offset of the `lamports` field inside this account's
    /// record. Only valid for unique slots.
    pub lamports_offset: usize,
    /// Byte offset of the `data_len` field. Only valid for unique
    /// slots. The program may have rewritten this on realloc.
    pub data_len_offset: usize,
    /// Byte offset of the `data` region. Length grows up to the
    /// realloc tail; reads use `data_len_offset` to determine the
    /// actual length.
    pub data_offset: usize,
    /// Byte offset of the `owner` field. The program may have
    /// rewritten this via system_instruction::assign.
    pub owner_offset: usize,
    /// `original_data_len` written into the header. Bounds the
    /// post-execution `data_len` (must be ≤ original + realloc tail).
    pub original_data_len: usize,
}

/// Serialised parameter buffer + the offsets needed to read it back.
///
/// Holds the full byte buffer that lives in the VM's input region,
/// plus a per-account offset table so deserialization doesn't need
/// to redo duplicate detection. This is the only thing
/// [`serialize_parameters`] returns and the only thing
/// [`deserialize_parameters`] needs to consume.
pub struct Parameters {
    /// The serialized byte buffer. Hand to the VM's input
    /// `MemoryRegion`.
    pub buffer: Vec<u8>,
    /// Per-account offsets, index-aligned with the input
    /// `account_metas` slice.
    pub offsets: Vec<AccountOffset>,
}

/// Compute the on-disk length of a serialised parameter buffer for
/// a given set of accounts + ix data, without actually building
/// it. Useful for tests that want to assert the buffer size matches
/// expectations.
pub fn serialized_length(
    metas: &[AccountMeta],
    accounts_by_meta: &[&KeyedAccount],
    ix_data_len: usize,
) -> usize {
    let mut len = 8; // num_accounts u64
    let mut seen: Vec<Pubkey> = Vec::with_capacity(metas.len());
    for (i, meta) in metas.iter().enumerate() {
        if seen.iter().any(|p| p == &meta.pubkey) {
            len += 8; // dup header (1 byte index + 7 bytes padding)
        } else {
            seen.push(meta.pubkey);
            len += per_account_unique_size(accounts_by_meta[i].data.len());
        }
    }
    len += 8; // ix_data_len
    len += ix_data_len;
    len += 32; // program_id
    len
}

/// Per-account length when the slot is a unique (non-duplicate)
/// occurrence. The 88-byte fixed header + variable data + 10240
/// realloc tail + alignment padding + 8-byte rent_epoch.
fn per_account_unique_size(data_len: usize) -> usize {
    // Fixed header before data:
    //   1  dup
    //   1  is_signer
    //   1  is_writable
    //   1  executable
    //   4  original_data_len
    //   32 pubkey
    //   32 owner
    //   8  lamports
    //   8  data_len
    //  ───
    //   88 bytes
    let header = 88usize;
    let body = data_len + MAX_PERMITTED_DATA_INCREASE;
    let aligned = align_up(header + body, BPF_ALIGN_OF_U128);
    aligned + 8 // rent_epoch
}

/// Round `n` up to the nearest multiple of `align`. `align` must be
/// a power of two.
fn align_up(n: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (n + align - 1) & !(align - 1)
}

/// Serialise an instruction's accounts + ix data + program ID into
/// the canonical Solana parameter buffer.
///
/// `metas` is the instruction's `account_metas` slice. `accounts`
/// is the harness-side state, used to look up each meta by pubkey.
/// Returns the buffer + offset table; if any meta references a
/// pubkey not present in `accounts`, returns `Err(missing_pubkey)`.
pub fn serialize_parameters(
    metas: &[AccountMeta],
    accounts: &[KeyedAccount],
    ix_data: &[u8],
    program_id: &Pubkey,
) -> Result<Parameters, Pubkey> {
    // Resolve each meta to the matching account in `accounts`.
    let mut resolved: Vec<&KeyedAccount> = Vec::with_capacity(metas.len());
    for meta in metas {
        match accounts.iter().find(|a| a.address == meta.pubkey) {
            Some(a) => resolved.push(a),
            None => return Err(meta.pubkey),
        }
    }

    let total_len = serialized_length(metas, &resolved, ix_data.len());
    let mut buf = vec![0u8; total_len];
    let mut offsets: Vec<AccountOffset> = Vec::with_capacity(metas.len());
    let mut cursor = 0usize;

    // num_accounts u64 LE.
    buf[cursor..cursor + 8].copy_from_slice(&(metas.len() as u64).to_le_bytes());
    cursor += 8;

    // Per-account records.
    for (i, meta) in metas.iter().enumerate() {
        let record_start = cursor;
        // Duplicate detection: scan earlier slots for the same pubkey.
        let dup_of = (0..i).find(|&j| metas[j].pubkey == meta.pubkey);
        if let Some(j) = dup_of {
            // Duplicate header: 1 byte index + 7 bytes padding.
            buf[cursor] = j as u8;
            // Padding bytes are zeros (already from the initial vec!).
            cursor += 8;
            offsets.push(AccountOffset {
                duplicate_of: Some(j),
                record_start,
                lamports_offset: 0,
                data_len_offset: 0,
                data_offset: 0,
                owner_offset: 0,
                original_data_len: 0,
            });
            continue;
        }

        let acct = resolved[i];

        // Unique account record:
        // 1 byte dup marker
        buf[cursor] = NON_DUP_MARKER;
        cursor += 1;
        // 1 byte is_signer
        buf[cursor] = u8::from(meta.is_signer);
        cursor += 1;
        // 1 byte is_writable
        buf[cursor] = u8::from(meta.is_writable);
        cursor += 1;
        // 1 byte executable
        buf[cursor] = u8::from(acct.executable);
        cursor += 1;
        // 4 bytes original_data_len u32 LE
        let original_data_len = acct.data.len();
        buf[cursor..cursor + 4]
            .copy_from_slice(&(original_data_len as u32).to_le_bytes());
        cursor += 4;
        // 32 bytes pubkey
        buf[cursor..cursor + 32].copy_from_slice(meta.pubkey.as_ref());
        cursor += 32;
        // 32 bytes owner
        let owner_offset = cursor;
        buf[cursor..cursor + 32].copy_from_slice(acct.owner.as_ref());
        cursor += 32;
        // 8 bytes lamports
        let lamports_offset = cursor;
        buf[cursor..cursor + 8].copy_from_slice(&acct.lamports.to_le_bytes());
        cursor += 8;
        // 8 bytes data_len
        let data_len_offset = cursor;
        buf[cursor..cursor + 8]
            .copy_from_slice(&(original_data_len as u64).to_le_bytes());
        cursor += 8;
        // data
        let data_offset = cursor;
        buf[cursor..cursor + original_data_len].copy_from_slice(&acct.data);
        cursor += original_data_len;
        // realloc tail (zeroes)
        cursor += MAX_PERMITTED_DATA_INCREASE;
        // alignment pad to 8 bytes
        let unaligned_end = cursor;
        cursor = align_up(unaligned_end - record_start, BPF_ALIGN_OF_U128) + record_start;
        // rent_epoch u64 LE — Phase 2 leaves at 0; tests can override
        // through KeyedAccount::rent_epoch when that becomes
        // observable (Phase 2.1).
        buf[cursor..cursor + 8].copy_from_slice(&acct.rent_epoch.to_le_bytes());
        cursor += 8;

        offsets.push(AccountOffset {
            duplicate_of: None,
            record_start,
            lamports_offset,
            data_len_offset,
            data_offset,
            owner_offset,
            original_data_len,
        });
    }

    // ix_data_len u64 LE + ix_data
    buf[cursor..cursor + 8].copy_from_slice(&(ix_data.len() as u64).to_le_bytes());
    cursor += 8;
    buf[cursor..cursor + ix_data.len()].copy_from_slice(ix_data);
    cursor += ix_data.len();
    // program_id 32 bytes
    buf[cursor..cursor + 32].copy_from_slice(program_id.as_ref());
    cursor += 32;

    debug_assert_eq!(
        cursor, total_len,
        "serialize_parameters: cursor {cursor} != computed total {total_len}"
    );

    Ok(Parameters { buffer: buf, offsets })
}

/// Read account post-state out of the (potentially-mutated)
/// parameter buffer. Pairs with [`serialize_parameters`].
///
/// `metas` is the same slice that was serialised. `original` is the
/// pre-execution account state, used to fill in fields the buffer
/// doesn't observably write back (executable flag, address —
/// addresses can't change; the buffer only stores them by reference
/// to the meta).
///
/// On a duplicate slot, we read the post-state from the slot it
/// duplicates rather than from the dup-header bytes (which carry no
/// account state).
pub fn deserialize_parameters(
    params: &Parameters,
    metas: &[AccountMeta],
    original: &[KeyedAccount],
) -> Vec<KeyedAccount> {
    debug_assert_eq!(params.offsets.len(), metas.len());
    let buf = &params.buffer;

    // Read the unique-slot post-state once per pubkey, then fan out
    // to every meta that pointed at it.
    let mut per_pubkey_state: Vec<(Pubkey, KeyedAccount)> = Vec::new();
    for (i, meta) in metas.iter().enumerate() {
        let off = &params.offsets[i];
        if off.duplicate_of.is_some() {
            continue;
        }
        // Resolve the original record so we know the executable flag
        // and any pre-existing fields the program doesn't get to
        // observably mutate.
        let pre = original
            .iter()
            .find(|a| a.address == meta.pubkey)
            .cloned()
            .unwrap_or_else(|| {
                KeyedAccount::new(meta.pubkey, 0, Pubkey::default(), Vec::new(), false)
            });

        let lamports = u64::from_le_bytes(
            buf[off.lamports_offset..off.lamports_offset + 8]
                .try_into()
                .expect("lamports slice"),
        );
        let mut new_data_len = u64::from_le_bytes(
            buf[off.data_len_offset..off.data_len_offset + 8]
                .try_into()
                .expect("data_len slice"),
        ) as usize;
        // Clamp to the realloc ceiling. A program that grew past the
        // tail is a runtime error in production; we surface it here
        // as a clamp because we already capped the buffer at
        // original + tail. Tests that need the strict check go
        // through the engine, which detects realloc overflow before
        // calling this function.
        let max_len = off.original_data_len + MAX_PERMITTED_DATA_INCREASE;
        if new_data_len > max_len {
            new_data_len = max_len;
        }
        let data = buf[off.data_offset..off.data_offset + new_data_len].to_vec();
        let owner = Pubkey::new_from_array(
            buf[off.owner_offset..off.owner_offset + 32]
                .try_into()
                .expect("owner slice"),
        );
        let post = KeyedAccount {
            address: meta.pubkey,
            lamports,
            data,
            owner,
            executable: pre.executable,
            rent_epoch: pre.rent_epoch,
        };
        per_pubkey_state.push((meta.pubkey, post));
    }

    // Now build the output list — one entry per unique pubkey, in
    // first-occurrence order. Duplicate metas don't produce extra
    // entries.
    let mut out: Vec<KeyedAccount> = Vec::new();
    for (pk, st) in per_pubkey_state {
        if !out.iter().any(|a| a.address == pk) {
            out.push(st);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_for(pubkey: Pubkey, signer: bool, writable: bool) -> AccountMeta {
        AccountMeta {
            pubkey,
            is_signer: signer,
            is_writable: writable,
        }
    }

    /// `align_up` must round to the next multiple of the alignment.
    /// Pin against off-by-one drift.
    #[test]
    fn align_up_rounds_correctly() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);
    }

    /// `serialized_length` must match what `serialize_parameters`
    /// actually writes — they're both invoked at the same site to
    /// pre-compute the buffer size, and any drift between them
    /// causes either a panic (cursor overrun) or a buffer that's
    /// too long.
    #[test]
    fn serialized_length_matches_actual_serialization() {
        let alice = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let pid = Pubkey::new_unique();
        let acct = KeyedAccount::new(alice, 1_000, owner, vec![1, 2, 3, 4], false);
        let metas = vec![meta_for(alice, true, true)];
        let ix_data = vec![0xAA, 0xBB, 0xCC];
        let expected =
            serialized_length(&metas, &[&acct], ix_data.len());
        let params = serialize_parameters(&metas, &[acct], &ix_data, &pid).unwrap();
        assert_eq!(params.buffer.len(), expected);
    }

    /// Round-trip: serialize, then deserialize without mutation.
    /// The deserialized accounts must equal the originals (modulo
    /// fields the buffer doesn't carry, like rent_epoch which is
    /// passed through but defaults to 0 here).
    #[test]
    fn parameter_buffer_round_trips_unmutated() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let pid = Pubkey::new_unique();
        let originals = vec![
            KeyedAccount::new(alice, 5_000, owner, vec![1, 2, 3, 4, 5], false),
            KeyedAccount::new(bob, 0, Pubkey::default(), vec![], false),
        ];
        let metas = vec![meta_for(alice, true, true), meta_for(bob, false, false)];
        let ix_data = vec![0x01, 0x02];
        let params =
            serialize_parameters(&metas, &originals, &ix_data, &pid).unwrap();
        let back = deserialize_parameters(&params, &metas, &originals);
        assert_eq!(back.len(), 2);
        // Find each account by address and compare relevant fields.
        let a = back.iter().find(|a| a.address == alice).unwrap();
        assert_eq!(a.lamports, 5_000);
        assert_eq!(a.data, vec![1, 2, 3, 4, 5]);
        assert_eq!(a.owner, owner);
        let b = back.iter().find(|a| a.address == bob).unwrap();
        assert_eq!(b.lamports, 0);
        assert_eq!(b.data, Vec::<u8>::new());
    }

    /// Mutated buffer: rewrite lamports through the buffer's
    /// `lamports_offset`, then deserialize. The mutation must show up.
    /// This is the path the BPF engine relies on for observing
    /// program effects.
    #[test]
    fn mutations_through_buffer_show_up_in_deserialize() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let pid = Pubkey::new_unique();
        let originals = vec![
            KeyedAccount::new(alice, 5_000, owner, vec![1, 2, 3], false),
            KeyedAccount::new(bob, 100, owner, vec![], false),
        ];
        let metas = vec![meta_for(alice, true, true), meta_for(bob, false, true)];
        let mut params =
            serialize_parameters(&metas, &originals, &[], &pid).unwrap();
        // Rewrite alice's lamports through the offset table —
        // simulating a program that called `lamports.borrow_mut()`
        // and assigned a new balance.
        let off = &params.offsets[0];
        params.buffer[off.lamports_offset..off.lamports_offset + 8]
            .copy_from_slice(&123u64.to_le_bytes());
        // Same for owner (simulating `assign`).
        let new_owner = Pubkey::new_unique();
        params.buffer[off.owner_offset..off.owner_offset + 32]
            .copy_from_slice(new_owner.as_ref());

        let back = deserialize_parameters(&params, &metas, &originals);
        let a = back.iter().find(|a| a.address == alice).unwrap();
        assert_eq!(a.lamports, 123);
        assert_eq!(a.owner, new_owner);
    }

    /// Realloc-style data growth: rewrite data_len + the realloc
    /// tail bytes, deserialize, observe the new (longer) data.
    /// Pin against the realloc tail being honored as growable
    /// space.
    #[test]
    fn realloc_tail_is_observable_after_deserialize() {
        let alice = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let pid = Pubkey::new_unique();
        let originals = vec![KeyedAccount::new(
            alice,
            5_000,
            owner,
            vec![1, 2, 3, 4],
            false,
        )];
        let metas = vec![meta_for(alice, true, true)];
        let mut params =
            serialize_parameters(&metas, &originals, &[], &pid).unwrap();
        let off = &params.offsets[0];
        // Grow by 2 bytes.
        params.buffer[off.data_len_offset..off.data_len_offset + 8]
            .copy_from_slice(&6u64.to_le_bytes());
        // Write into the realloc tail (positions just after the
        // original 4 bytes of data).
        params.buffer[off.data_offset + 4] = 0xDE;
        params.buffer[off.data_offset + 5] = 0xAD;
        let back = deserialize_parameters(&params, &metas, &originals);
        let a = back.iter().find(|a| a.address == alice).unwrap();
        assert_eq!(a.data, vec![1, 2, 3, 4, 0xDE, 0xAD]);
    }

    /// Duplicate accounts: when meta[1] points at the same pubkey
    /// as meta[0], the second slot should be a 1-byte index + 7
    /// bytes of padding, and `deserialize_parameters` should
    /// produce only one entry per pubkey.
    #[test]
    fn duplicate_metas_serialize_compactly() {
        let alice = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let pid = Pubkey::new_unique();
        let originals = vec![KeyedAccount::new(alice, 10, owner, vec![1, 2], false)];
        let metas = vec![
            meta_for(alice, true, true),
            // Same pubkey as slot 0 — should serialize as duplicate.
            meta_for(alice, false, false),
        ];
        let params =
            serialize_parameters(&metas, &originals, &[], &pid).unwrap();
        assert!(params.offsets[0].duplicate_of.is_none());
        assert_eq!(params.offsets[1].duplicate_of, Some(0));
        // The duplicate slot's record_start should be right after
        // the unique record's full extent — we can check that the
        // index byte at that offset is 0 (pointing back to slot 0).
        let dup_start = params.offsets[1].record_start;
        assert_eq!(params.buffer[dup_start], 0);
        // Padding bytes 1..8 of the dup header are zero.
        assert_eq!(&params.buffer[dup_start + 1..dup_start + 8], &[0; 7]);

        // Deserialization yields one entry per unique pubkey.
        let back = deserialize_parameters(&params, &metas, &originals);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].address, alice);
    }

    /// Missing pubkey on serialization must error rather than
    /// silently produce a buffer with the wrong contents.
    #[test]
    fn missing_pubkey_returns_err() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let pid = Pubkey::new_unique();
        let originals = vec![KeyedAccount::new(
            alice,
            10,
            Pubkey::default(),
            vec![],
            false,
        )];
        let metas = vec![meta_for(alice, false, false), meta_for(bob, false, false)];
        let err =
            serialize_parameters(&metas, &originals, &[], &pid).unwrap_err();
        assert_eq!(err, bob);
    }
}
