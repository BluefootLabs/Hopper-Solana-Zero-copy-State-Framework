//! Raw loader input parsing for Hopper Native.
//!
//! This is the single source of truth for Solana loader input decoding. It owns
//! duplicate-account resolution, canonical-account lookup, and original-index
//! tracking so higher layers operate on already-resolved account views.

use core::mem::MaybeUninit;

use crate::account_view::AccountView;
use crate::address::Address;
use crate::raw_account::RuntimeAccount;
use crate::MAX_PERMITTED_DATA_INCREASE;

const BPF_ALIGN_OF_U128: usize = 8;

/// Malformed-input trap.
///
/// The Solana loader guarantees duplicate markers refer only to **earlier**
/// account slots (Solana's account serialization documents the marker as
/// "the index of the first account it is a duplicate of". necessarily a
/// lower index). A forward-pointing marker therefore cannot be the result
/// of a well-formed invocation: it either indicates a loader bug or
/// adversarial input attempting to synthesize an aliasing `AccountView`.
/// Pre-audit the parser silently fell back to account zero (or null for
/// slot 0), which produced either a null-pointer `AccountView` or an
/// aliasing view to an unrelated account. The Hopper Safety Audit flagged
/// this as the most urgent must-fix. We now trap immediately via
/// `sol_panic_` (on Solana) so the transaction fails at parse time.
#[inline(never)]
#[cold]
pub(crate) fn malformed_duplicate_marker(marker: u8, slot: usize) -> ! {
    #[cfg(target_os = "solana")]
    unsafe {
        // Keep the message short and on-chain-cheap. The loader log
        // attaches the program id automatically.
        const MSG: &[u8] = b"hopper: malformed duplicate marker";
        crate::syscalls::sol_panic_(MSG.as_ptr(), MSG.len() as u64, slot as u64, marker as u64);
    }
    #[cfg(not(target_os = "solana"))]
    {
        panic!(
            "hopper: malformed duplicate marker at slot {}: marker {} points forward",
            slot, marker
        );
    }
}

/// Metadata for one parsed account slot in the loader input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawAccountIndex {
    /// Index of this slot in the original loader account array.
    pub original_index: usize,
    /// Canonical account index this slot resolves to, if duplicated.
    pub duplicate_of: Option<usize>,
}

impl RawAccountIndex {
    /// Whether this slot is a duplicate reference to an earlier account.
    #[inline(always)]
    pub const fn is_duplicate(&self) -> bool {
        self.duplicate_of.is_some()
    }
}

/// Instruction tail discovered after scanning the loader input buffer.
#[derive(Clone)]
pub struct RawInstructionFrame {
    pub accounts_start: *mut u8,
    pub account_count: usize,
    pub instruction_data: &'static [u8],
    pub program_id: Address,
}

/// Deserialize the loader input into `AccountView`s.
///
/// Duplicate-account resolution happens here. A duplicate slot reuses the
/// canonical `RuntimeAccount` pointer of the earlier slot it references, and
/// its `original_index` remains the loader slot where it appeared.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
#[inline(always)]
pub unsafe fn deserialize_accounts<const MAX: usize>(
    input: *mut u8,
    accounts: &mut [MaybeUninit<AccountView>; MAX],
) -> (Address, usize, &'static [u8]) {
    let frame = unsafe { scan_instruction_frame(input) };

    let mut offset = 8usize;
    let count = frame.account_count.min(MAX);

    let mut slot = 0usize;
    while slot < count {
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            let raw = unsafe { input.add(offset) as *mut RuntimeAccount };
            accounts[slot] = MaybeUninit::new(unsafe { AccountView::new_unchecked(raw) });

            let data_len = unsafe { (*raw).data_len as usize };
            offset += RuntimeAccount::SIZE;
            offset += data_len + MAX_PERMITTED_DATA_INCREASE;
            offset += unsafe { input.add(offset).align_offset(BPF_ALIGN_OF_U128) };
            offset += 8;
        } else {
            let duplicate_of = marker as usize;
            // The marker must refer strictly to an earlier slot. Anything
            // else (forward reference, or a duplicate marker on slot 0
            // which has no prior slot to reference) is malformed loader
            // input. we trap rather than synthesize a null or aliasing
            // `AccountView`.
            if duplicate_of >= slot {
                malformed_duplicate_marker(marker, slot);
            }
            let raw = unsafe { accounts[duplicate_of].assume_init_ref().raw_ptr() };
            accounts[slot] = MaybeUninit::new(unsafe { AccountView::new_unchecked(raw) });
            offset += 8;
        }

        slot += 1;
    }

    while slot < frame.account_count {
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            let raw = unsafe { input.add(offset) as *const RuntimeAccount };
            let data_len = unsafe { (*raw).data_len as usize };
            offset += RuntimeAccount::SIZE;
            offset += data_len + MAX_PERMITTED_DATA_INCREASE;
            offset += unsafe { input.add(offset).align_offset(BPF_ALIGN_OF_U128) };
            offset += 8;
        } else {
            offset += 8;
        }
        slot += 1;
    }

    (frame.program_id, count, frame.instruction_data)
}

/// Fast two-argument deserialize: instruction data and program id are provided
/// directly by the caller (from the SVM's second entrypoint register), so the
/// full account-scan pass is skipped entirely.
///
/// # Safety
///
/// * `input` must point to a valid Solana BPF input buffer.
/// * `ix_data` must point to the instruction data with its length stored as
///   `u64` at offset `-8`.
/// * `program_id` must be the correct program id for this invocation.
#[inline(always)]
pub unsafe fn deserialize_accounts_fast<const MAX: usize>(
    input: *mut u8,
    accounts: &mut [MaybeUninit<AccountView>; MAX],
    instruction_data: &'static [u8],
    program_id: Address,
) -> (Address, usize, &'static [u8]) {
    let num_accounts = unsafe { *(input as *const u64) as usize };
    let count = num_accounts.min(MAX);
    let mut offset = 8usize;

    let mut slot = 0usize;
    while slot < count {
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            let raw = unsafe { input.add(offset) as *mut RuntimeAccount };
            accounts[slot] = MaybeUninit::new(unsafe { AccountView::new_unchecked(raw) });

            let data_len = unsafe { (*raw).data_len as usize };
            offset += RuntimeAccount::SIZE;
            offset += data_len + MAX_PERMITTED_DATA_INCREASE;
            offset += unsafe { input.add(offset).align_offset(BPF_ALIGN_OF_U128) };
            offset += 8;
        } else {
            let duplicate_of = marker as usize;
            // Identical well-formedness check as the scanning-variant above.
            if duplicate_of >= slot {
                malformed_duplicate_marker(marker, slot);
            }
            let raw = unsafe { accounts[duplicate_of].assume_init_ref().raw_ptr() };
            accounts[slot] = MaybeUninit::new(unsafe { AccountView::new_unchecked(raw) });
            offset += 8;
        }

        slot += 1;
    }

    // Skip remaining accounts. not needed, but slot tracking isn't required
    // since we don't need to find the instruction tail.

    (program_id, count, instruction_data)
}

/// Parse just the instruction tail and account span from the loader input.
///
/// This supports both eager entrypoint parsing and lazy account iteration.
/// The returned frame carries the original account span start so duplicate and
/// canonical-account relationships remain defined at the loader level.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
#[inline(always)]
pub unsafe fn scan_instruction_frame(input: *mut u8) -> RawInstructionFrame {
    let mut scan = input;

    let num_accounts = unsafe { *(scan as *const u64) as usize };
    scan = unsafe { scan.add(8) };
    let accounts_start = scan;

    let mut slot = 0usize;
    while slot < num_accounts {
        let marker = unsafe { *scan };
        if marker == u8::MAX {
            let raw = scan as *const RuntimeAccount;
            let data_len = unsafe { (*raw).data_len as usize };
            let mut step = RuntimeAccount::SIZE + data_len + MAX_PERMITTED_DATA_INCREASE;
            step += unsafe { scan.add(step).align_offset(BPF_ALIGN_OF_U128) };
            step += 8;
            scan = unsafe { scan.add(step) };
        } else {
            scan = unsafe { scan.add(8) };
        }
        slot += 1;
    }

    let data_len = unsafe { *(scan as *const u64) as usize };
    scan = unsafe { scan.add(8) };
    let instruction_data = unsafe { core::slice::from_raw_parts(scan as *const u8, data_len) };
    scan = unsafe { scan.add(data_len) };

    let program_id_ptr = scan as *const [u8; 32];
    let program_id = Address::new_from_array(unsafe { *program_id_ptr });

    RawInstructionFrame {
        accounts_start,
        account_count: num_accounts.min(254),
        instruction_data,
        program_id,
    }
}

// =====================================================================
// Safe bounds-checked loader-input parser (fuzz and off-chain harness).
// =====================================================================
//
// The primary parser above is a pure-pointer fast path: on-chain it
// consumes an SVM-loaded byte buffer whose layout is guaranteed by the
// loader. Off-chain tools (`hopper dump`, `hopper test`, fuzz harnesses,
// RPC decoders) do **not** have that guarantee. they receive arbitrary
// byte slices. Feeding one to `scan_instruction_frame` would invite OOB
// reads on any short / truncated input.
//
// `parse_instruction_frame_checked` is the safe companion: it walks a
// `&[u8]` using a bounds-checked cursor and returns structured
// `Result<FrameInfo, FrameError>`. It enforces exactly the same
// duplicate-marker well-formedness rules (forward references are
// rejected, not silently-aliased) and the same loader framing (88-byte
// `RuntimeAccount` header, `MAX_PERMITTED_DATA_INCREASE` reserve, u128
// alignment padding, `rent_epoch` tail, instruction_data with u64-LE
// length prefix, 32-byte program id trailer).

/// Hard cap on accounts the safe parser will record slot offsets for.
///
/// Matches Solana's own 256-account cap per instruction. Buffers that
/// declare more than this are rejected with
/// [`FrameError::AccountCountOutOfRange`].
pub const MAX_SAFE_ACCOUNT_SLOTS: usize = 256;

/// Summary of a safely-parsed loader input frame.
///
/// Only metadata is returned. the full `AccountView` construction
/// requires the raw pointer path. This struct is what off-chain tools
/// (and fuzz harnesses) need to verify a buffer is well-formed.
///
/// The `slot_offsets` array is a fixed `[usize; MAX_SAFE_ACCOUNT_SLOTS]`
/// with the first `account_count` entries populated. Remaining entries
/// are zero. Callers can distinguish duplicate vs canonical slots by
/// checking whether `buffer[offset]` equals `0xFF`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    /// Number of accounts the loader would hand to the program.
    pub account_count: usize,
    /// Byte range of the instruction data within the original buffer.
    pub instruction_data_range: core::ops::Range<usize>,
    /// Byte offset of the 32-byte program id within the original buffer.
    pub program_id_offset: usize,
    /// Byte offsets of each account slot, indexable 0..account_count.
    pub slot_offsets: [usize; MAX_SAFE_ACCOUNT_SLOTS],
}

/// Errors returned by the safe parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Buffer ended before the full frame could be parsed.
    UnexpectedEof { needed: usize, at: usize },
    /// Account count exceeds the compiled-in cap (256).
    AccountCountOutOfRange(u64),
    /// Duplicate marker refers to a non-earlier slot (forward ref or self).
    MalformedDuplicateMarker { slot: usize, marker: u8 },
    /// Data length field larger than the remaining buffer.
    DataLenOutOfRange { slot: usize, data_len: u64 },
    /// Arithmetic overflow while computing the next slot offset.
    OffsetOverflow { slot: usize },
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof { needed, at } => {
                write!(f, "unexpected EOF: need {needed} bytes at offset {at}")
            }
            Self::AccountCountOutOfRange(n) => {
                write!(f, "account count {n} exceeds cap 256")
            }
            Self::MalformedDuplicateMarker { slot, marker } => {
                write!(
                    f,
                    "malformed duplicate marker at slot {slot}: marker {marker} does not refer to an earlier slot"
                )
            }
            Self::DataLenOutOfRange { slot, data_len } => {
                write!(f, "slot {slot}: data_len {data_len} exceeds remaining buffer")
            }
            Self::OffsetOverflow { slot } => {
                write!(f, "slot {slot}: offset arithmetic overflow")
            }
        }
    }
}

/// Parse a loader-input byte buffer with full bounds checking.
///
/// This is the safe companion to `scan_instruction_frame` /
/// `deserialize_accounts`. It returns `Err` (never panics, never reads
/// out of bounds) for any malformed or truncated input, and preserves
/// the exact same forward-duplicate-marker rejection rule that the
/// pointer parser uses (see `malformed_duplicate_marker`).
///
/// Off-chain tools, fuzz harnesses, and RPC decoders should prefer
/// this function. On-chain entrypoints continue to use the pointer
/// parser for zero-overhead access.
pub fn parse_instruction_frame_checked(buf: &[u8]) -> Result<FrameInfo, FrameError> {
    // Helper: read a u64 LE at `pos`, bumping the cursor. Returns
    // `UnexpectedEof` if the 8 bytes aren't in range.
    fn read_u64_le(buf: &[u8], pos: &mut usize) -> Result<u64, FrameError> {
        let end = pos.checked_add(8).ok_or(FrameError::OffsetOverflow { slot: 0 })?;
        let slice = buf.get(*pos..end).ok_or(FrameError::UnexpectedEof {
            needed: 8,
            at: *pos,
        })?;
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(slice);
        *pos = end;
        Ok(u64::from_le_bytes(bytes))
    }

    fn read_u8(buf: &[u8], pos: &mut usize) -> Result<u8, FrameError> {
        let byte = *buf
            .get(*pos)
            .ok_or(FrameError::UnexpectedEof { needed: 1, at: *pos })?;
        *pos += 1;
        Ok(byte)
    }

    fn advance(buf: &[u8], pos: &mut usize, n: usize) -> Result<(), FrameError> {
        let end = pos.checked_add(n).ok_or(FrameError::OffsetOverflow { slot: 0 })?;
        if end > buf.len() {
            return Err(FrameError::UnexpectedEof { needed: n, at: *pos });
        }
        *pos = end;
        Ok(())
    }

    let mut pos = 0usize;
    let account_count = read_u64_le(buf, &mut pos)?;
    if account_count > MAX_SAFE_ACCOUNT_SLOTS as u64 {
        return Err(FrameError::AccountCountOutOfRange(account_count));
    }
    let account_count = account_count as usize;

    let mut slot_offsets = [0usize; MAX_SAFE_ACCOUNT_SLOTS];

    for slot in 0..account_count {
        let slot_start = pos;
        slot_offsets[slot] = slot_start;

        let marker = read_u8(buf, &mut pos)?;
        if marker == u8::MAX {
            // Canonical account: the remaining 87 bytes of RuntimeAccount
            // follow (we already consumed the marker byte).
            advance(buf, &mut pos, RuntimeAccount::SIZE - 1)
                .map_err(|_| FrameError::UnexpectedEof { needed: RuntimeAccount::SIZE - 1, at: pos })?;
            // data_len lives at offset 80 in RuntimeAccount; we read it
            // directly from the slot header. Offset within this slot:
            // borrow_state(1) + flags(3) + resize_delta(4) + address(32) +
            // owner(32) + lamports(8) = 80 -> data_len(8).
            let data_len_pos = slot_start
                .checked_add(80)
                .ok_or(FrameError::OffsetOverflow { slot })?;
            let mut dl_bytes = [0u8; 8];
            let dl_slice = buf
                .get(data_len_pos..data_len_pos + 8)
                .ok_or(FrameError::UnexpectedEof { needed: 8, at: data_len_pos })?;
            dl_bytes.copy_from_slice(dl_slice);
            let data_len = u64::from_le_bytes(dl_bytes);

            // data_bytes + realloc reserve + u128 alignment padding + rent_epoch
            let data_sz: usize = (data_len as usize)
                .checked_add(MAX_PERMITTED_DATA_INCREASE)
                .ok_or(FrameError::DataLenOutOfRange { slot, data_len })?;
            advance(buf, &mut pos, data_sz)
                .map_err(|_| FrameError::DataLenOutOfRange { slot, data_len })?;
            let pad = pos.wrapping_neg() & (BPF_ALIGN_OF_U128 - 1);
            advance(buf, &mut pos, pad)
                .map_err(|_| FrameError::UnexpectedEof { needed: pad, at: pos })?;
            advance(buf, &mut pos, 8)
                .map_err(|_| FrameError::UnexpectedEof { needed: 8, at: pos })?;
        } else {
            // Duplicate marker: must refer to a strictly earlier slot.
            // This is the Hopper Safety Audit Must-Fix #1 invariant.
            let duplicate_of = marker as usize;
            if duplicate_of >= slot {
                return Err(FrameError::MalformedDuplicateMarker { slot, marker });
            }
            // 7 padding bytes follow the marker.
            advance(buf, &mut pos, 7)
                .map_err(|_| FrameError::UnexpectedEof { needed: 7, at: pos })?;
        }
    }

    // Instruction data: u64 LE length prefix + bytes.
    let ix_data_len = read_u64_le(buf, &mut pos)? as usize;
    let ix_start = pos;
    advance(buf, &mut pos, ix_data_len)
        .map_err(|_| FrameError::UnexpectedEof { needed: ix_data_len, at: pos })?;
    let instruction_data_range = ix_start..pos;

    // 32-byte program id trailer.
    let program_id_offset = pos;
    advance(buf, &mut pos, 32)
        .map_err(|_| FrameError::UnexpectedEof { needed: 32, at: pos })?;

    Ok(FrameInfo {
        account_count,
        instruction_data_range,
        program_id_offset,
        slot_offsets,
    })
}

#[cfg(test)]
mod checked_parser_tests {
    use super::*;

    /// Size of the single-account canonical frame used by tests.
    /// 8 (account_count) + 88 (RuntimeAccount) + 10240 (realloc reserve)
    /// + 0 (already u128-aligned at 10336) + 8 (rent_epoch)
    /// + 8 (ix_data_len) + 32 (program_id) = 10384
    const MINIMAL_FRAME_LEN: usize = 8 + 88 + MAX_PERMITTED_DATA_INCREASE + 0 + 8 + 8 + 32;

    /// Build a valid one-canonical-account frame with zero-byte data.
    fn build_minimal_frame() -> [u8; MINIMAL_FRAME_LEN] {
        let mut buf = [0u8; MINIMAL_FRAME_LEN];
        buf[0..8].copy_from_slice(&1u64.to_le_bytes()); // account_count = 1
        buf[8] = 0xFF; // marker = canonical
        // remaining bytes of RuntimeAccount stay zero
        // realloc reserve stays zero
        // rent_epoch zero
        // ix_data_len = 0 (already zero)
        // program_id stays zero
        buf
    }

    #[test]
    fn parses_minimal_valid_frame() {
        let buf = build_minimal_frame();
        let frame = parse_instruction_frame_checked(&buf).expect("well-formed");
        assert_eq!(frame.account_count, 1);
        assert_eq!(frame.instruction_data_range.len(), 0);
        assert_eq!(frame.program_id_offset + 32, buf.len());
    }

    #[test]
    fn truncated_header_is_rejected() {
        let buf = [0u8; 4]; // less than 8 bytes = no room for account_count
        let err = parse_instruction_frame_checked(&buf).unwrap_err();
        assert!(matches!(err, FrameError::UnexpectedEof { .. }));
    }

    #[test]
    fn oversized_account_count_is_rejected() {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&1_000u64.to_le_bytes());
        let err = parse_instruction_frame_checked(&buf).unwrap_err();
        assert!(matches!(err, FrameError::AccountCountOutOfRange(1000)));
    }

    #[test]
    fn forward_duplicate_marker_is_rejected() {
        // 2-account frame where slot 0 is a duplicate of slot 1
        // (forward reference). Must be rejected.
        let mut buf = [0u8; 16];
        buf[0..8].copy_from_slice(&2u64.to_le_bytes());
        buf[8] = 1; // slot 0 marker = 1 (forward ref)
        let err = parse_instruction_frame_checked(&buf).unwrap_err();
        assert!(matches!(
            err,
            FrameError::MalformedDuplicateMarker { slot: 0, marker: 1 }
        ));
    }

    #[test]
    fn self_duplicate_marker_is_rejected() {
        // Slot 0 marker=0 is self-reference: forbidden.
        let mut buf = [0u8; 16];
        buf[0..8].copy_from_slice(&1u64.to_le_bytes());
        buf[8] = 0; // marker = 0, referring to slot 0 itself
        let err = parse_instruction_frame_checked(&buf).unwrap_err();
        assert!(matches!(
            err,
            FrameError::MalformedDuplicateMarker { slot: 0, marker: 0 }
        ));
    }

    #[test]
    fn arbitrary_short_input_never_panics() {
        // Bounds-checking contract: feeding every length from 0..=256
        // bytes of zeroes must never panic or UB.
        let buf = [0u8; 256];
        for len in 0..=256 {
            let _ = parse_instruction_frame_checked(&buf[..len]);
        }
    }

    #[test]
    fn arbitrary_ff_input_never_panics() {
        let buf = [0xFFu8; 256];
        for len in 0..=256 {
            let _ = parse_instruction_frame_checked(&buf[..len]);
        }
    }
}
