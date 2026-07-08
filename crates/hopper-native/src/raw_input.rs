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
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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

/// Advance a record-start offset past one canonical account record.
///
/// Folds the entire per-account stride — 88-byte `RuntimeAccount` header,
/// `data_len` bytes of account data, the `MAX_PERMITTED_DATA_INCREASE`
/// realloc reserve, the u128 alignment padding, and the 8-byte rent-epoch
/// tail — into one integer expression: adds plus one `and`-mask. This is
/// the Pinocchio-shape stride and compiles to straight-line ALU ops,
/// unlike `<*mut u8>::align_offset`, which the compiler cannot fold when
/// the pointer's base alignment is opaque (~6 extra instructions per
/// account).
///
/// Correctness of aligning the *relative* offset instead of the absolute
/// address: the SVM loader serializes the input region at
/// `MM_INPUT_START` (`0x4_0000_0000`; agave's `solana-sbpf`
/// `ebpf::MM_INPUT_START`), so the buffer base is 8-aligned
/// (`BPF_ALIGN_OF_U128`) and `offset % 8 == (base + offset) % 8` — the
/// two formulations land on the same byte for every `data_len`. Because
/// `RuntimeAccount::SIZE` (88), `MAX_PERMITTED_DATA_INCREASE` (10240),
/// the rent-epoch tail (8), and the duplicate stride (8) are all
/// multiples of 8, every record starts at an 8-aligned offset and only
/// `data_len` contributes misalignment. Folding the trailing rent-epoch
/// `+ 8` inside the round-up is exact since `8 ≡ 0 (mod 8)`:
/// `((x + 8) + 7) & !7 == (((x + 7) & !7) + 8)`.
#[inline(always)]
const fn next_record_offset(offset: usize, data_len: usize) -> usize {
    (offset
        + RuntimeAccount::SIZE
        + data_len
        + MAX_PERMITTED_DATA_INCREASE
        + 8
        + (BPF_ALIGN_OF_U128 - 1))
        & !(BPF_ALIGN_OF_U128 - 1)
}

/// Deserialize the loader input into `AccountView`s.
///
/// Duplicate-account resolution happens here. A duplicate slot reuses the
/// canonical `RuntimeAccount` pointer of the earlier slot it references, and
/// its `original_index` remains the loader slot where it appeared.
///
/// This is a single fused walk over the account region: one loop both
/// materializes `AccountView`s (up to `MAX`) and carries the cursor to the
/// end of the region, where the instruction data and program id live.
/// Accounts beyond `MAX` are skip-only — advanced past without being
/// materialized — so the instruction tail is still found. The pre-fusion
/// shape walked the region twice (`scan_instruction_frame` to locate the
/// tail, then a second offset-based materialize loop), costing ~30
/// instructions per account; the fused walk is ~8.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
#[inline(always)]
pub unsafe fn deserialize_accounts<'info, const MAX: usize>(
    input: *mut u8,
    accounts: &mut [MaybeUninit<AccountView<'info>>; MAX],
) -> (Address, usize, &'info [u8]) {
    // SAFETY: `input` points to the head of the Solana BPF input buffer,
    // whose first 8 bytes are the account count. `read_unaligned` reads the
    // u64 without assuming 8-byte pointer alignment.
    let num_accounts = unsafe { core::ptr::read_unaligned(input as *const u64) as usize };
    // Duplicate markers are a single byte with 0xFF reserved for canonical
    // records, so marker values 0x00..=0xFE can address 255 slots (indices
    // 0..=254). We clamp materialization at 254 — one below that encoding
    // limit — purely to preserve the pre-fusion behavior
    // (`scan_instruction_frame` capped `account_count` at 254); slot 254,
    // though addressable by marker 0xFE, is handled skip-only in the tail.
    // Then clamp to the caller's capacity MAX.
    let addressable = if num_accounts > 254 {
        254
    } else {
        num_accounts
    };
    let count = if addressable > MAX { MAX } else { addressable };

    let mut offset = 8usize;

    // Fused walk, hot loop: materialize AND advance in one pass.
    let mut slot = 0usize;
    while slot < count {
        // SAFETY: `slot < count <= num_accounts`, so `offset` sits on a
        // loader-produced record boundary and the marker byte is in bounds.
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            // SAFETY: a 0xFF marker means a canonical `RuntimeAccount`
            // record starts at this record boundary; the loader guarantees
            // the full 88-byte header (plus data) follows in bounds.
            let raw = unsafe { input.add(offset) as *mut RuntimeAccount };
            // SAFETY: `slot < count <= MAX`, and `raw` points at a valid
            // canonical account record in the loader input.
            unsafe {
                *accounts.get_unchecked_mut(slot) =
                    MaybeUninit::new(AccountView::new_unchecked(raw))
            };

            // SAFETY: `raw` points to the RuntimeAccount header just decoded
            // from the current input slot; `data_len` is 8-aligned within it
            // because record starts are 8-aligned (see `next_record_offset`).
            let data_len = unsafe { (*raw).data_len as usize };
            // Pinocchio-shape stride: pure integer adds + mask. Byte-for-byte
            // identical to the old absolute-address `align_offset` math
            // because the loader input base is 8-aligned (MM_INPUT_START;
            // see `next_record_offset` docs).
            offset = next_record_offset(offset, data_len);
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
            // SAFETY: `duplicate_of < slot < count`, so the referenced slot
            // was initialized by an earlier iteration of this loop.
            let raw = unsafe {
                accounts
                    .get_unchecked(duplicate_of)
                    .assume_init_ref()
                    .raw_ptr()
            };
            // SAFETY: `slot < count <= MAX`, and `raw` came from a validated
            // earlier slot in this same frame.
            unsafe {
                *accounts.get_unchecked_mut(slot) =
                    MaybeUninit::new(AccountView::new_unchecked(raw))
            };
            // Duplicate slots occupy 8 bytes: marker byte + 7 padding bytes.
            offset += 8;
        }

        slot += 1;
    }

    // Skip-only tail: accounts beyond MAX (or beyond the 254 addressable
    // slots) are not materialized, but the cursor must still advance past
    // their records so the instruction data and program id can be located.
    // Duplicate-marker well-formedness is still enforced here, exactly as
    // the pre-fusion scan pass did for every slot.
    while slot < num_accounts {
        // SAFETY: `slot < num_accounts`, so `offset` sits on a
        // loader-produced record boundary within the input buffer.
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            // SAFETY: canonical record at a loader-produced record boundary;
            // its `data_len` header field is in bounds and 8-aligned.
            let data_len =
                unsafe { (*(input.add(offset) as *const RuntimeAccount)).data_len } as usize;
            offset = next_record_offset(offset, data_len);
        } else {
            let duplicate_of = marker as usize;
            if duplicate_of >= slot {
                malformed_duplicate_marker(marker, slot);
            }
            offset += 8;
        }
        slot += 1;
    }

    // Instruction tail: u64 LE length prefix, data bytes, 32-byte program id.
    // SAFETY: the walk above advanced `offset` past all `num_accounts`
    // records, so it now points at the 8-byte instruction-data length in the
    // loader input buffer. `read_unaligned` avoids assuming pointer alignment
    // (the offset is in fact 8-aligned here, but the read is free either way).
    let ix_data_len =
        unsafe { core::ptr::read_unaligned(input.add(offset) as *const u64) as usize };
    offset += 8;
    // SAFETY: the loader serializes `ix_data_len` instruction-data bytes
    // immediately after the length prefix; the buffer lives for the whole
    // invocation, matching the returned lifetime.
    let instruction_data =
        unsafe { core::slice::from_raw_parts(input.add(offset) as *const u8, ix_data_len) };
    offset += ix_data_len;
    // SAFETY: the 32-byte program id trails the instruction data per the
    // loader serialization layout; `[u8; 32]` has alignment 1, so the read
    // by value is valid at any offset.
    let program_id = Address::new_from_array(unsafe { *(input.add(offset) as *const [u8; 32]) });

    (program_id, count, instruction_data)
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
pub unsafe fn deserialize_accounts_fast<'info, const MAX: usize>(
    input: *mut u8,
    accounts: &mut [MaybeUninit<AccountView<'info>>; MAX],
    instruction_data: &'info [u8],
    program_id: Address,
) -> (Address, usize, &'info [u8]) {
    // SAFETY: `input` points to the head of the Solana BPF input buffer, whose
    // first 8 bytes are the account count. `read_unaligned` reads the u64 without
    // assuming 8-byte pointer alignment, so this stays sound even if the loader
    // ever hands us an unaligned buffer.
    let num_accounts = unsafe { core::ptr::read_unaligned(input as *const u64) as usize };
    let count = num_accounts.min(MAX);
    let mut offset = 8usize;

    let mut slot = 0usize;
    while slot < count {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            // SAFETY: `offset` is on a Solana account record boundary produced
            // by the loader input format.
            let raw = unsafe { input.add(offset) as *mut RuntimeAccount };
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            unsafe {
                *accounts.get_unchecked_mut(slot) =
                    MaybeUninit::new(AccountView::new_unchecked(raw))
            };

            // SAFETY: `raw` points to the RuntimeAccount header just decoded
            // from the current input slot.
            let data_len = unsafe { (*raw).data_len as usize };
            // Pinocchio-shape stride: pure integer adds + mask, identical to
            // the old absolute-address `align_offset` math because the loader
            // input base is 8-aligned (see `next_record_offset` docs).
            offset = next_record_offset(offset, data_len);
        } else {
            let duplicate_of = marker as usize;
            // Identical well-formedness check as the scanning-variant above.
            if duplicate_of >= slot {
                malformed_duplicate_marker(marker, slot);
            }
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            let raw = unsafe {
                accounts
                    .get_unchecked(duplicate_of)
                    .assume_init_ref()
                    .raw_ptr()
            };
            // SAFETY: `slot < count <= MAX`, and `raw` came from a validated
            // earlier slot in this same frame.
            unsafe {
                *accounts.get_unchecked_mut(slot) =
                    MaybeUninit::new(AccountView::new_unchecked(raw))
            };
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

    // SAFETY: `scan` starts at the head of the Solana BPF input buffer, whose
    // first 8 bytes are the account count. `read_unaligned` avoids assuming the
    // pointer is 8-byte aligned.
    let num_accounts = unsafe { core::ptr::read_unaligned(scan as *const u64) as usize };
    // SAFETY: advancing past the 8-byte account-count prefix keeps `scan`
    // within the loader input buffer, at the first account record boundary.
    scan = unsafe { scan.add(8) };
    let accounts_start = scan;

    let mut slot = 0usize;
    while slot < num_accounts {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let marker = unsafe { *scan };
        if marker == u8::MAX {
            let raw = scan as *const RuntimeAccount;
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            let data_len = unsafe { (*raw).data_len as usize };
            let mut step = RuntimeAccount::SIZE + data_len + MAX_PERMITTED_DATA_INCREASE;
            step += unsafe { scan.add(step).align_offset(BPF_ALIGN_OF_U128) };
            step += 8;
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            scan = unsafe { scan.add(step) };
        } else {
            let duplicate_of = marker as usize;
            if duplicate_of >= slot {
                malformed_duplicate_marker(marker, slot);
            }
            // SAFETY: Duplicate-account entries are 8-byte slots in the
            // Solana input frame format; scanner bounds are driven by
            // `num_accounts` and validated traversal above.
            scan = unsafe { scan.add(8) };
        }
        slot += 1;
    }

    // SAFETY: `scan` now points at the 8-byte instruction-data length in the
    // Solana BPF input buffer. `read_unaligned` avoids assuming 8-byte pointer
    // alignment of `scan`.
    let data_len = unsafe { core::ptr::read_unaligned(scan as *const u64) as usize };
    scan = unsafe { scan.add(8) };
    let instruction_data = unsafe { core::slice::from_raw_parts(scan as *const u8, data_len) };
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    scan = unsafe { scan.add(data_len) };

    let program_id_ptr = scan as *const [u8; 32];
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
                write!(
                    f,
                    "slot {slot}: data_len {data_len} exceeds remaining buffer"
                )
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
        let end = pos
            .checked_add(8)
            .ok_or(FrameError::OffsetOverflow { slot: 0 })?;
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
        let byte = *buf.get(*pos).ok_or(FrameError::UnexpectedEof {
            needed: 1,
            at: *pos,
        })?;
        *pos += 1;
        Ok(byte)
    }

    fn advance(buf: &[u8], pos: &mut usize, n: usize) -> Result<(), FrameError> {
        let end = pos
            .checked_add(n)
            .ok_or(FrameError::OffsetOverflow { slot: 0 })?;
        if end > buf.len() {
            return Err(FrameError::UnexpectedEof {
                needed: n,
                at: *pos,
            });
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

    // The slot index is load-bearing: it backs the duplicate-marker invariant
    // (`duplicate_of >= slot`) and every `FrameError { slot, .. }` report, so
    // an iterator over values would lose the information the loop exists for.
    #[allow(clippy::needless_range_loop)]
    for slot in 0..account_count {
        let slot_start = pos;
        slot_offsets[slot] = slot_start;

        let marker = read_u8(buf, &mut pos)?;
        if marker == u8::MAX {
            // Canonical account: the remaining 87 bytes of RuntimeAccount
            // follow (we already consumed the marker byte).
            advance(buf, &mut pos, RuntimeAccount::SIZE - 1).map_err(|_| {
                FrameError::UnexpectedEof {
                    needed: RuntimeAccount::SIZE - 1,
                    at: pos,
                }
            })?;
            // data_len lives at offset 80 in RuntimeAccount; we read it
            // directly from the slot header. Offset within this slot:
            // borrow_state(1) + flags(3) + resize_delta(4) + address(32) +
            // owner(32) + lamports(8) = 80 -> data_len(8).
            let data_len_pos = slot_start
                .checked_add(80)
                .ok_or(FrameError::OffsetOverflow { slot })?;
            let mut dl_bytes = [0u8; 8];
            let dl_slice =
                buf.get(data_len_pos..data_len_pos + 8)
                    .ok_or(FrameError::UnexpectedEof {
                        needed: 8,
                        at: data_len_pos,
                    })?;
            dl_bytes.copy_from_slice(dl_slice);
            let data_len = u64::from_le_bytes(dl_bytes);

            // data_bytes + realloc reserve + u128 alignment padding + rent_epoch
            let data_sz: usize = (data_len as usize)
                .checked_add(MAX_PERMITTED_DATA_INCREASE)
                .ok_or(FrameError::DataLenOutOfRange { slot, data_len })?;
            advance(buf, &mut pos, data_sz)
                .map_err(|_| FrameError::DataLenOutOfRange { slot, data_len })?;
            let pad = pos.wrapping_neg() & (BPF_ALIGN_OF_U128 - 1);
            advance(buf, &mut pos, pad).map_err(|_| FrameError::UnexpectedEof {
                needed: pad,
                at: pos,
            })?;
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
    advance(buf, &mut pos, ix_data_len).map_err(|_| FrameError::UnexpectedEof {
        needed: ix_data_len,
        at: pos,
    })?;
    let instruction_data_range = ix_start..pos;

    // 32-byte program id trailer.
    let program_id_offset = pos;
    advance(buf, &mut pos, 32).map_err(|_| FrameError::UnexpectedEof {
        needed: 32,
        at: pos,
    })?;

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
    const MINIMAL_FRAME_LEN: usize = 8 + 88 + MAX_PERMITTED_DATA_INCREASE + 8 + 8 + 32;

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

#[cfg(test)]
mod fused_walk_tests {
    extern crate std;

    use std::vec;
    use std::vec::Vec;

    use super::*;

    /// One account slot description for the frame builder.
    enum Slot {
        /// Canonical account: 0xFF marker, header, `data` bytes, realloc
        /// reserve, alignment padding, rent epoch.
        Fresh { data: Vec<u8>, lamports: u64 },
        /// Duplicate reference: 1 marker byte + 7 padding bytes.
        Dup(u8),
    }

    fn fresh(data_len: usize, lamports: u64) -> Slot {
        Slot::Fresh {
            data: vec![0xABu8; data_len],
            lamports,
        }
    }

    /// 8-aligned loader-input fixture. The `u64` backing guarantees the
    /// base pointer is 8-aligned, matching the loader's `MM_INPUT_START`
    /// guarantee that the fused stride math relies on.
    struct Frame {
        words: Vec<u64>,
    }

    impl Frame {
        fn as_mut_ptr(&mut self) -> *mut u8 {
            self.words.as_mut_ptr() as *mut u8
        }
    }

    /// Serialize a loader input frame exactly per the Solana BPF loader
    /// layout: u64 account count; per canonical account an 88-byte
    /// `RuntimeAccount` header (marker byte 0xFF first), `data_len` data
    /// bytes, `MAX_PERMITTED_DATA_INCREASE` reserve, padding to the next
    /// 8-byte boundary, and an 8-byte rent epoch; per duplicate 8 bytes
    /// (marker + 7 padding); then u64 ix-data length, ix-data bytes, and
    /// the 32-byte program id.
    fn build_frame(slots: &[Slot], ix_data: &[u8], program_id: [u8; 32]) -> Frame {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&(slots.len() as u64).to_le_bytes());

        for (i, slot) in slots.iter().enumerate() {
            match slot {
                Slot::Fresh { data, lamports } => {
                    let mut header = [0u8; RuntimeAccount::SIZE];
                    header[0] = 0xFF; // canonical marker / borrow_state
                    header[1] = 1; // is_signer
                    header[2] = 1; // is_writable
                                   // address: recognizable per-slot pattern
                    header[8..40].copy_from_slice(&[i as u8 + 1; 32]);
                    // owner
                    header[40..72].copy_from_slice(&[0x55; 32]);
                    // lamports at offset 72
                    header[72..80].copy_from_slice(&lamports.to_le_bytes());
                    // data_len at offset 80
                    header[80..88].copy_from_slice(&(data.len() as u64).to_le_bytes());
                    buf.extend_from_slice(&header);
                    buf.extend_from_slice(data);
                    buf.extend_from_slice(&vec![0u8; MAX_PERMITTED_DATA_INCREASE]);
                    // Pad to the next 8-byte boundary. The base is 8-aligned,
                    // so padding the relative length equals padding the
                    // absolute address — this is the loader's ground truth.
                    while buf.len() % BPF_ALIGN_OF_U128 != 0 {
                        buf.push(0);
                    }
                    // rent epoch
                    buf.extend_from_slice(&u64::MAX.to_le_bytes());
                }
                Slot::Dup(of) => {
                    buf.push(*of);
                    buf.extend_from_slice(&[0u8; 7]);
                }
            }
        }

        buf.extend_from_slice(&(ix_data.len() as u64).to_le_bytes());
        buf.extend_from_slice(ix_data);
        buf.extend_from_slice(&program_id);

        // Copy into 8-aligned u64 backing.
        let mut words = vec![0u64; buf.len().div_ceil(8)];
        // SAFETY: `words` has at least `buf.len()` bytes of capacity and the
        // regions do not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), words.as_mut_ptr() as *mut u8, buf.len());
        }
        Frame { words }
    }

    fn uninit_views<'a, const MAX: usize>() -> [MaybeUninit<AccountView<'a>>; MAX] {
        // SAFETY: an array of `MaybeUninit` is valid in the uninitialized
        // state by definition.
        unsafe { MaybeUninit::uninit().assume_init() }
    }

    const PID: [u8; 32] = [0xC4; 32];

    #[test]
    fn zero_accounts_finds_ix_data_and_program_id() {
        let mut frame = build_frame(&[], &[9, 8, 7], PID);
        let mut views = uninit_views::<4>();
        // SAFETY: `frame` is a well-formed loader-layout buffer with an
        // 8-aligned base.
        let (pid, count, ix) = unsafe { deserialize_accounts::<4>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 0);
        assert_eq!(ix, &[9, 8, 7]);
        assert_eq!(pid.as_array(), &PID);
    }

    #[test]
    fn one_account_materializes_and_finds_tail() {
        let mut frame = build_frame(&[fresh(11, 42)], &[1, 2, 3, 4], PID);
        let mut views = uninit_views::<4>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (pid, count, ix) = unsafe { deserialize_accounts::<4>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 1);
        // SAFETY: slot 0 was initialized by the parser (count == 1).
        let view = unsafe { views[0].assume_init_ref() };
        assert_eq!(view.data_len(), 11);
        assert_eq!(view.lamports(), 42);
        assert!(view.is_signer());
        assert_eq!(ix, &[1, 2, 3, 4]);
        assert_eq!(pid.as_array(), &PID);
    }

    #[test]
    fn exactly_max_accounts() {
        let slots: Vec<Slot> = (0..4).map(|i| fresh(i * 3 + 1, 100 + i as u64)).collect();
        let mut frame = build_frame(&slots, &[0xEE; 5], PID);
        let mut views = uninit_views::<4>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (pid, count, ix) = unsafe { deserialize_accounts::<4>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 4);
        for i in 0..4 {
            // SAFETY: slots 0..count were initialized by the parser.
            let view = unsafe { views[i].assume_init_ref() };
            assert_eq!(view.data_len(), i * 3 + 1);
            assert_eq!(view.lamports(), 100 + i as u64);
        }
        assert_eq!(ix, &[0xEE; 5]);
        assert_eq!(pid.as_array(), &PID);
    }

    #[test]
    fn beyond_max_is_skip_only_and_tail_still_found() {
        // MAX = 4, 7 accounts (MAX + 3). The tail accounts get assorted
        // data_len residues so the skip-only stride is exercised too.
        let slots: Vec<Slot> = (0..7).map(|i| fresh(i * 5 + 2, i as u64)).collect();
        let mut frame = build_frame(&slots, &[0xD1, 0xD2], PID);
        let mut views = uninit_views::<4>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (pid, count, ix) = unsafe { deserialize_accounts::<4>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 4);
        for i in 0..4 {
            // SAFETY: slots 0..count were initialized by the parser.
            let view = unsafe { views[i].assume_init_ref() };
            assert_eq!(view.data_len(), i * 5 + 2);
        }
        assert_eq!(ix, &[0xD1, 0xD2]);
        assert_eq!(pid.as_array(), &PID);
    }

    #[test]
    fn duplicates_alias_the_canonical_record() {
        let slots = [fresh(9, 7), Slot::Dup(0), fresh(3, 8), Slot::Dup(2)];
        let mut frame = build_frame(&slots, &[0x11], PID);
        let mut views = uninit_views::<8>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (_, count, ix) = unsafe { deserialize_accounts::<8>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 4);
        // SAFETY: slots 0..count were initialized by the parser.
        let (v0, v1, v2, v3) = unsafe {
            (
                views[0].assume_init_ref(),
                views[1].assume_init_ref(),
                views[2].assume_init_ref(),
                views[3].assume_init_ref(),
            )
        };
        assert_eq!(v0.raw_ptr(), v1.raw_ptr(), "dup slot aliases canonical");
        assert_eq!(v2.raw_ptr(), v3.raw_ptr(), "dup slot aliases canonical");
        assert_ne!(v0.raw_ptr(), v2.raw_ptr());
        assert_eq!(v1.data_len(), 9);
        assert_eq!(v3.data_len(), 3);
        assert_eq!(ix, &[0x11]);
    }

    #[test]
    fn duplicate_in_skip_only_tail_advances_eight_bytes() {
        // MAX = 2; slots 2 and 3 (a fresh account and a duplicate) are
        // skip-only. If the duplicate stride were wrong, the ix data would
        // be misread.
        let slots = [fresh(5, 1), fresh(6, 2), fresh(7, 3), Slot::Dup(1)];
        let mut frame = build_frame(&slots, &[0xAA, 0xBB, 0xCC], PID);
        let mut views = uninit_views::<2>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (pid, count, ix) = unsafe { deserialize_accounts::<2>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 2);
        assert_eq!(ix, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(pid.as_array(), &PID);
    }

    #[test]
    fn every_data_len_alignment_residue_walks_correctly() {
        // data_len 0..=7 covers every alignment residue; 8..=15 repeats them
        // one stride later. All must land the cursor exactly on the ix tail.
        for base in [0usize, 8] {
            let slots: Vec<Slot> = (0..8).map(|r| fresh(base + r, r as u64)).collect();
            let mut frame = build_frame(&slots, &[0x42; 9], PID);
            let mut views = uninit_views::<8>();
            // SAFETY: well-formed 8-aligned loader-layout fixture.
            let (pid, count, ix) =
                unsafe { deserialize_accounts::<8>(frame.as_mut_ptr(), &mut views) };
            assert_eq!(count, 8);
            for r in 0..8 {
                // SAFETY: slots 0..count were initialized by the parser.
                let view = unsafe { views[r].assume_init_ref() };
                assert_eq!(view.data_len(), base + r);
            }
            assert_eq!(ix, &[0x42; 9]);
            assert_eq!(pid.as_array(), &PID);
        }
    }

    /// Differential test: the folded integer stride must match the old
    /// pointer `align_offset` formula byte-for-byte for every data_len,
    /// given an 8-aligned base (the loader guarantee).
    #[test]
    fn folded_stride_matches_align_offset_formula() {
        // Real 8-aligned base pointer; align_offset is pure address
        // arithmetic, so wrapping_add beyond the allocation is fine.
        let backing = [0u64; 1];
        let base = backing.as_ptr() as *const u8;
        assert_eq!(base as usize % 8, 0, "test base must be 8-aligned");

        for start in [8usize, 96, 10344, 20696] {
            for data_len in 0usize..64 {
                // Old formula (pre-fusion deserialize_accounts body):
                let mut old = start;
                old += RuntimeAccount::SIZE;
                old += data_len + MAX_PERMITTED_DATA_INCREASE;
                old += base.wrapping_add(old).align_offset(BPF_ALIGN_OF_U128);
                old += 8;
                // New folded formula:
                let new = next_record_offset(start, data_len);
                assert_eq!(
                    old, new,
                    "stride mismatch at start={start} data_len={data_len}"
                );
            }
        }
    }

    #[test]
    fn huge_data_len_near_region_end() {
        // A single account whose data dwarfs the rest of the frame; the
        // ix tail sits immediately after its (padded) record.
        let big = 100_003usize; // residue 3 to force nonzero padding
        let mut frame = build_frame(&[fresh(big, 5)], &[0x77, 0x66], PID);
        let mut views = uninit_views::<2>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (pid, count, ix) = unsafe { deserialize_accounts::<2>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 1);
        // SAFETY: slot 0 was initialized by the parser.
        assert_eq!(unsafe { views[0].assume_init_ref() }.data_len(), big);
        assert_eq!(ix, &[0x77, 0x66]);
        assert_eq!(pid.as_array(), &PID);
    }

    #[test]
    fn account_count_clamps_at_254_materialized_slots() {
        // 1 canonical + 259 duplicates = 260 declared accounts. Slots
        // 254..259 must be skip-only even though MAX = 255, mirroring the
        // pre-fusion `min(254)` clamp; the walk must still reach the tail.
        let mut slots: Vec<Slot> = vec![fresh(4, 9)];
        slots.extend((0..259).map(|_| Slot::Dup(0)));
        let mut frame = build_frame(&slots, &[0x0F; 3], PID);
        let mut views = uninit_views::<255>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (pid, count, ix) =
            unsafe { deserialize_accounts::<255>(frame.as_mut_ptr(), &mut views) };
        assert_eq!(count, 254);
        assert_eq!(ix, &[0x0F; 3]);
        assert_eq!(pid.as_array(), &PID);
    }

    #[test]
    #[should_panic(expected = "malformed duplicate marker")]
    fn forward_duplicate_marker_traps_in_materialize_range() {
        let slots = [fresh(1, 1), Slot::Dup(1)]; // self-reference at slot 1
        let mut frame = build_frame(&slots, &[], PID);
        let mut views = uninit_views::<4>();
        // SAFETY: buffer layout is loader-shaped; the malformed marker is
        // the condition under test and traps before any OOB access.
        let _ = unsafe { deserialize_accounts::<4>(frame.as_mut_ptr(), &mut views) };
    }

    #[test]
    #[should_panic(expected = "malformed duplicate marker")]
    fn forward_duplicate_marker_traps_in_skip_only_tail() {
        // MAX = 1, so slot 1 is skip-only — the trap must still fire there.
        let slots = [fresh(1, 1), Slot::Dup(5)];
        let mut frame = build_frame(&slots, &[], PID);
        let mut views = uninit_views::<1>();
        // SAFETY: buffer layout is loader-shaped; the malformed marker is
        // the condition under test and traps before any OOB access.
        let _ = unsafe { deserialize_accounts::<1>(frame.as_mut_ptr(), &mut views) };
    }

    #[test]
    fn fast_variant_uses_same_stride_and_aliases_duplicates() {
        // `deserialize_accounts_fast` shares `next_record_offset`; verify it
        // still parses mixed-residue accounts and duplicates correctly when
        // ix data and program id are supplied out of band.
        let slots = [fresh(13, 3), Slot::Dup(0), fresh(6, 4)];
        let mut frame = build_frame(&slots, &[0x99], PID);
        let mut views = uninit_views::<4>();
        let ix: &[u8] = &[0x99];
        // SAFETY: well-formed 8-aligned loader-layout fixture; ix data and
        // program id are supplied directly per the fast-path contract.
        let (pid, count, out_ix) = unsafe {
            deserialize_accounts_fast::<4>(
                frame.as_mut_ptr(),
                &mut views,
                ix,
                Address::new_from_array(PID),
            )
        };
        assert_eq!(count, 3);
        // SAFETY: slots 0..count were initialized by the parser.
        let (v0, v1, v2) = unsafe {
            (
                views[0].assume_init_ref(),
                views[1].assume_init_ref(),
                views[2].assume_init_ref(),
            )
        };
        assert_eq!(v0.raw_ptr(), v1.raw_ptr());
        assert_eq!(v0.data_len(), 13);
        assert_eq!(v2.data_len(), 6);
        assert_eq!(out_ix, ix);
        assert_eq!(pid.as_array(), &PID);
    }

    /// The fused walk and the safe checked parser must agree on where the
    /// instruction tail lives for the same buffer.
    #[test]
    fn fused_walk_agrees_with_checked_parser() {
        let slots = [fresh(7, 1), Slot::Dup(0), fresh(0, 2), fresh(33, 3)];
        let ix_data = [5u8, 4, 3, 2, 1];
        let mut frame = build_frame(&slots, &ix_data, PID);

        let byte_len = frame.words.len() * 8;
        // SAFETY: `words` owns `byte_len` initialized bytes.
        let bytes: &[u8] =
            unsafe { core::slice::from_raw_parts(frame.words.as_ptr() as *const u8, byte_len) };
        let checked = parse_instruction_frame_checked(bytes).expect("well-formed");

        let mut views = uninit_views::<8>();
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let (pid, count, ix) = unsafe { deserialize_accounts::<8>(frame.as_mut_ptr(), &mut views) };

        assert_eq!(count, checked.account_count);
        assert_eq!(ix, &bytes[checked.instruction_data_range.clone()]);
        assert_eq!(
            pid.as_array().as_slice(),
            &bytes[checked.program_id_offset..checked.program_id_offset + 32]
        );
    }
}
