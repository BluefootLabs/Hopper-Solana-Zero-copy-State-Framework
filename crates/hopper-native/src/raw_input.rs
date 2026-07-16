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

// ── SIMD-0449: the pre-computed account-pointer table ────────────────
//
// SIMD-0449 has the runtime append a `[u64; num_accounts]` array of
// account-record pointers to the input, after the instruction tail,
// "regardless of whether it is read or not" and fully backwards
// compatible — programs that keep scanning simply keep paying O(n).
// Each entry is the address of a CANONICAL `RuntimeAccount` record,
// pre-deduplicated by the runtime (a duplicate slot carries the same
// pointer value as the slot it duplicates), so consuming it needs no
// stride walk and no duplicate-marker resolution.
//
// Hopper is uniquely positioned to consume it: `AccountView` is one
// raw `*mut RuntimeAccount` (const-asserted below), so the SIMD's
// `[u64]` array IS a valid `[AccountView]` — resolution becomes a
// single `from_raw_parts`, where an SDK `AccountInfo`
// (`Rc<RefCell<…>>`) must still loop to construct each element.
//
// Table location (per the SIMD, relative to the SIMD-0321 r2
// instruction-data pointer): the instruction tail is
// `[ix_data][program_id: 32]`, and the table starts at the next
// 8-aligned byte after it. The account COUNT stays where it always
// was — the input buffer's first u64.
//
// The runtime feature gate for SIMD-0449 has NO assigned pubkey yet;
// nothing on any cluster serializes this table today. These functions
// are compiled unconditionally (they are inert unless called); the
// `simd-0449` cargo feature only flips [`SIMD_0449_TABLE_ENABLED`],
// which `hopper_fast_entrypoint!` consults to select the table path —
// a `const`, so the untaken branch folds away entirely.

/// Whether this build trusts the SIMD-0449 account-pointer table
/// (`feature = "simd-0449"`). Enabling it before the SIMD activates on
/// the target cluster reads garbage — ship it only alongside the
/// cluster gate, exactly like `simd-0321`.
pub const SIMD_0449_TABLE_ENABLED: bool = cfg!(feature = "simd-0449");

// Layout precondition for the table cast, checked at compile time: an
// `AccountView` must be exactly one 8-byte pointer for `[u64; n]` to
// reinterpret as `[AccountView; n]`.
const _: () = assert!(
    core::mem::size_of::<AccountView<'static>>() == 8
        && core::mem::align_of::<AccountView<'static>>() == 8,
    "AccountView must stay a single 8-byte pointer for the SIMD-0449 table cast"
);

/// SIMD-0449 O(1) account resolution: overlay the runtime's appended
/// account-pointer table as a borrowed `[AccountView]` — one bounds
/// computation and one `from_raw_parts`, regardless of account count.
///
/// # Safety
///
/// * `input` must point to a valid Solana BPF input buffer.
/// * `instruction_data` must be the loader-serialized instruction data
///   for this invocation (as delivered via the SIMD-0321 `r2`
///   register), with the 32-byte program id trailing it.
/// * The SIMD-0449 table MUST actually be present — i.e. the SIMD is
///   active on the executing cluster. Calling this where the runtime
///   did not serialize the table reads unrelated bytes past the
///   program id.
#[inline(always)]
pub unsafe fn deserialize_accounts_0449<'info>(
    input: *mut u8,
    instruction_data: &'info [u8],
) -> &'info [AccountView<'info>] {
    // SAFETY: the input buffer's first 8 bytes are the account count,
    // unchanged by SIMD-0449.
    let num_accounts = unsafe { core::ptr::read_unaligned(input as *const u64) as usize };
    // Table start: first 8-aligned byte after `[ix_data][program_id]`.
    let tail_end = instruction_data.as_ptr() as usize + instruction_data.len() + 32;
    let table = ((tail_end + (BPF_ALIGN_OF_U128 - 1)) & !(BPF_ALIGN_OF_U128 - 1))
        as *const AccountView<'info>;
    // SAFETY: with the SIMD active, the runtime serialized exactly
    // `num_accounts` pre-deduplicated canonical record pointers at
    // `table`; the layout const-assert above proves `AccountView` is
    // pointer-shaped, and the buffer outlives `'info`.
    unsafe { core::slice::from_raw_parts(table, num_accounts) }
}

/// Adapter matching the `deserialize_accounts_fast` shape: copy up to
/// `MAX` table entries into the caller's array (8 bytes per account —
/// a pointer copy, not a record parse) so the existing entrypoint
/// plumbing consumes the table without changing its account storage.
///
/// # Safety
///
/// Same contract as [`deserialize_accounts_0449`]; additionally
/// `program_id` must be the correct program id for this invocation.
#[inline(always)]
pub unsafe fn deserialize_accounts_0449_into<'info, const MAX: usize>(
    input: *mut u8,
    accounts: &mut [MaybeUninit<AccountView<'info>>; MAX],
    instruction_data: &'info [u8],
    program_id: Address,
) -> (Address, usize, &'info [u8]) {
    // SAFETY: forwarded caller contract.
    let table = unsafe { deserialize_accounts_0449(input, instruction_data) };
    let count = if table.len() > MAX { MAX } else { table.len() };
    let mut slot = 0usize;
    while slot < count {
        // SAFETY: `slot < count <= MAX` and `slot < table.len()`.
        unsafe {
            *accounts.get_unchecked_mut(slot) = MaybeUninit::new(table.get_unchecked(slot).clone());
        }
        slot += 1;
    }
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

// =====================================================================
// Kani proof harnesses for the fused entrypoint walk.
// =====================================================================
//
// Three harness families, run by `scripts/kani-native-rawinput.{sh,ps1}`
// (CI job `kani-native-rawinput-proofs`):
//
// (a) **Stride lemma** — `next_record_offset` equals the checked
//     `align_offset`-style formula for *every* offset reachable inside
//     the SBF input region and every `data_len` up to the loader's
//     10 MiB bound, never overflows, always lands 8-aligned, and always
//     makes progress. Pure integer proof over the full bounded range.
//
// (b) **Bounded differential** — for frames with N <= 3 accounts,
//     symbolic marker bytes and bounded symbolic `data_len` fields
//     (record bodies stay concrete zero to keep CBMC tractable), the
//     fused walk's materialized slot pointers, count, instruction-data
//     range, and program id equal what the in-file safe oracle
//     `parse_instruction_frame_checked` reports. The oracle result is
//     *asserted* Ok, never assumed, so a builder/stride bug fails the
//     proof instead of vacuously pruning paths. Because the buffers are
//     real fixed-size allocations, Kani also model-checks every memory
//     access inside the unsafe walk on these paths — against the
//     *allocation* bound: these accept-side buffers retain worst-case
//     padding slack, so it is the assert-based offset equalities (not
//     the allocation edge) that pin the walk's accesses to the oracle's
//     frame layout; the byte-exact frame-boundary memory check lives in
//     family (c).
//
// (c) **Trap-before-OOB** — `#[kani::should_panic]` harnesses over
//     malformed (self/forward) duplicate markers, with backing buffers
//     sized *exactly* to the encoded frame (no worst-case padding), so
//     any access even one byte past the legitimate frame is a CBMC
//     violation. Precisely, each harness proves two things: the
//     `malformed_duplicate_marker` panic is reachable (existential),
//     AND no path in the assumed space has a non-panic failure (OOB
//     access, invalid write, arithmetic overflow). `should_panic` does
//     NOT by itself prove every malformed marker traps. Universal
//     rejection is machine-checked only where stated: the assert-based
//     `oracle_rejects_exactly_the_malformed_markers` proves the safe
//     oracle rejects *every* malformed marker, and the concrete-marker
//     slot-zero sub-harnesses are deterministic (single path), making
//     their trap verdicts universal for those values. Fused-walk
//     universal rejection follows only from the combination of (a),
//     (b), and a structural argument — see the family (c) block comment
//     for the exact semantics and the residual gap.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    // ── Model constants ─────────────────────────────────────────────

    /// Base of the SBF input memory region (`solana-sbpf`'s
    /// `ebpf::MM_INPUT_START` = 0x4_0000_0000). This base is 8-aligned,
    /// which is the fact `next_record_offset` relies on to fold the
    /// absolute-address `align_offset` into relative-offset math.
    const MM_INPUT_START: usize = 0x4_0000_0000;

    /// Loader bound on serialized account data (10 MiB).
    const LOADER_MAX_DATA_LEN: usize = 10_485_760;

    /// SBF memory regions are 4 GiB apart, so no byte offset inside the
    /// input region can exceed `u32::MAX`.
    const MAX_REGION_OFFSET: usize = u32::MAX as usize;

    /// Bound on the symbolic per-account `data_len` in the differential
    /// harnesses. 8 covers every alignment residue 0..=7 plus one exact
    /// stride boundary; family (a) covers the full 10 MiB range.
    const MAX_DL: usize = 8;

    /// Bound on the symbolic instruction-data length.
    const MAX_IX: usize = 8;

    /// Worst-case bytes one canonical record consumes when
    /// `data_len <= MAX_DL` (a duplicate slot consumes 8 < this).
    const RECORD_MAX: usize = next_record_offset(0, MAX_DL);

    /// Buffer bytes covering `n` worst-case records plus the count
    /// prefix, instruction tail, and program-id trailer.
    const fn frame_len(n: usize) -> usize {
        8 + n * RECORD_MAX + 8 + MAX_IX + 32
    }

    /// Recognizable instruction-data filler.
    const IX_SENTINEL: [u8; MAX_IX] = [0xA5; MAX_IX];
    /// Recognizable program-id trailer.
    const PID_SENTINEL: [u8; 32] = [0xC4; 32];
    /// One 8-byte word of [`PID_SENTINEL`]. The program id is written and
    /// compared a word at a time (see `write_frame` /
    /// `check_fused_walk_against_oracle`) so the harness never contains a
    /// 32-byte `memcpy`/`memcmp` loop — such a loop would force the whole
    /// harness unwind past 32 and blow up the SAT formula. Every real loop
    /// then fits in `unwind(10)`, matching the trap/stride harnesses.
    const PID_WORD: [u8; 8] = [0xC4; 8];

    /// 8-aligned fixed-size backing buffer, mirroring the loader
    /// guarantee that the input region starts at the 8-aligned
    /// `MM_INPUT_START`.
    #[repr(C, align(8))]
    struct AlignedBuf<const LEN: usize>([u8; LEN]);

    // ── Kani-friendly symbolic values ───────────────────────────────

    /// Symbolic marker constrained to the loader's well-formed set for
    /// slot `i`: canonical (0xFF) or a strictly-earlier slot index.
    fn any_valid_marker(i: usize) -> u8 {
        let m: u8 = kani::any();
        kani::assume(m == u8::MAX || (m as usize) < i);
        m
    }

    /// Symbolic `data_len` bounded to keep the frame inside `RECORD_MAX`.
    fn any_bounded_data_len() -> usize {
        let dl: usize = kani::any();
        kani::assume(dl <= MAX_DL);
        dl
    }

    /// Symbolic instruction-data length bounded by the sentinel size.
    fn any_bounded_ix_len() -> usize {
        let n: usize = kani::any();
        kani::assume(n <= MAX_IX);
        n
    }

    // ── Kani-friendly frame builder ─────────────────────────────────

    /// Serialize a loader input frame into `buf` (which must be zeroed):
    /// concrete account count `N`, symbolic marker bytes, bounded
    /// symbolic `data_len` fields, concrete-zero record bodies, and
    /// sentinel instruction-data / program-id bytes. Returns the
    /// exclusive end offset of the encoded frame (one past the program
    /// id), which the `trap_frame_layout_is_exact_*` harnesses use to
    /// prove the trap-family buffers are sized exactly.
    ///
    /// Record placement reuses `next_record_offset`, but this is not
    /// circular: the accept-side harnesses *assert* (never assume) that
    /// the independent bounds-checked oracle accepts the frame and lands
    /// on the same offsets, so a stride bug becomes an assertion failure
    /// rather than a vacuously-pruned path.
    fn write_frame<const N: usize>(
        buf: &mut [u8],
        markers: &[u8; N],
        data_lens: &[usize; N],
        ix_len: usize,
    ) -> usize {
        buf[0..8].copy_from_slice(&(N as u64).to_le_bytes());
        let mut pos = 8usize;
        let mut i = 0;
        while i < N {
            buf[pos] = markers[i];
            if markers[i] == u8::MAX {
                // Canonical record: `data_len` lives at header offset 80.
                // Body bytes (data, realloc reserve, padding, rent epoch)
                // stay concrete zero to keep CBMC tractable.
                buf[pos + 80..pos + 88].copy_from_slice(&(data_lens[i] as u64).to_le_bytes());
                pos = next_record_offset(pos, data_lens[i]);
            } else {
                // Duplicate slot: marker byte + 7 zero padding bytes.
                pos += 8;
            }
            i += 1;
        }
        buf[pos..pos + 8].copy_from_slice(&(ix_len as u64).to_le_bytes());
        pos += 8;
        buf[pos..pos + ix_len].copy_from_slice(&IX_SENTINEL[..ix_len]);
        pos += ix_len;
        // Program id written as 4x 8-byte words (never one 32-byte copy):
        // keeps the harness free of any 32-iteration memcpy loop.
        let mut w = 0;
        while w < 4 {
            buf[pos + w * 8..pos + w * 8 + 8].copy_from_slice(&PID_WORD);
            w += 1;
        }
        pos + 32
    }

    /// Resolve a slot to its canonical record slot by chasing duplicate
    /// markers. Terminates because well-formed markers strictly decrease.
    fn resolve_canonical<const N: usize>(markers: &[u8; N], mut i: usize) -> usize {
        while markers[i] != u8::MAX {
            i = markers[i] as usize;
        }
        i
    }

    // ── Family (a): stride lemma ────────────────────────────────────

    /// For every offset reachable inside the input region and every
    /// loader-permitted `data_len`, the folded integer stride equals the
    /// checked `align_offset`-style formula (computed on the *absolute*
    /// `MM_INPUT_START`-based address), never overflows, stays 8-aligned,
    /// and strictly advances. No unwinding concerns: straight-line
    /// integer math over the full bounded range.
    #[kani::proof]
    fn stride_lemma_matches_checked_align_offset_formula() {
        let offset: usize = kani::any();
        let data_len: usize = kani::any();
        kani::assume(offset <= MAX_REGION_OFFSET);
        kani::assume(data_len <= LOADER_MAX_DATA_LEN);

        // Checked reference: the pre-fusion cursor advance. Every
        // `checked_add` doubles as the no-overflow proof.
        let unpadded = offset
            .checked_add(RuntimeAccount::SIZE)
            .and_then(|x| x.checked_add(data_len))
            .and_then(|x| x.checked_add(MAX_PERMITTED_DATA_INCREASE))
            .expect("pre-alignment cursor must not overflow");
        // `align_offset`-style padding on the absolute address, exactly
        // what `scan_instruction_frame` computes via `align_offset` and
        // `parse_instruction_frame_checked` via `wrapping_neg`.
        let absolute = MM_INPUT_START
            .checked_add(unpadded)
            .expect("absolute address must not overflow");
        let pad_absolute = absolute.wrapping_neg() & (BPF_ALIGN_OF_U128 - 1);
        // The 8-aligned-base lemma: relative and absolute padding agree.
        let pad_relative = unpadded.wrapping_neg() & (BPF_ALIGN_OF_U128 - 1);
        assert_eq!(pad_absolute, pad_relative);
        let expected = unpadded
            .checked_add(pad_absolute)
            .and_then(|x| x.checked_add(8))
            .expect("aligned cursor must not overflow");

        // Kani's built-in overflow checks cover the unchecked `+` chain
        // inside `next_record_offset` itself.
        let got = next_record_offset(offset, data_len);
        assert_eq!(got, expected);
        assert_eq!(got & (BPF_ALIGN_OF_U128 - 1), 0);
        assert!(got > offset);
    }

    // ── Family (b): bounded differential vs the safe oracle ────────

    /// Accept-side differential body shared by the `deserialize_accounts`
    /// harnesses: build a frame with `N` symbolic well-formed slots,
    /// require the safe oracle to accept it, run the fused walk with
    /// capacity `MAX`, and assert both parsers agree on every observable.
    fn check_fused_walk_against_oracle<const N: usize, const MAX: usize, const LEN: usize>() {
        let mut markers = [0u8; N];
        let mut data_lens = [0usize; N];
        let mut i = 0;
        while i < N {
            markers[i] = any_valid_marker(i);
            data_lens[i] = any_bounded_data_len();
            i += 1;
        }
        let ix_len = any_bounded_ix_len();

        let mut backing = AlignedBuf::<LEN>([0u8; LEN]);
        write_frame::<N>(&mut backing.0, &markers, &data_lens, ix_len);

        // Asserted, not assumed: see `write_frame` docs.
        let oracle = parse_instruction_frame_checked(&backing.0)
            .expect("oracle must accept a well-formed loader frame");
        assert_eq!(oracle.account_count, N);
        assert_eq!(oracle.instruction_data_range.len(), ix_len);

        let base = backing.0.as_ptr() as usize;
        // SAFETY: an array of `MaybeUninit` is valid in the uninitialized
        // state by definition.
        let mut views: [MaybeUninit<AccountView<'_>>; MAX] =
            unsafe { MaybeUninit::uninit().assume_init() };
        // SAFETY: `backing` is an 8-aligned loader-layout buffer built by
        // `write_frame` and accepted by the bounds-checked oracle above,
        // satisfying the "valid Solana BPF input buffer" contract; Kani
        // additionally model-checks every memory access inside the walk.
        let (pid, count, ix) =
            unsafe { deserialize_accounts::<MAX>(backing.0.as_mut_ptr(), &mut views) };

        // Count: the fused walk clamps at MAX (the 254 clamp is
        // unreachable for N <= 3).
        let expected_count = if N > MAX { MAX } else { N };
        assert_eq!(count, expected_count);

        // Every materialized slot resolves to exactly the canonical
        // record offset the oracle reported.
        let mut s = 0;
        while s < count {
            let canon = resolve_canonical::<N>(&markers, s);
            // SAFETY: slots `0..count` were initialized by the fused walk.
            let got = unsafe { views[s].assume_init_ref() }.raw_ptr() as usize;
            assert_eq!(got - base, oracle.slot_offsets[canon]);
            s += 1;
        }

        // Instruction-data range agrees (start and length), which also
        // pins the program-id offset: both parsers read it at the end of
        // the instruction data.
        assert_eq!(ix.len(), oracle.instruction_data_range.len());
        assert_eq!(
            ix.as_ptr() as usize - base,
            oracle.instruction_data_range.start
        );
        assert_eq!(oracle.program_id_offset, oracle.instruction_data_range.end);
        // Word-wise program-id equality: four u64 comparisons rather than a
        // 32-byte slice `==` (which lowers to a `memcmp` loop that would
        // force the harness unwind past 32). Reads are scalar; no loop
        // exceeds `unwind(10)`.
        let pid_bytes = pid.as_array();
        let poff = oracle.program_id_offset;
        let mut w = 0;
        while w < 4 {
            let o = w * 8;
            let got = u64::from_le_bytes([
                pid_bytes[o],
                pid_bytes[o + 1],
                pid_bytes[o + 2],
                pid_bytes[o + 3],
                pid_bytes[o + 4],
                pid_bytes[o + 5],
                pid_bytes[o + 6],
                pid_bytes[o + 7],
            ]);
            let want = u64::from_le_bytes([
                backing.0[poff + o],
                backing.0[poff + o + 1],
                backing.0[poff + o + 2],
                backing.0[poff + o + 3],
                backing.0[poff + o + 4],
                backing.0[poff + o + 5],
                backing.0[poff + o + 6],
                backing.0[poff + o + 7],
            ]);
            assert_eq!(got, want);
            w += 1;
        }
    }

    #[kani::proof]
    // 10 suffices: the program id is written and compared a word at a time
    // (see `PID_WORD`), so the harness contains no 32-byte memcpy/memcmp
    // loop — every real loop (skip-tail, materialize, 4-word compares) is
    // <= 9 iterations. A naive 32-byte slice `==` here previously forced
    // the bound past 32 and blew up the SAT formula.
    #[kani::unwind(10)]
    fn differential_zero_accounts() {
        check_fused_walk_against_oracle::<0, 4, { frame_len(0) }>();
    }

    #[kani::proof]
    #[kani::unwind(10)]
    fn differential_one_canonical_account() {
        check_fused_walk_against_oracle::<1, 4, { frame_len(1) }>();
    }

    #[kani::proof]
    #[kani::unwind(10)]
    fn differential_two_accounts_symbolic_markers() {
        check_fused_walk_against_oracle::<2, 4, { frame_len(2) }>();
    }

    #[kani::proof]
    #[kani::unwind(10)]
    fn differential_three_accounts_symbolic_markers() {
        check_fused_walk_against_oracle::<3, 4, { frame_len(3) }>();
    }

    /// Accounts beyond `MAX` take the skip-only tail: the cursor must
    /// still advance record-exactly so the instruction tail is found.
    #[kani::proof]
    #[kani::unwind(10)]
    fn differential_skip_only_tail_beyond_max() {
        check_fused_walk_against_oracle::<3, 1, { frame_len(3) }>();
    }

    /// `deserialize_accounts_fast` shares the stride but never scans the
    /// tail; its materialized slots must still match the oracle's.
    #[kani::proof]
    #[kani::unwind(10)]
    fn differential_fast_walk_two_accounts() {
        const N: usize = 2;
        const LEN: usize = frame_len(N);
        let markers = [u8::MAX, any_valid_marker(1)];
        let data_lens = [any_bounded_data_len(), any_bounded_data_len()];

        let mut backing = AlignedBuf::<LEN>([0u8; LEN]);
        write_frame::<N>(&mut backing.0, &markers, &data_lens, 0);

        let oracle = parse_instruction_frame_checked(&backing.0)
            .expect("oracle must accept a well-formed loader frame");

        let base = backing.0.as_ptr() as usize;
        // SAFETY: an array of `MaybeUninit` is valid in the uninitialized
        // state by definition.
        let mut views: [MaybeUninit<AccountView<'_>>; 4] =
            unsafe { MaybeUninit::uninit().assume_init() };
        static EMPTY_IX: [u8; 0] = [];
        // SAFETY: same oracle-validated 8-aligned loader-layout buffer
        // contract as `check_fused_walk_against_oracle`; instruction data
        // and program id are supplied out of band per the fast-path
        // contract and are opaque pass-throughs to this walk.
        let (pid, count, ix) = unsafe {
            deserialize_accounts_fast::<4>(
                backing.0.as_mut_ptr(),
                &mut views,
                &EMPTY_IX,
                Address::new_from_array(PID_SENTINEL),
            )
        };
        assert_eq!(count, N);
        assert_eq!(ix.len(), 0);
        assert_eq!(pid.as_array(), &PID_SENTINEL);

        let mut s = 0;
        while s < count {
            let canon = resolve_canonical::<N>(&markers, s);
            // SAFETY: slots `0..count` were initialized by the fast walk.
            let got = unsafe { views[s].assume_init_ref() }.raw_ptr() as usize;
            assert_eq!(got - base, oracle.slot_offsets[canon]);
            s += 1;
        }
    }

    /// `scan_instruction_frame` (the lazy-path scanner, which still uses
    /// pointer `align_offset` internally) must locate the same account
    /// span and instruction tail as the oracle.
    #[kani::proof]
    #[kani::unwind(10)]
    fn differential_scan_frame_two_accounts() {
        const N: usize = 2;
        const LEN: usize = frame_len(N);
        let markers = [u8::MAX, any_valid_marker(1)];
        let data_lens = [any_bounded_data_len(), any_bounded_data_len()];
        let ix_len = any_bounded_ix_len();

        let mut backing = AlignedBuf::<LEN>([0u8; LEN]);
        write_frame::<N>(&mut backing.0, &markers, &data_lens, ix_len);

        let oracle = parse_instruction_frame_checked(&backing.0)
            .expect("oracle must accept a well-formed loader frame");

        let base = backing.0.as_ptr() as usize;
        // SAFETY: same oracle-validated 8-aligned loader-layout buffer
        // contract as `check_fused_walk_against_oracle`.
        let frame = unsafe { scan_instruction_frame(backing.0.as_mut_ptr()) };

        assert_eq!(frame.account_count, N);
        assert_eq!(frame.accounts_start as usize - base, 8);
        assert_eq!(
            frame.instruction_data.len(),
            oracle.instruction_data_range.len()
        );
        assert_eq!(
            frame.instruction_data.as_ptr() as usize - base,
            oracle.instruction_data_range.start
        );
        assert_eq!(
            frame.program_id.as_array().as_slice(),
            &backing.0[oracle.program_id_offset..oracle.program_id_offset + 32]
        );
    }

    // ── Family (c): trap-before-OOB on malformed markers ───────────
    //
    // Proof semantics, stated precisely. `#[kani::should_panic]` is
    // EXISTENTIAL on the panic side: a harness verifies iff
    //   (1) at least one path in the assumed input space panics, and
    //   (2) NO path exhibits a non-panic property failure — an
    //       out-of-bounds read/write, an invalid `accounts[]` write, or
    //       an arithmetic overflow is a verification FAILURE, because
    //       those are not panics.
    // Clause (2) holds on EVERY path; clause (1) alone does NOT prove
    // that every malformed marker traps — a hypothetical path that
    // silently *returned* for some malformed marker would still verify.
    // Universal statements are machine-checked only where noted:
    //   * `oracle_rejects_exactly_the_malformed_markers` is assert-based
    //     (no `should_panic`), so it proves the safe oracle rejects
    //     EVERY malformed marker in the symbolic space;
    //   * the `trap_slot_zero_marker_*` sub-harnesses each fix one
    //     CONCRETE marker, making execution deterministic (one path),
    //     so their `should_panic` verdicts are universal for those
    //     specific marker values;
    //   * "the fused walk traps on every malformed marker on every
    //     path" is NOT established by any single harness here. It
    //     follows in combination: family (b) pins the accept side to
    //     the oracle, the oracle harness pins the reject set, the
    //     stride lemma (a) pins the cursor, and structurally the walk's
    //     only non-trapping branch for a non-0xFF marker is
    //     `duplicate_of < slot`, which the harness assumptions exclude.
    //     That final step is a source-level argument, not a CBMC check.
    //
    // Exact allocation — the mechanism every trap harness below uses
    // (this is what makes clause (2) sharp): each backing buffer is
    // sized TO THE BYTE of the encoded malformed frame, with no
    // worst-case padding, so a read or write even one byte past the
    // legitimate frame is a CBMC violation instead of slack absorbed by
    // an oversized allocation. The two-slot frame's length depends on
    // the symbolic `data_len` only through u128 alignment: `dl == 0`
    // needs one 8-byte padding step fewer than `dl` in `1..=MAX_DL`,
    // which all encode to the same length (compile-time-checked below).
    // Each two-slot trap harness is therefore split into exactly two
    // size classes — `_dl0` (concrete `dl = 0`) and `_dl_nonzero`
    // (symbolic `dl` in `1..=MAX_DL`) — each with an exactly-sized
    // buffer; together they cover the same `0..=MAX_DL` space the
    // padded originals did. The slot-zero frame has no `data_len` at
    // all, so a single exact size covers it.
    //
    // The `trap_frame_layout_is_exact_*` companions prove, assert-based
    // over the SAME symbolic space, that the builder fills each buffer
    // exactly (`end == LEN`) and never panics while doing so — so a
    // `should_panic` trap harness cannot pass vacuously via a builder
    // panic or leave hidden slack.
    //
    // The trap harnesses themselves are deliberately assertion-free: an
    // `assert!` before the call would itself panic on failure and be
    // masked by `should_panic`.

    /// Exact encoded length of the canonical-then-malformed two-slot
    /// trap frame: 8-byte count prefix, canonical record 0 starting at
    /// offset 8 with `data_len = dl`, 8-byte malformed duplicate slot,
    /// 8-byte instruction-data length (zero, no data bytes), 32-byte
    /// program id.
    const fn trap_frame_len(dl: usize) -> usize {
        next_record_offset(8, dl) + 8 + 8 + 32
    }

    /// Two-slot trap frame length for the `dl = 0` size class.
    const TRAP_LEN_DL0: usize = trap_frame_len(0);
    /// Two-slot trap frame length shared by every `dl` in `1..=MAX_DL`
    /// (u128 alignment folds them all to one size).
    const TRAP_LEN_DL_NONZERO: usize = trap_frame_len(1);
    /// Exact encoded length of the one-slot slot-zero trap frame:
    /// count prefix + 8-byte duplicate slot + ix-len prefix + program id.
    const TRAP_LEN_SLOT_ZERO: usize = 8 + 8 + 8 + 32;

    // Compile-time proof that the two size classes are exhaustive over
    // `0..=MAX_DL`: every nonzero `dl` encodes to `TRAP_LEN_DL_NONZERO`
    // and `dl = 0` is strictly its own (smaller) class.
    const _: () = {
        let mut dl = 1;
        while dl <= MAX_DL {
            assert!(trap_frame_len(dl) == TRAP_LEN_DL_NONZERO);
            dl += 1;
        }
        assert!(TRAP_LEN_DL0 < TRAP_LEN_DL_NONZERO);
    };

    /// Build the canonical-then-malformed two-slot trap frame: slot 0 is
    /// canonical with symbolic `data_len` drawn from `dl_min..=dl_max`
    /// (one exact-size class), slot 1 carries a symbolic malformed
    /// marker (`!= 0xFF`, `>= 1`, i.e. self or forward reference at
    /// slot 1). Returns the buffer and the builder's exclusive end
    /// offset; the `trap_frame_layout_is_exact_*` harnesses assert
    /// `end == LEN` over this same symbolic space.
    fn build_two_slot_trap_frame<const LEN: usize>(
        dl_min: usize,
        dl_max: usize,
    ) -> (AlignedBuf<LEN>, usize) {
        let bad: u8 = kani::any();
        kani::assume(bad != u8::MAX && bad as usize >= 1);
        let dl: usize = kani::any();
        kani::assume(dl >= dl_min && dl <= dl_max);

        let mut backing = AlignedBuf::<LEN>([0u8; LEN]);
        let end = write_frame::<2>(&mut backing.0, &[u8::MAX, bad], &[dl, 0], 0);
        (backing, end)
    }

    /// Build the one-slot slot-zero trap frame whose sole slot carries
    /// `marker` (symbolic or concrete; the caller guarantees it is not
    /// 0xFF, so the slot encodes as an 8-byte duplicate slot).
    fn build_slot_zero_trap_frame(marker: u8) -> (AlignedBuf<TRAP_LEN_SLOT_ZERO>, usize) {
        let mut backing = AlignedBuf::<TRAP_LEN_SLOT_ZERO>([0u8; TRAP_LEN_SLOT_ZERO]);
        let end = write_frame::<1>(&mut backing.0, &[marker], &[0], 0);
        (backing, end)
    }

    // Assert-based (NOT should_panic) exactness companions: over the
    // same symbolic space as the trap harnesses, the builder terminates
    // without panicking and fills the buffer to exactly `LEN` bytes.
    // These close the two vacuity holes of the trap family: a builder
    // panic masked by `should_panic`, and hidden slack past the frame.

    #[kani::proof]
    #[kani::unwind(10)]
    fn trap_frame_layout_is_exact_dl0() {
        let (_backing, end) = build_two_slot_trap_frame::<TRAP_LEN_DL0>(0, 0);
        assert_eq!(end, TRAP_LEN_DL0);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    fn trap_frame_layout_is_exact_dl_nonzero() {
        let (_backing, end) = build_two_slot_trap_frame::<TRAP_LEN_DL_NONZERO>(1, MAX_DL);
        assert_eq!(end, TRAP_LEN_DL_NONZERO);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    fn trap_frame_layout_is_exact_slot_zero() {
        let marker: u8 = kani::any();
        kani::assume(marker != u8::MAX);
        let (_backing, end) = build_slot_zero_trap_frame(marker);
        assert_eq!(end, TRAP_LEN_SLOT_ZERO);
    }

    /// Shared trap body: run `deserialize_accounts::<MAX>` on one
    /// exact-size malformed two-slot frame class. `MAX >= 2` puts the
    /// malformed slot 1 in the materialize range; `MAX = 1` pushes it
    /// into the skip-only tail loop.
    fn trap_deserialize_two_slot<const MAX: usize, const LEN: usize>(dl_min: usize, dl_max: usize) {
        let (mut backing, _end) = build_two_slot_trap_frame::<LEN>(dl_min, dl_max);
        // SAFETY: an array of `MaybeUninit` is valid in the uninitialized
        // state by definition.
        let mut views: [MaybeUninit<AccountView<'_>>; MAX] =
            unsafe { MaybeUninit::uninit().assume_init() };
        // SAFETY: 8-aligned loader-layout buffer sized exactly to the
        // encoded frame (`trap_frame_layout_is_exact_*`); the malformed
        // marker is the condition under test and must trap before any
        // access past the frame end — Kani checks every access on every
        // path of this harness against that exact allocation boundary.
        let _ = unsafe { deserialize_accounts::<MAX>(backing.0.as_mut_ptr(), &mut views) };
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_on_malformed_marker_in_materialize_range_dl0() {
        trap_deserialize_two_slot::<4, TRAP_LEN_DL0>(0, 0);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_on_malformed_marker_in_materialize_range_dl_nonzero() {
        trap_deserialize_two_slot::<4, TRAP_LEN_DL_NONZERO>(1, MAX_DL);
    }

    // MAX = 1, so the malformed slot 1 is handled by the skip-only tail
    // loop — the trap must fire there exactly as in the materialize
    // range.

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_on_malformed_marker_in_skip_only_tail_dl0() {
        trap_deserialize_two_slot::<1, TRAP_LEN_DL0>(0, 0);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_on_malformed_marker_in_skip_only_tail_dl_nonzero() {
        trap_deserialize_two_slot::<1, TRAP_LEN_DL_NONZERO>(1, MAX_DL);
    }

    /// Shared trap body for `deserialize_accounts_fast` on one
    /// exact-size malformed two-slot frame class.
    fn trap_fast_walk_two_slot<const LEN: usize>(dl_min: usize, dl_max: usize) {
        let (mut backing, _end) = build_two_slot_trap_frame::<LEN>(dl_min, dl_max);
        // SAFETY: an array of `MaybeUninit` is valid in the uninitialized
        // state by definition.
        let mut views: [MaybeUninit<AccountView<'_>>; 4] =
            unsafe { MaybeUninit::uninit().assume_init() };
        static EMPTY_IX: [u8; 0] = [];
        // SAFETY: 8-aligned loader-layout buffer sized exactly to the
        // encoded frame, with out-of-band tail per the fast-path
        // contract; the malformed marker is the condition under test and
        // must trap before any access past the frame end — Kani checks
        // every access on every path against that exact allocation
        // boundary.
        let _ = unsafe {
            deserialize_accounts_fast::<4>(
                backing.0.as_mut_ptr(),
                &mut views,
                &EMPTY_IX,
                Address::new_from_array(PID_SENTINEL),
            )
        };
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_in_fast_walk_dl0() {
        trap_fast_walk_two_slot::<TRAP_LEN_DL0>(0, 0);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_in_fast_walk_dl_nonzero() {
        trap_fast_walk_two_slot::<TRAP_LEN_DL_NONZERO>(1, MAX_DL);
    }

    /// Shared trap body for `scan_instruction_frame` on one exact-size
    /// malformed two-slot frame class.
    fn trap_scan_frame_two_slot<const LEN: usize>(dl_min: usize, dl_max: usize) {
        let (mut backing, _end) = build_two_slot_trap_frame::<LEN>(dl_min, dl_max);
        // SAFETY: 8-aligned loader-layout buffer sized exactly to the
        // encoded frame; the malformed marker is the condition under test
        // and must trap before any access past the frame end — Kani
        // checks every access on every path against that exact
        // allocation boundary.
        let _ = unsafe { scan_instruction_frame(backing.0.as_mut_ptr()) };
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_in_scan_frame_dl0() {
        trap_scan_frame_two_slot::<TRAP_LEN_DL0>(0, 0);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_in_scan_frame_dl_nonzero() {
        trap_scan_frame_two_slot::<TRAP_LEN_DL_NONZERO>(1, MAX_DL);
    }

    /// Shared body for the slot-zero trap harnesses: slot 0 has no
    /// earlier slot, so every non-canonical marker value is malformed
    /// there. Exact-size buffer, no `data_len` dimension at all.
    fn trap_slot_zero(marker: u8) {
        let (mut backing, _end) = build_slot_zero_trap_frame(marker);
        // SAFETY: an array of `MaybeUninit` is valid in the uninitialized
        // state by definition.
        let mut views: [MaybeUninit<AccountView<'_>>; 4] =
            unsafe { MaybeUninit::uninit().assume_init() };
        // SAFETY: 8-aligned loader-layout buffer sized exactly to the
        // encoded frame (`trap_frame_layout_is_exact_slot_zero`); the
        // malformed marker is the condition under test and must trap
        // before any access past the frame end — Kani checks every
        // access on every path against that exact allocation boundary.
        let _ = unsafe { deserialize_accounts::<4>(backing.0.as_mut_ptr(), &mut views) };
    }

    /// Existential over the full symbolic malformed-marker space at
    /// slot 0 (see the family (c) comment for exactly what that means).
    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_fires_on_any_duplicate_marker_at_slot_zero() {
        let bad: u8 = kani::any();
        kani::assume(bad != u8::MAX);
        trap_slot_zero(bad);
    }

    // Per-concrete-value slot-zero sub-harnesses: with every input byte
    // concrete, execution is deterministic — a single path — so each
    // `should_panic` verdict below is UNIVERSAL for that marker value
    // (the walk provably traps on it), not merely existential.

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_slot_zero_marker_0x00_self_reference() {
        trap_slot_zero(0x00);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_slot_zero_marker_0x01_forward_reference() {
        trap_slot_zero(0x01);
    }

    #[kani::proof]
    #[kani::unwind(10)]
    #[kani::should_panic]
    fn trap_slot_zero_marker_0xfe_max_forward_reference() {
        trap_slot_zero(0xFE);
    }

    /// Oracle side of the rejection story, and the only harness in this
    /// module that machine-checks a UNIVERSAL rejection property: it is
    /// assert-based (no `should_panic`), so over *fully* symbolic
    /// markers for a two-slot frame it proves the safe parser accepts
    /// iff both markers are well-formed, and every rejection is
    /// precisely `MalformedDuplicateMarker` — on every path. Combined
    /// with family (b) (well-formed => both parsers accept, outputs
    /// equal) and the family (c) trap harnesses (existential trap
    /// reachability + no memory-safety failure on any assumed path,
    /// against exact-size buffers), this supports — but note, per the
    /// family (c) comment, does not single-handedly machine-check —
    /// "both reject exactly the same inputs" for the marker dimension.
    #[kani::proof]
    #[kani::unwind(10)]
    fn oracle_rejects_exactly_the_malformed_markers() {
        const LEN: usize = frame_len(2);
        let m0: u8 = kani::any();
        let m1: u8 = kani::any();
        let data_lens = [any_bounded_data_len(), any_bounded_data_len()];
        let ix_len = any_bounded_ix_len();

        let mut backing = AlignedBuf::<LEN>([0u8; LEN]);
        write_frame::<2>(&mut backing.0, &[m0, m1], &data_lens, ix_len);

        let result = parse_instruction_frame_checked(&backing.0);
        let well_formed = m0 == u8::MAX && (m1 == u8::MAX || m1 == 0);
        assert_eq!(result.is_ok(), well_formed);
        if let Err(err) = result {
            assert!(matches!(err, FrameError::MalformedDuplicateMarker { .. }));
        }
    }
}
