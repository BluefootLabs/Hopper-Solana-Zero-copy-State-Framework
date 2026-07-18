//! Substrate conformance: Hopper's entrypoint layout assumptions,
//! pinned against a byte-exact reconstruction of the Solana BPF loader
//! input format.
//!
//! Two jobs:
//!
//! 1. **Drift alarm.** The scanning deserializer's stride math folds the
//!    entire per-account record (88-byte `RuntimeAccount` header, data,
//!    the 10,240-byte realloc reserve, u128-alignment padding, 8-byte
//!    rent epoch) into integer adds + a mask. If upstream ever changes
//!    any of those constants, these tests fail loudly instead of the
//!    parser reading garbage on-chain.
//! 2. **SIMD-0449 equivalence.** The O(1) account-pointer-table path
//!    (`deserialize_accounts_0449`) must resolve to EXACTLY the views
//!    the O(n) stride walk produces — pinned by building the same frame
//!    with a synthetic table appended and comparing view-for-view.
//!    The table functions are compiled unconditionally (the `simd-0449`
//!    feature only flips the entrypoint's const selector), so this
//!    equivalence is enforced on every CI run, not just feature runs.

use core::mem::MaybeUninit;

use hopper_native::raw_input::{
    deserialize_accounts, deserialize_accounts_0449, deserialize_accounts_0449_checked,
    DirectMappingError,
};
use hopper_native::{AccountView, RuntimeAccount, MAX_PERMITTED_DATA_INCREASE};

const ALIGN: usize = 8;

/// One account slot description for the frame builder.
enum Slot {
    /// Canonical account: 0xFF marker byte (doubling as `borrow_state`),
    /// 88-byte header, `data_len` bytes, realloc reserve, alignment
    /// padding, rent-epoch tail.
    Fresh { data_len: usize, lamports: u64 },
    /// Duplicate reference: 1 marker byte + 7 padding bytes.
    Dup(u8),
}

/// 8-aligned loader-input fixture (u64 backing ⇒ 8-aligned base,
/// matching the loader's `MM_INPUT_START` guarantee the folded stride
/// relies on). Also records each canonical record's byte offset so the
/// SIMD-0449 table can be synthesized with real pointers.
struct Frame {
    words: Vec<u64>,
    /// Byte offset of each slot's CANONICAL record (duplicates carry
    /// the offset of the record they reference).
    canonical_offsets: Vec<usize>,
}

impl Frame {
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.words.as_mut_ptr() as *mut u8
    }
}

/// Serialize a loader input frame per the Solana BPF loader layout,
/// optionally appending a synthetic SIMD-0449 account-pointer table
/// (patched with real addresses after the backing is allocated).
fn build_frame(slots: &[Slot], ix_data: &[u8], program_id: [u8; 32], with_0449: bool) -> Frame {
    let mut buf: Vec<u8> = Vec::new();
    let mut canonical_offsets: Vec<usize> = Vec::new();
    buf.extend_from_slice(&(slots.len() as u64).to_le_bytes());

    for (i, slot) in slots.iter().enumerate() {
        match slot {
            Slot::Fresh { data_len, lamports } => {
                canonical_offsets.push(buf.len());
                let mut header = [0u8; RuntimeAccount::SIZE];
                header[0] = 0xFF; // canonical marker / borrow_state
                header[1] = 0; // is_signer
                header[2] = 1; // is_writable
                               // address: recognizable per-slot pattern
                header[8..40].copy_from_slice(&[i as u8 + 1; 32]);
                // owner
                header[40..72].copy_from_slice(&[0x55; 32]);
                header[72..80].copy_from_slice(&lamports.to_le_bytes());
                header[80..88].copy_from_slice(&(*data_len as u64).to_le_bytes());
                buf.extend_from_slice(&header);
                buf.extend_from_slice(&vec![0xABu8; *data_len]);
                buf.extend_from_slice(&vec![0u8; MAX_PERMITTED_DATA_INCREASE]);
                while buf.len() % ALIGN != 0 {
                    buf.push(0);
                }
                buf.extend_from_slice(&u64::MAX.to_le_bytes()); // rent epoch
            }
            Slot::Dup(of) => {
                canonical_offsets.push(canonical_offsets[*of as usize]);
                buf.push(*of);
                buf.extend_from_slice(&[0u8; 7]); // duplicate padding
            }
        }
    }

    // Instruction tail: u64 LE length, data, 32-byte program id.
    buf.extend_from_slice(&(ix_data.len() as u64).to_le_bytes());
    buf.extend_from_slice(ix_data);
    buf.extend_from_slice(&program_id);

    // SIMD-0449 table: first 8-aligned byte after the program id, one
    // canonical-record pointer per slot (pre-deduplicated). Reserve the
    // space now; patch real addresses once the backing is final.
    let mut table_offset = None;
    if with_0449 {
        while buf.len() % ALIGN != 0 {
            buf.push(0);
        }
        table_offset = Some(buf.len());
        for _ in slots {
            buf.extend_from_slice(&0u64.to_le_bytes());
        }
    }

    // Word-aligned backing (alignment 8, like MM_INPUT_START).
    let mut words = vec![0u64; buf.len().div_ceil(8)];
    // SAFETY: `words` owns at least `buf.len()` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), words.as_mut_ptr() as *mut u8, buf.len());
    }

    let mut frame = Frame {
        words,
        canonical_offsets,
    };

    // Patch the table with the REAL record addresses in the final
    // allocation — exactly what the runtime would serialize.
    if let Some(table_at) = table_offset {
        let base = frame.as_mut_ptr() as usize;
        for (i, rec_off) in frame.canonical_offsets.clone().iter().enumerate() {
            let entry = (base + rec_off) as u64;
            // SAFETY: `table_at + 8*i + 8 <= buf.len()` by construction.
            unsafe {
                core::ptr::write_unaligned(
                    (frame.as_mut_ptr()).add(table_at + 8 * i) as *mut u64,
                    entry,
                );
            }
        }
    }
    frame
}

fn walk<const MAX: usize>(frame: &mut Frame) -> (Vec<AccountView<'static>>, Vec<u8>, [u8; 32]) {
    const UNINIT: MaybeUninit<AccountView<'static>> = MaybeUninit::uninit();
    let mut accounts = [UNINIT; MAX];
    // SAFETY: `frame` is a well-formed loader input fixture with an
    // 8-aligned base, alive for the duration of this test.
    let (program_id, count, ix) =
        unsafe { deserialize_accounts::<MAX>(frame.as_mut_ptr(), &mut accounts) };
    let views = accounts[..count]
        .iter()
        // SAFETY: slots below `count` were initialized by the parser.
        .map(|slot| unsafe { slot.assume_init_ref() }.clone())
        .collect();
    (views, ix.to_vec(), *program_id.as_array())
}

/// Locate the exact in-frame instruction-data slice and the SIMD-0449 table.
/// Raw addresses are returned so tests can keep using the frame while the
/// borrowed instruction slice is live.
fn locate_0449_tail(frame: &mut Frame, ix: &[u8], program_id: [u8; 32]) -> (*mut u8, usize, usize) {
    let base = frame.as_mut_ptr();
    let len = frame.words.len() * 8;
    // SAFETY: `words` owns `len` readable bytes.
    let bytes = unsafe { core::slice::from_raw_parts(base as *const u8, len) };
    let ix_offset = bytes
        .windows(ix.len() + 32)
        .position(|w| w[..ix.len()] == *ix && w[ix.len()..] == program_id)
        .expect("instruction tail present exactly once");
    let table_address = (base as usize + ix_offset + ix.len() + 32 + 7) & !7usize;
    (base, ix_offset, table_address - base as usize)
}

// ── 0. Entrypoint expansion compile-proof ───────────────────────────
//
// `hopper_fast_entrypoint!` gates its account resolution on the
// SIMD_0449_TABLE_ENABLED const. Expanding it here proves BOTH branches
// type-check whenever the 0321 feature (which the macro requires) is
// on — run this suite with `--features simd-0321` and with
// `--features simd-0449` to cover the const in both states.
#[cfg(feature = "simd-0321")]
mod fast_entrypoint_expands {
    use hopper_native::{Address, ProgramResult};

    hopper_native::hopper_fast_entrypoint!(process, 4);

    pub fn process(
        _program_id: &Address,
        _accounts: &[hopper_native::AccountView],
        _instruction_data: &[u8],
    ) -> ProgramResult {
        Ok(())
    }
}

// ── 1. Geometry pins (the drift alarm) ─────────────────────────────

#[test]
fn stride_walk_pins_the_loader_geometry_across_data_len_residues() {
    // Every alignment residue a data_len can produce, plus a duplicate
    // in the middle so marker framing is exercised too.
    for residue in 0..=16usize {
        let ix = [9u8, 8, 7];
        let pid = [0x77u8; 32];
        let mut frame = build_frame(
            &[
                Slot::Fresh {
                    data_len: residue,
                    lamports: 11,
                },
                Slot::Dup(0),
                Slot::Fresh {
                    data_len: 100 + residue,
                    lamports: 22,
                },
            ],
            &ix,
            pid,
            false,
        );
        let (views, got_ix, got_pid) = walk::<8>(&mut frame);
        assert_eq!(views.len(), 3, "residue {residue}");
        // Canonical identities survive the walk.
        assert_eq!(views[0].address().as_array(), &[1u8; 32]);
        assert_eq!(views[2].address().as_array(), &[3u8; 32]);
        assert_eq!(views[0].lamports(), 11);
        assert_eq!(views[2].lamports(), 22);
        assert_eq!(views[0].data_len(), residue);
        assert_eq!(views[2].data_len(), 100 + residue);
        // The duplicate resolves to the SAME canonical record.
        assert_eq!(views[1], views[0], "dup must alias slot 0");
        // The instruction tail was located exactly.
        assert_eq!(got_ix, ix, "residue {residue}");
        assert_eq!(got_pid, pid, "residue {residue}");
    }
}

#[test]
fn account_view_stays_pointer_shaped_for_the_0449_table_cast() {
    // Compile-time asserted in raw_input.rs; pinned here so a report
    // names the test if the layout ever changes.
    assert_eq!(core::mem::size_of::<AccountView<'static>>(), 8);
    assert_eq!(core::mem::align_of::<AccountView<'static>>(), 8);
}

// ── 2. SIMD-0449 table equivalence ─────────────────────────────────

#[test]
fn simd_0449_table_resolves_to_exactly_the_stride_walk_views() {
    let ix = [1u8, 2, 3, 4, 5];
    let pid = [0x66u8; 32];
    let slots = [
        Slot::Fresh {
            data_len: 32,
            lamports: 5,
        },
        Slot::Fresh {
            data_len: 7, // odd residue: table must start 8-aligned anyway
            lamports: 6,
        },
        Slot::Dup(1),
        Slot::Fresh {
            data_len: 0,
            lamports: 7,
        },
    ];
    let mut frame = build_frame(&slots, &ix, pid, true);

    // The O(n) stride walk (ground truth).
    let (walk_views, walk_ix, _) = walk::<8>(&mut frame);
    assert_eq!(walk_views.len(), 4);

    // The O(1) table path, handed the same instruction data slice the
    // SIMD-0321 r2 register would carry.
    let base = frame.as_mut_ptr();
    // Locate ix data exactly as the walk reported it (same bytes).
    assert_eq!(walk_ix, ix);
    // Reconstruct the r2-style slice: it must point INTO the frame.
    let ix_in_frame = {
        let frame_bytes = unsafe {
            // SAFETY: the words backing owns this whole region.
            core::slice::from_raw_parts(base as *const u8, frame.words.len() * 8)
        };
        let pos = frame_bytes
            .windows(ix.len() + 32)
            .position(|w| w[..ix.len()] == ix && w[ix.len()..] == pid)
            .expect("instruction tail present exactly once");
        // SAFETY: `pos + ix.len()` is in bounds of the backing.
        unsafe { core::slice::from_raw_parts(base.add(pos) as *const u8, ix.len()) }
    };

    // SAFETY: the fixture appended a well-formed table (the exact
    // serialization the SIMD specifies), and `ix_in_frame` points at
    // the in-frame instruction data.
    let table_views = unsafe { deserialize_accounts_0449(base, ix_in_frame) };

    assert_eq!(table_views.len(), walk_views.len());
    for (i, (t, w)) in table_views.iter().zip(walk_views.iter()).enumerate() {
        assert_eq!(t, w, "slot {i}: table view must alias the walk view");
    }
    // Duplicate pre-deduplication: slot 2's table entry IS slot 1's.
    assert_eq!(table_views[2], table_views[1]);
}

// -- 3. Checked conformance decoder -----------------------------------------

#[test]
fn checked_0449_decoder_matches_the_stride_walk_for_every_alignment_residue() {
    for residue in 0..=16usize {
        let ix = [0x31, residue as u8, 0x7F];
        let pid = [0x42; 32];
        let slots = [
            Slot::Fresh {
                data_len: residue,
                lamports: 10,
            },
            Slot::Dup(0),
            Slot::Fresh {
                data_len: 33 + residue,
                lamports: 20,
            },
        ];
        let mut frame = build_frame(&slots, &ix, pid, true);
        let (base, ix_offset, _) = locate_0449_tail(&mut frame, &ix, pid);
        let input_len = frame.words.len() * 8;
        // SAFETY: `ix_offset..ix_offset + ix.len()` is the located in-frame
        // instruction payload and the frame remains alive for the call.
        let ix_in_frame = unsafe { core::slice::from_raw_parts(base.add(ix_offset), ix.len()) };
        const UNINIT: MaybeUninit<AccountView<'static>> = MaybeUninit::uninit();
        let mut out = [UNINIT; 8];
        // SAFETY: the fixture is a complete loader frame with a canonical
        // pointer table, and `input_len` covers its word-aligned backing.
        let (got_pid, count, got_ix) = unsafe {
            deserialize_accounts_0449_checked(base, input_len, &mut out, ix_in_frame)
        }
        .expect("canonical table validates");
        assert_eq!(count, 3, "residue {residue}");
        assert_eq!(got_pid.as_array(), &pid);
        assert_eq!(got_ix, ix);
        let first = unsafe { out[0].assume_init_ref() };
        let duplicate = unsafe { out[1].assume_init_ref() };
        let last = unsafe { out[2].assume_init_ref() };
        assert_eq!(first, duplicate, "residue {residue}: duplicate aliases");
        assert_eq!(first.data_len(), residue);
        assert_eq!(last.data_len(), 33 + residue);
    }
}

#[test]
fn checked_0449_decoder_rejects_equal_but_out_of_frame_instruction_bytes() {
    let ix = [1, 2, 3];
    let pid = [7; 32];
    let mut frame = build_frame(
        &[Slot::Fresh {
            data_len: 8,
            lamports: 1,
        }],
        &ix,
        pid,
        true,
    );
    let base = frame.as_mut_ptr();
    let input_len = frame.words.len() * 8;
    const UNINIT: MaybeUninit<AccountView<'static>> = MaybeUninit::uninit();
    let mut out = [UNINIT; 2];
    let foreign = ix;
    let err = unsafe {
        deserialize_accounts_0449_checked(base, input_len, &mut out, &foreign)
    }
    .unwrap_err();
    assert_eq!(err, DirectMappingError::InstructionDataMismatch);
}

#[test]
fn checked_0449_decoder_rejects_out_of_region_and_noncanonical_pointers() {
    let ix = [9, 9];
    let pid = [8; 32];
    let slots = [
        Slot::Fresh {
            data_len: 4,
            lamports: 1,
        },
        Slot::Fresh {
            data_len: 5,
            lamports: 2,
        },
        Slot::Dup(0),
    ];

    // An arbitrary external address is rejected before any AccountView is
    // materialized.
    let mut frame = build_frame(&slots, &ix, pid, true);
    let (base, ix_offset, table_offset) = locate_0449_tail(&mut frame, &ix, pid);
    let input_len = frame.words.len() * 8;
    unsafe {
        core::ptr::write_unaligned(base.add(table_offset) as *mut u64, 0);
    }
    let ix_in_frame = unsafe { core::slice::from_raw_parts(base.add(ix_offset), ix.len()) };
    const UNINIT: MaybeUninit<AccountView<'static>> = MaybeUninit::uninit();
    let mut out = [UNINIT; 4];
    let err = unsafe {
        deserialize_accounts_0449_checked(base, input_len, &mut out, ix_in_frame)
    }
    .unwrap_err();
    assert_eq!(err, DirectMappingError::PointerOutOfBounds { slot: 0 });

    // A valid canonical pointer for the WRONG slot is in-bounds and aligned,
    // but still rejected. This pins identity, not merely pointer hygiene.
    let mut frame = build_frame(&slots, &ix, pid, true);
    let (base, ix_offset, table_offset) = locate_0449_tail(&mut frame, &ix, pid);
    let input_len = frame.words.len() * 8;
    let wrong = base as usize + frame.canonical_offsets[1];
    unsafe {
        core::ptr::write_unaligned(base.add(table_offset) as *mut u64, wrong as u64);
    }
    let ix_in_frame = unsafe { core::slice::from_raw_parts(base.add(ix_offset), ix.len()) };
    let mut out = [UNINIT; 4];
    let err = unsafe {
        deserialize_accounts_0449_checked(base, input_len, &mut out, ix_in_frame)
    }
    .unwrap_err();
    assert_eq!(err, DirectMappingError::NonCanonicalPointer { slot: 0 });

    // A duplicate slot must point to its canonical earlier record, never to
    // another in-frame canonical account.
    let mut frame = build_frame(&slots, &ix, pid, true);
    let (base, ix_offset, table_offset) = locate_0449_tail(&mut frame, &ix, pid);
    let input_len = frame.words.len() * 8;
    let wrong = base as usize + frame.canonical_offsets[1];
    unsafe {
        core::ptr::write_unaligned(
            base.add(table_offset + 2 * core::mem::size_of::<u64>()) as *mut u64,
            wrong as u64,
        );
    }
    let ix_in_frame = unsafe { core::slice::from_raw_parts(base.add(ix_offset), ix.len()) };
    let mut out = [UNINIT; 4];
    let err = unsafe {
        deserialize_accounts_0449_checked(base, input_len, &mut out, ix_in_frame)
    }
    .unwrap_err();
    assert_eq!(err, DirectMappingError::NonCanonicalPointer { slot: 2 });
}

#[test]
fn checked_0449_decoder_rejects_malformed_duplicates_and_truncation() {
    let ix = [5];
    let pid = [6; 32];
    let mut frame = build_frame(
        &[Slot::Fresh {
            data_len: 0,
            lamports: 1,
        }],
        &ix,
        pid,
        true,
    );
    let (base, ix_offset, _) = locate_0449_tail(&mut frame, &ix, pid);
    let input_len = frame.words.len() * 8;
    // Slot zero cannot be a duplicate of slot zero.
    unsafe {
        *base.add(8) = 0;
    }
    let ix_in_frame = unsafe { core::slice::from_raw_parts(base.add(ix_offset), ix.len()) };
    const UNINIT: MaybeUninit<AccountView<'static>> = MaybeUninit::uninit();
    let mut out = [UNINIT; 2];
    let err = unsafe {
        deserialize_accounts_0449_checked(base, input_len, &mut out, ix_in_frame)
    }
    .unwrap_err();
    assert_eq!(
        err,
        DirectMappingError::MalformedDuplicate {
            slot: 0,
            duplicate_of: 0
        }
    );

    let mut frame = build_frame(
        &[Slot::Fresh {
            data_len: 0,
            lamports: 1,
        }],
        &ix,
        pid,
        true,
    );
    let (base, ix_offset, table_offset) = locate_0449_tail(&mut frame, &ix, pid);
    let ix_in_frame = unsafe { core::slice::from_raw_parts(base.add(ix_offset), ix.len()) };
    let mut out = [UNINIT; 2];
    let err = unsafe {
        deserialize_accounts_0449_checked(base, table_offset, &mut out, ix_in_frame)
    }
    .unwrap_err();
    assert_eq!(err, DirectMappingError::TruncatedInput);
}

#[test]
fn checked_table_and_stride_views_have_identical_mutation_semantics() {
    let ix = [0xA4, 0x01];
    let pid = [0x91; 32];
    let mut frame = build_frame(
        &[
            Slot::Fresh {
                data_len: 32,
                lamports: 50,
            },
            Slot::Dup(0),
        ],
        &ix,
        pid,
        true,
    );
    let (walk_views, _, _) = walk::<4>(&mut frame);
    let (base, ix_offset, _) = locate_0449_tail(&mut frame, &ix, pid);
    let input_len = frame.words.len() * 8;
    let ix_in_frame = unsafe { core::slice::from_raw_parts(base.add(ix_offset), ix.len()) };
    const UNINIT: MaybeUninit<AccountView<'static>> = MaybeUninit::uninit();
    let mut out = [UNINIT; 4];
    let (_, count, _) = unsafe {
        deserialize_accounts_0449_checked(base, input_len, &mut out, ix_in_frame)
    }
    .unwrap();
    assert_eq!(count, 2);
    let table_first = unsafe { out[0].assume_init_ref() };
    let table_duplicate = unsafe { out[1].assume_init_ref() };

    // Last-byte write through the table path is visible through both the
    // legacy-walk view and the duplicate. This pins payload alias semantics,
    // not just header pointer equality.
    {
        let mut byte = table_first.segment_mut::<u8>(31, 1).unwrap();
        *byte = 0xCD;
    }
    assert_eq!(walk_views[0].try_borrow().unwrap()[31], 0xCD);
    assert_eq!(table_duplicate.try_borrow().unwrap()[31], 0xCD);

    // Header transitions also share one canonical RuntimeAccount.
    walk_views[0].resize(16).unwrap();
    assert_eq!(table_first.data_len(), 16);
    table_duplicate.set_lamports(77);
    assert_eq!(walk_views[0].lamports(), 77);
    let new_owner = hopper_native::Address::new_from_array([0xE1; 32]);
    unsafe {
        table_first.assign(&new_owner);
    }
    assert_eq!(walk_views[0].read_owner(), new_owner);
}
