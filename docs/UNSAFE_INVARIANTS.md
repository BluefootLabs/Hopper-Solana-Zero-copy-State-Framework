# Unsafe Invariants Ledger - Trust Posture Document

## Trust Posture

Hopper is a safety-layered framework that deliberately uses `unsafe` for the
operations where Rust's ownership system cannot express the invariants we need
(pointer casts onto account byte slices, zero-copy overlays, CPI invocation).
Every other layer of the framework (header validation, fingerprint checking,
tiered loading, frame borrow tracking, validation graphs) exists to make the
unsafe core as small and auditable as possible.

**Design commitment**: unsafe is never used for convenience. It is used only
when a safe alternative would require allocation, serialization, or loss of
the zero-copy property that makes Hopper competitive.

**Audit scope**: the unsafe surface is confined to five modules in
`hopper-core` (abi, account, check, cpi, collections) and two modules in
`hopper-solana` (token readers, CPI guards). Everything else is safe Rust.

Every `unsafe` block in Hopper, its justification, and the invariants
that must hold. Organized by module boundary.

## Trust Summary

Hopper's unsafe surface is deliberately narrow and follows three foundational rules:

1. **All overlay targets are alignment-1.** No pointer cast in the codebase produces a reference with `align > 1`. This eliminates alignment UB entirely.
2. **All casts are bounds-checked.** Every `pod_from_bytes` / `overlay_at` / `read_unaligned` call is preceded by a length check against `T::SIZE` or explicit offset arithmetic.
3. **Aliasing is structurally prevented.** Mutable borrows flow through `&mut self` (compile-time) or frame-level bitmask tracking (runtime). No two mutable references can alias the same account data.

### What tests prove it

| Invariant | Test Suite | File |
|---|---|---|
| Pod boundary rejection | 38 tests | `tests/unsafe_boundary_tests.rs` |
| Overlay checked/unchecked parity | 24 tests | `tests/overlay_equivalence_tests.rs` |
| Compat regression & receipt wire format | 26 tests | `tests/compat_regression_tests.rs` |
| Property-based ABI roundtrip | 36 tests | `tests/property_tests.rs` |
| CPI guard, collections, registry, validation | 96 tests | `tests/trust_tests.rs` |

### What callers must guarantee

| API | Caller Obligation |
|---|---|
| `unsafe impl Pod for T` | `T` is `#[repr(C)]` or `#[repr(transparent)]`, all fields are `[u8; N]` or Pod, `align_of::<T>() == 1` |
| `cast_unchecked` / `cast_unchecked_mut` | `data.len() >= size_of::<T>()`. No concurrent aliasing. |
| `hopper_layout!` `load_unchecked` | Account data is valid for the layout. Caller accepts all risk. |
| `MaybeUninit` transmute in CPI builders | All `ACCTS` slots initialized via `add_account()` before `invoke()` |

---

## Global Guarantees

1. **`#![deny(unsafe_op_in_unsafe_fn)]`** -- enforced in `hopper-core` and `hopper-solana`. All unsafe operations must be explicitly wrapped even inside `unsafe fn`.
2. **Pod trait** -- `unsafe trait Pod: Copy + Sized` requires `align_of == 1` and all bit patterns valid. Every `unsafe impl Pod` is for types whose fields are `[u8; N]` or nested Pod types under `#[repr(C)]`/`#[repr(transparent)]`.
3. **All pointer casts target align-1 types.** No pointer cast in the codebase produces a reference to a type with alignment > 1.

---

## hopper-core::abi

### Wire types (`integers.rs`, `boolean.rs`)

| Line(s) | Construct | Invariant |
|---|---|---|
| `unsafe impl WireType` | Trait impl per wire type | Type is `#[repr(transparent)]` over `[u8; N]`, `align == 1`, `size == N` (compile-time asserted). All bit patterns valid. |
| `unsafe impl Pod` | Trait impl per wire type | Same as above. |

### `typed_address.rs`

| Line | Construct | Invariant |
|---|---|---|
| 61 | `unsafe impl Pod for TypedAddress<T>` | `#[repr(transparent)]` over `[u8; 32]`. PhantomData is ZST. Size == 32, align == 1 (compile-time asserted). |
| 99 | `&*(account.address() as *const Address as *const [u8; 32])` | `hopper_native::Address` is `[u8; 32]` (same repr). Read-only, no-alloc. |
| 198 | `unsafe impl Pod for UntypedAddress` | `#[repr(transparent)]` over `[u8; 32]`. |

### `field_ref.rs`

| Line | Construct | Invariant |
|---|---|---|
| 88 | `&*(self.data.as_ptr() as *const [u8; 32])` | Slice length checked ≥ 32 before cast. Target type is `[u8; 32]`, align 1. |

---

## hopper-core::account

### `pod.rs`

| Line | Construct | Invariant |
|---|---|---|
| 13 | `pub unsafe trait Pod` | Marker trait. Implementors guarantee align-1, all bit patterns valid. |
| 16-17 | `unsafe impl Pod for u8` / `[u8; 32]` | Trivially safe. |
| 39 | `pod_from_bytes`: `&*(data.as_ptr() as *const T)` | Size checked: `data.len() >= T::SIZE`. T: Pod guarantees align-1. No aliasing: immutable borrow. |
| 54 | `pod_from_bytes_mut`: `&mut *(data.as_mut_ptr() as *mut T)` | Size checked. T: Pod. Caller must ensure exclusive access. |
| 64 | `pod_read`: `read_unaligned` | Size checked. T: Pod. Returns by value, no alias concern. |
| 74 | `pod_write`: `write_unaligned` | Size checked. T: Pod. Caller must hold `&mut [u8]`. |

### `header.rs`

| Line | Construct | Invariant |
|---|---|---|
| 49 | `unsafe impl Pod for AccountHeader` | `#[repr(C)]` of all byte-array fields. Size == 16, align == 1 (asserted). |

### `verified.rs`

| Line | Construct | Invariant |
|---|---|---|
| 36 | `VerifiedAccount::get()`: `&*(data.as_ptr() as *const T)` | Size validated at construction (`data.len() >= T::SIZE`). T: Pod. Immutable. |
| 99 | `overlay_at`: `&*(data.as_ptr().add(offset) as *const U)` | Bounds checked: `offset + U::SIZE <= data.len()`. U: Pod. |
| 126 | `VerifiedAccountMut::get()` | Same as VerifiedAccount::get(). |
| 133 | `get_mut()`: `&mut *(data.as_mut_ptr() as *mut T)` | Size validated. Exclusive access via `&mut self`. |
| 180 | `overlay_at` (mut variant) | Bounds checked. |
| 190 | `overlay_at_mut` | Bounds checked. Exclusive access via `&mut self`. |

### `reader.rs`

| Line | Construct | Invariant |
|---|---|---|
| 42 | Header overlay cast | Data length checked ≥ `HEADER_LEN` at construction. |
| 114 | Address overlay at offset | Bounds checked. |

### `segment.rs`

| Line | Construct | Invariant |
|---|---|---|
| 37 | `unsafe impl Pod for SegmentDescriptor` | `#[repr(C)]`, all byte fields. |
| 147, 175, 246, 257, 310, 321 | Pointer offset casts | All bounds-checked before cast. Target types are Pod (align-1). |

### `registry.rs`

| Line | Construct | Invariant |
|---|---|---|
| 226, 236, 268, 320, 418, 439, 449, 489 | Pointer offset casts | All preceded by bounds checks against `self.data.len()`. Target types: `SegmentEntry` (Pod, align-1), or generic T: Pod. |

### `cursor.rs`

| Line | Construct | Invariant |
|---|---|---|
| 135 | Address cast at cursor position | Position + 32 <= data.len() checked. |

### `lifecycle.rs`

No raw unsafe blocks. Uses `hopper_runtime::AccountView` safe APIs.

---

## hopper-core::check

### `mod.rs`

| Line | Construct | Invariant |
|---|---|---|
| 142 | `keys_eq_fast`: `read_unaligned` x 4 | Input is `&[u8; 32]`, always valid for u64 reads at offsets 0/8/16/24. |
| 159 | `is_zero_address`: `read_unaligned` x 4 | Same as above. |
| 179 | Address cast in `check_has_one` | `hopper_native::Address` is `[u8; 32]`. |
| 239 | `borrow_unchecked()` in `check_account` | Immutable borrow for validation only. No conflicting mutable borrows at this point (called before execution phase). |
| 353 | `borrow_unchecked()` in `check_discriminator` (via macro) | Same pattern. |
| 405 | `account.owner()` in `check_owner_multi` | AccountView's unsafe owner() reads the owner field. No alias concern (read-only). |

### `fast.rs`

| Line | Construct | Invariant |
|---|---|---|
| 71-82 | `read_account_header` | Reads first 4 bytes of `RuntimeAccount` via pointer dereference. Relies on `AccountView` being `#[repr(C)]` with first field = pointer to RuntimeAccount base. Gated to `target_os = "solana"` only. Preconditions: SVM guarantees valid input buffer layout. |
| 103 | Call to `read_account_header` | Within `check_account_fast`, called on SVM-provided `AccountView`. |

### `modifier.rs`

| Line | Construct | Invariant |
|---|---|---|
| 160 | `borrow_unchecked()` in `Account<T>::from_account` | Owner check passed. Frame-level borrow tracking prevents conflicting mutable borrows. |
| 179 | `borrow_unchecked_mut()` in `AccountMut<T>::from_account` | Owner + writable checks passed. Caller ensures exclusive access at frame level. |

---

## hopper-core::cpi

### `mod.rs`

| Line | Construct | Invariant |
|---|---|---|
| 58, 207 | `MaybeUninit::uninit().assume_init()` (array of MaybeUninit) | Creating an array of `MaybeUninit<&AccountView>` from uninit is sound: `MaybeUninit<T>` does not require initialization. Added slots are initialized via `add_account` before invoke. |
| 76, 224 | Address cast from `AccountView::address()` | `Address` is `[u8; 32]`. Read-only cast. |
| 122, 260 | View transmute from `MaybeUninit` array | All `ACCTS` slots initialized via `add_account` (enforced by `debug_assert_eq!(acct_cursor, ACCTS)`). The transmute from `[MaybeUninit<&T>; N]` to `[&T; N]` is sound when all N elements are initialized. |
| 128, 265 | `core::mem::zeroed()` for `InstructionAccount` array | `InstructionAccount` has no invalid bit patterns (contains `&[u8; 32]` pointer + 2 bools). Zeroed pointers are overwritten before use. |
| 150, 154, 285, 287 | `core::mem::zeroed()` for Signer/Seed buffers | Same pattern. All used slots are written before `invoke_signed_unchecked`. |

---

## hopper-core::collections

All collections follow the same pattern: bounds-checked pointer arithmetic on `&[u8]` / `&mut [u8]` slices, with target types that are Pod (align-1).

| Module | Pattern | Invariant |
|---|---|---|
| `fixed_vec` | `read_unaligned`, overlay casts | Count/capacity validated. Offset arithmetic checked against data.len(). |
| `ring_buffer` | `write_unaligned`, overlay casts | Head/count maintained modulo capacity. Offsets checked. |
| `slot_map` | Overlay casts with generation counter | Slot index validated. |
| `bit_set` | None (all byte-level) | N/A |
| `sorted_vec` | `read_unaligned`, `write_unaligned`, `copy_within` | Count validated, offsets checked. `copy_within` uses `ptr::copy` for overlapping regions. |
| `journal` | `write_unaligned`, `read_unaligned` | Cursor wraps within capacity. Bounds checked. |
| `slab` | Offset casts, `read_unaligned` | Bitmap allocation tracking. Slot offset validated against data length. |
| `packed_map` | `read_unaligned`, `write_unaligned` | Count validated, entry size arithmetic checked. |

---

## hopper-core::frame

### `phase.rs`

| Line | Construct | Invariant |
|---|---|---|
| `borrow_mut` | `borrow_unchecked_mut()` via `ExecutionContext` | Runtime borrow tracking via u64 bitmask (`mutable_borrows`). Each bit corresponds to an account index. `AccountBorrowFailed` returned on double-mutable-borrow. |
| `borrow` | `borrow_unchecked()` | Immutable borrow. No conflict tracking needed (follows Rust's shared-borrow model). |

---

## hopper-macros

### `hopper_layout!`

| Construct | Invariant |
|---|---|
| `unsafe impl Pod for $name` | Generated struct is `#[repr(C)]` over alignment-1 fields. Compile-time assertions enforce `size_of == LEN` and `align_of == 1`. |
| `borrow_unchecked()` / `borrow_unchecked_mut()` in load functions | Protected by tiered validation: T1 checks owner + disc + version + layout_id + size before borrow. T2 checks owner + layout_id + size. |
| `load_unchecked` | Explicitly marked `unsafe fn`. Caller assumes all risk. |
| `load_unverified` | Size checked. Returns overlay even without full validation (tier 5 for indexers). |

### `hopper_check!`

| Construct | Invariant |
|---|---|
| `borrow_unchecked()` in disc/size arms | Immutable borrow for validation reads. Called during resolve/validate phase (before any mutable borrows). |

---

## hopper-solana

### `token.rs`, `mint.rs`

| Line | Construct | Invariant |
|---|---|---|
| All pointer casts | Data length >= `TOKEN_ACCOUNT_LEN` or `MINT_LEN` checked before cast. Target: `Address` (align 1). |

### `cpi_guard.rs`

| Line | Construct | Invariant |
|---|---|---|
| 71 | `instructions_sysvar.borrow_unchecked()` | Used to read the Instructions sysvar. Immutable, read-only. |

### `typed_cpi.rs`

| Line | Construct | Invariant |
|---|---|---|
| 298-299 | `borrow_unchecked()` in `checked_token_transfer` | Read-only borrows to compare mint fields before CPI. No conflicting mutable access at this point. |

---

## Audit Checklist

For any new `unsafe` added to the codebase, verify:

- [ ] Bounds check precedes every pointer offset/cast
- [ ] Target type is Pod (align-1, all bits valid)
- [ ] `// SAFETY:` comment present and accurate
- [ ] Mutable borrows tracked by frame bitmask or exclusive `&mut` access
- [ ] No UB on the off-chain (non-SVM) path
- [ ] `target_os = "solana"` gate if relying on SVM runtime layout

---

## Unsafe Review Checklist (for auditors)

When reviewing Hopper code (or code that depends on Hopper), walk through
these questions for every `unsafe` block:

1. **Is the target type alignment-1?** Every Pod type in Hopper is
   `#[repr(C)]` or `#[repr(transparent)]` with all fields being `[u8; N]`
   or nested Pod types. If a new type is introduced, verify `align_of == 1`
   with a compile-time assertion.

2. **Is the slice length checked before the cast?** Every `pod_from_bytes`,
   `overlay_at`, and manual pointer cast must be preceded by
   `data.len() >= T::SIZE` or equivalent bounds arithmetic.

3. **Is aliasing structurally prevented?** Mutable access must flow through
   either `&mut self` (compile-time) or the frame-level borrow bitmask
   (runtime). No two mutable references should be able to alias the same
   account data within a single instruction.

4. **Does it work off-chain?** Code gated to `target_os = "solana"` may
   assume SVM account layout. Verify that the non-SVM path either provides
   equivalent safety or is unreachable.

5. **Is the `// SAFETY:` comment accurate and complete?** It must state
   the precondition, why it holds, and what would go wrong if it didn't.

6. **Are MaybeUninit uses fully initialized before read?** CPI builders
   use `MaybeUninit` arrays. Verify that `add_account()` is called for
   every slot before `invoke()`.

7. **Does the test suite cover the boundary?** Each unsafe boundary should
   have at least one test that exercises the happy path and one that
   exercises the rejection path (wrong size, wrong alignment, etc.).

---

## Test Coverage by Danger Zone

Every module with `unsafe` blocks has corresponding tests that exercise the
invariant boundaries. This table maps each risk area to its test coverage.

| Module | Risk | Key Invariant | Test Coverage |
|---|---|---|---|
| `abi::integers` | Wire type soundness | `align == 1`, `size == WIRE_SIZE` | Compile-time assertions + `prop_wire_*` property tests |
| `abi::typed_address` | Address cast soundness | `Address` is `[u8; 32]`, read-only | `prop_typed_address_*` property tests |
| `abi::fingerprint` | Deterministic hashing | SHA-256 prefix must change with schema | `fingerprint_*` golden tests in trust_tests |
| `account::pod` | Overlay cast bounds | `data.len() >= T::SIZE` before cast | `prop_pod_*` + compile-time `size_of` assertions |
| `account::segment` | Segment offset math | Bounds checked before every cast | `segment_*` trust tests + property tests |
| `account::registry` | Registry pointer offset | All offsets validated against `data.len()` | `registry_*` trust tests |
| `check::mod` | Sysvar instruction parsing | Offset table + per-ix layout fidelity | `cpi_guard_*` + `sysvar_parse_*` golden tests (with 0/1/N account metas) |
| `check::fast` | RuntimeAccount header read | SVM-only, gated to `target_os = "solana"` | Relies on SVM runtime guarantees; untestable off-chain |
| `cpi::mod` | MaybeUninit transmute | All `ACCTS` slots initialized before transmute | `debug_assert_eq!(acct_cursor, ACCTS)` + off-chain no-op path |
| `cpi::mod` | CPI builder zeroed data | `InstructionAccount` overwritten before invoke | Off-chain path returns `Ok(())`, SVM path exercises full path |
| `collections::journal` | Circular wrap + `copy_nonoverlapping` | Head wraps within capacity, bounds checked | `journal_*` trust tests: strict/circular, wrap-many, ordering, latest, out-of-bounds |
| `collections::slab` | Bitmap + offset arithmetic | Slot index validated, bounds checked | `slab_*` trust tests: alloc/free cycle, double-free reject, full/realloc |
| `collections::fixed_vec` | `read_unaligned` overlay | Count/capacity validated | `fixed_vec_*` unit tests |
| `collections::ring_buffer` | `write_unaligned` overlay | Head/count modulo capacity | `ring_buffer_*` unit tests |
| `collections::sorted_vec` | `ptr::copy` for insert/remove | Count validated, offsets checked | `sorted_vec_*` trust + property tests |
| `frame::phase` | Borrow tracking bitmask | u64 bitmask prevents double-mutable-borrow | `frame_*` property tests |
| `hopper-macros` | `hopper_layout!` Pod derivation | Compile-time `size_of == LEN`, `align_of == 1` | Every generated type gets static assertions; used in all test layouts |
| `hopper-solana::token` | Token account overlay | `data.len() >= TOKEN_ACCOUNT_LEN` checked | `token_*` integration tests |
| `hopper-solana::cpi_guard` | Instructions sysvar borrow | Immutable read for validation | `cpi_guard_*` trust tests (12 tests covering all guard variants) |
| `receipt` | Fingerprint hashing of account data | FNV-1a deterministic, before/after tracked | `receipt_*` trust tests (12 tests) + `prop_receipt_*` property tests (9 tests) |

### Boundary Test Files

The following dedicated test files exercise unsafe boundaries directly:

- **`tests/unsafe_boundary_tests.rs`** - Pod from undersized/empty/oversized buffers, VerifiedAccount rejection, overlay-at OOB rejection, `usize::MAX` overflow check, header wire layout verification, segment descriptor boundary conditions, wire type roundtrips, unchecked cast parity.
- **`tests/overlay_equivalence_tests.rs`** - `pod_from_bytes` vs `pod_read` value equivalence, `VerifiedAccount::get()` vs raw pod parity, `overlay_at` vs manual slice pod parity, `cast_unchecked` vs checked parity, mutable write-through equivalence, wire type overlay vs raw bytes, header overlay vs constructor.
- **`tests/compat_regression_tests.rs`** - Append-safe addition detection, forbidden field rename/resize, field removal as breaking, `compare_fields` report accuracy, `is_backward_readable` / `requires_migration` correctness, receipt wire format encode/decode roundtrip, Phase/CompatImpact enum roundtrips, segment/field mask roundtrip, reserved byte verification.
