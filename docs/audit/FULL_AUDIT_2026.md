# Hopper Full Line-by-Line Audit — 2026

Exhaustive whole-repo audit: every line of every Rust file. Multi-session;
this document is the durable state. Each batch marks files audited, findings
are logged with `file:line`, severity, and disposition. Cross-referenced
against the competitor sources at `E:\Frameworks\{anchor,quasar,pinocchio}`
where their approach suggests a latent issue or a better pattern.

## Severity scale

- **P0** — soundness: UB, memory safety, wrong on-chain validation.
- **P1** — correctness: wrong results, broken invariants, API misbehavior.
- **P2** — footgun: public API that invites misuse; missing check a caller
  would reasonably assume exists.
- **P3** — cleanup: dead code, duplication, inefficiency, unidiomatic.
- **DOC** — docs/comments contradict behavior.

Disposition: `fixed@<commit>` | `accepted-risk (<why>)` | `open`.

## Progress checklist

Order is highest-risk-first. A crate is checked only when every file in it has
been read line-by-line and its findings logged.

### Batch 1 — hopper-native (33 files, ~9.2k lines) — COMPLETE ✅

- [x] raw_input.rs (re-audited post-hardening; see findings)
- [x] entrypoint.rs
- [x] raw_account.rs
- [x] address.rs (moved up: needed for entrypoint soundness proof)
- [x] instruction.rs (re-audited post-hardening)
- [x] account_view.rs
- [ ] raw.rs
- [x] borrow.rs
- [x] mem.rs
- [x] pod.rs
- [x] project.rs
- [x] lens.rs (moved up: shares the projection surface)
- [x] wire.rs
- [x] pda.rs (+ the runtime pda.rs host fallback it exposed)
- [x] cpi.rs
- [x] syscalls.rs
- [x] lazy.rs (Send/Sync gating fixed)
- [x] sha256.rs / hash.rs
- [x] batch.rs / budget.rs / capability.rs / error.rs / expert.rs /
      introspect.rs / lib.rs / log.rs / return_data.rs / safe.rs / system.rs /
      sysvar.rs / token.rs / verify.rs

### Batch 2 — hopper-runtime (46 files, ~20.3k lines) — IN PROGRESS

- [x] segment_borrow.rs (flagship differentiator — see findings)
- [x] pda.rs (fixed with the Batch 1 PDA hardening: host fallback panic)
- [~] account.rs (large; loaders/compact/cross-program/close/check surfaces
      audited during this session's fixes — remaining: wrappers/init/realloc
      internals)
- [x] borrow.rs (host-repr provenance fix — see findings)
- [x] memory.rs
- [x] zerocopy.rs
- [x] segment_lease.rs (+ account.rs::split_segments_mut ordering fix)
- [x] tail.rs
- [x] layout.rs
- [x] compact.rs (fully read + extended with `CompactDynamicLayout` and
      tests during this session's compact-dynamic feature work)
- [x] account_wrappers.rs
- [ ] compact.rs, tail.rs, segment_lease.rs, layout.rs,
      account_wrappers.rs, borrow_registry.rs, address.rs, cpi.rs,
      token.rs, crypto.rs, context.rs, policy.rs, native_boundary.rs,
      syscalls.rs, lib.rs, foreign.rs, instruction.rs, interop.rs,
      pod.rs, remaining files

#### Batch 2 findings

- **verified sound** `hopper-runtime/segment_borrow.rs` — the segment
  registry (the disjoint-borrow differentiator no competitor has):
  `ranges_overlap` widens to u64 so `offset + size` cannot wrap;
  fingerprint-then-verify identity means collisions cost one extra compare
  and can never manufacture or miss a conflict; the conflict matrix admits
  only Read+Read on overlap; `MaybeUninit` slots are strictly len-gated;
  `release` removes by full `(key, offset, size, kind)` identity with
  swap-remove; `release_last_registered` fast-pops with exact-removal
  fallback; duplicate identical writes are rejected by the Write+Write rule.
  In-file Kani proofs (overlap symmetry et al.) and unit tests pin the
  invariants. **P3 open:** `register_guard` returns a guard holding
  `&mut registry`, so two RAII guards cannot coexist — simultaneous disjoint
  segments go through the `segment_lease`/`split_segments_mut` path; the
  guard docs should say so explicitly.
- **P2 fixed (this commit)** `hopper-runtime/borrow.rs::RefMut::from_backend`
  (host repr) — derived its write pointer from a **shared** reborrow
  (`(&*inner as *const [u8]).cast_mut()`), giving the pointer shared
  provenance; subsequent writes through it are UB under the aliasing model
  (Miri flags this pattern). Now takes the pointer from a mutable reborrow
  (`&mut *inner`). Host-only repr; the Solana repr extracts raw parts and was
  unaffected.
- **verified sound** `hopper-runtime/borrow.rs` — dual-repr guards: Solana
  `{ptr, state}` with build-enforced 2-word size asserts, host
  `{ptr, guard, token}` where the backend guard + alias token keep the
  pointee alive for `deref`. Drop paths mirror the native state machine
  (decrement / restore NOT_BORROWED); `project` transfers release ownership
  without double-release (`mem::forget` on the Solana arm, destructuring on
  the host arm which has no Drop impl); `slice` is overflow- and
  bounds-checked. **P3 open:** `slice_from` panics (Rust slicing) on
  offset > len rather than returning `Err` — document or route through the
  checked `slice`.
- **verified sound** `hopper-runtime/memory.rs` — raw syscall wrappers with
  documented contracts; safe wrappers bounds-check (`copy_bytes`,
  overflow-checked `move_within` with overlap-safe memmove, `fill_bytes`,
  length-aware `compare_bytes`); unit-tested.
- **verified sound** `hopper-runtime/zerocopy.rs` — the sealed unified trait
  stack: `ZeroCopy: Pod + 'static + HopperZeroCopySealed` with the seal in a
  doc-hidden module so a hand-rolled `unsafe impl Pod` cannot pick up
  `ZeroCopy` for free (pinned by the `SneakyBypass` compile-fail test);
  `WireLayout`/`AccountLayout` blankets coherent; the
  `LAYOUT_ID → WIRE_FINGERPRINT` LE reinterpretation documented. All unsafe
  here is trait contracts, not operations.
- **P2 fixed (this commit)** `hopper-runtime/account.rs::split_segments_mut`
  — derived the leases' shared raw registry pointer (`reg_ptr`) **before**
  the phase-1 registrations that go through the parent `&mut borrows`. Under
  Stacked Borrows, using the parent `&mut` invalidates the earlier-derived
  raw, so the rollback paths and every lease drop wrote through a dead
  pointer (host-Miri-flaggable; single-threaded SVM masks it). Reordered:
  phase-1 registration/rollback flows through the `&mut`
  (`release_registered` now takes `&mut SegmentBorrowRegistry`), and the raw
  pointer is derived once as the **final** `&mut` use, so every subsequent
  registry access (lease drops) is a child of that single live derivation.
- **verified sound** `hopper-runtime/segment_lease.rs` — the RAII lease
  layer: `SegmentLease` pins the registry lifetime via
  `PhantomData<&'a mut _>` and releases by exact identity on drop;
  `SegRef`/`SegRefMut` pair the typed guard with the lease so registry
  entries live exactly as long as the data guard (fixing the audit's
  "sticky ledger"). The **sequential** `segment_ref`/`segment_mut` path is
  provenance-clean by construction: the raw is derived in
  `SegmentLease::new` *after* the registration's last `&mut` use, and the
  returned guard holds `'a` so no new `&mut` can exist while the lease
  lives. `SegmentsMut::all_mut` hands out N `&mut T` that are pairwise
  disjoint (registry-proven at construction) inside one exclusive byte
  borrow — the generalized `split_at_mut` claim holds.
- **verified sound** `hopper-runtime/tail.rs` — the dynamic-tail system:
  every helper (`read_tail_len`, `tail_payload`, `tail_capacity`,
  `borrow_bounded_str`, `borrow_address_slice`, `read_tail`, `write_tail`,
  `write_tail_payload`) is bounds- and overflow-checked; `read_tail`
  enforces exact prefix consumption so trailing bytes are a fail-closed
  malformed-encoding error; `borrow_address_slice`'s slice cast is
  layout-safe (`Address` is align-1 `repr(transparent)`, `checked_mul`
  length); `BoundedString`/`BoundedVec` codecs validate `len ≤ N` on decode
  and zero removed slots on `pop`/`clear`/`remove_first` (the audit's tail
  shrink-hygiene ask, delivered at element level). Notes: (1) an external
  `TailCodec` impl returning `consumed > input.len()` makes the next
  `BoundedVec` decode iteration panic via slice indexing — safe abort, never
  UB; worth a line in the trait docs. (2) `write_tail` writes the payload
  before updating the length prefix, so a mid-encode error leaves the old
  prefix over a partial payload — unobservable on-chain (transaction
  atomicity rolls the account back) and only visible to host harnesses that
  ignore the error.
- **verified sound** `hopper-runtime/layout.rs` — the 16-byte header
  contract: `HopperHeader` is `repr(C, packed)` (align-1 casts sound;
  packed fields correctly copied by value, never referenced); every reader
  (`read_disc/version/flags/layout_id/schema_epoch`) bounds-checks;
  `write_header_with_epoch` bounds-checks and zeroes flags; the legacy
  epoch-0 → 1 mapping is applied consistently in `validate_header` and
  `LayoutInfo::matches`, and non-default-epoch layouts correctly reject
  legacy-0 accounts (effective 1 ≠ N).
- **verified sound** `hopper-runtime/compact.rs` — both the exact-length
  `CompactLayout` validator and the relaxed (`>=` MIN_LEN)
  `CompactDynamicLayout` validator were read in full and extended under
  test during this session's compact-dynamic feature work
  (unit + integration + trybuild coverage).
- **verified sound** `hopper-runtime/account_wrappers.rs` — the
  Anchor-parity role-wrapper layer: every wrapper is `repr(transparent)`
  over `&AccountView` and enforces exactly its named role at `try_new`
  (`Signer` → `check_signer`; `Account<T>` → owner + full `load::<T>()`
  header/fingerprint validation; `Program<P>` → address-pin + executable;
  `SystemAccount` → system ownership; `UncheckedAccount` → honestly nothing,
  by name; `InitAccount<T>` → deferred to the paired `init_{field}()`
  lifecycle helper, documented). Mutability flows through the audited
  borrow-tracked `load_mut` path, so `Copy` role wrappers cannot mint
  aliased writable access. The `Interface`/`InterfaceAccount` half
  (owner-set + layout validation, wrong-owner and bad-layout rejection
  tests) was audited during this session's `owner_any` work. Parity note
  for COMPARISON.md: this matches Anchor's wrapper vocabulary at zero
  runtime cost (no RefCell).

### Batch 3 — hopper-core (69 files, ~18.6k lines)

- [ ] manifest.rs, collections/*, account/*, accounts/*, check/*, frame/*,
      virtual_state/*, segment_map.rs, dispatch/*, cpi/*, remaining files

### Batch 4 — macros (hopper-macros-proc 15 files ~12.2k + hopper-macros ~1.9k)

- [ ] state.rs, context.rs, program.rs, dynamic.rs, declare_program.rs,
      pod.rs, remaining; audit the *generated* code paths

### Batch 5 — hopper-schema (9 files, ~13.8k lines)

- [ ] lib.rs, clientgen.rs, rust_client.rs, python_client.rs, go_client.rs,
      c_client.rs, codama.rs, anchor_idl.rs, accounts.rs

### Batch 6 — solana surface

hopper-solana (19 files ~4.2k), hopper-spl/* (~1.9k), hopper-system,
hopper-memo, hopper-svm, hopper-anchor.

- [ ] all files

### Batch 7 — domain and tooling crates

finance, lending, staking, vesting, distribute, multisig (~0.9k total),
hopper-sdk (~2k), hopper-manager (~0.9k), hopper-test.

- [ ] all files

### Batch 8 — examples, tools/hopper-cli, tests

- [ ] all files

## Findings log

### Fixed earlier in this pass (pre-audit, 2026-06-27..30)

- **P0 fixed@0061fe0** `hopper-native/raw_input.rs:183,254,285` — aligned
  `*(p as *const u64)` reads of the BPF input buffer replaced with
  `read_unaligned`; same for `instruction.rs:130` (u32 header read).
- **P2 fixed@0061fe0** `hopper-runtime/account.rs::load_cross_program` —
  missing `required_len` guard before pointer cast (defense-in-depth against
  overridden `validate_header`); regression-tested with a lax foreign contract.

### Batch 1 findings (continued in the entries below; latest first within topics)

- **verified sound** `hopper-native/cpi.rs` — the checked `invoke`/
  `invoke_signed` validate account count, address identity, signer/writable
  flags, **and borrow compatibility** (writable → `check_borrow_mut`,
  readonly → `check_borrow`) before the `sol_invoke_signed_c` syscall, so the
  safe CPI path upholds `invoke_unchecked`'s "no live aliasing borrow"
  invariant. P3 note: the `MaybeUninit::uninit().assume_init()` array idiom is
  sound (array-of-`MaybeUninit` is always init) but could modernize to
  `[const { MaybeUninit::uninit() }; N]`.
- **verified sound** `hopper-native/syscalls.rs` — 35 `extern "C"` syscall
  declarations only; signatures match the Solana/Agave SVM ABI. No logic.
- **P2 fixed (this commit)** `hopper-native/lazy.rs` — `LazyContext`'s
  `Send`/`Sync` impls were **ungated**, while `AccountView` deliberately gates
  the identical raw-pointer impls to `target_os = "solana"` so host fuzzers /
  test harnesses can't share the input-buffer pointers across threads. Gated
  `LazyContext` to match; on-chain behavior (single-threaded) is unchanged.
- **P1 fixed (batch-close commit)** `hopper-native/batch.rs::zero_data` —
  third instance of the unguarded-memset class: a safe fn `write_bytes`-ing
  the whole data region with no borrow check (and duplicating, less
  efficiently, the already-fixed `mem::zero_account_data`). Now delegates to
  the borrow-guarded, SVM-memset-optimized helper.
- **P2 open** `hopper-native/system.rs` + `token.rs` CPI helpers — build
  `CpiAccount`s and call `sol_invoke_signed_c` directly, bypassing the
  borrow-compatibility validation `cpi::invoke` performs (writable →
  `check_borrow_mut`, readonly → `check_borrow`). A live data borrow on an
  account these helpers pass writable is mutated by the callee underneath
  the borrow. Recommendation: run each helper's account list through
  `validate_cpi_accounts` (or take borrows) before the syscall, or document
  the helpers as Tier-C with the aliasing contract.
- **verified sound (batch-close)** remaining Batch 1 files —
  `sha256.rs` (FIPS 180-4-correct const implementation; compression, message
  schedule, and the rem≤55 padding boundary all check out; pinned by
  known-answer tests), `hash.rs`/`log.rs` (slice-descriptor syscall wrappers,
  same convention as sol_sha256), `return_data.rs` (`as_type` is bounds- and
  alignment-checked over a self-owned buffer — no account aliasing),
  `introspect.rs`, `budget.rs` (honestly documents that its CU "guard" is
  diagnostic-only), `verify.rs` (raw-pointer FNV over account data — no
  reference formation, bounds-checked), `sysvar.rs` (`repr(C)` Clock/Rent/
  EpochSchedule match the runtime ABI; **accepted-risk**:
  `rent_exempt_minimum` uses hardcoded cluster rent constants — exact for
  current clusters; use `get_rent()` where exactness matters),
  `batch.rs` (checked lamport math; transfer-before-resize ordering),
  `raw.rs`/`safe.rs`/`expert.rs` (re-export tiers), `capability.rs`/
  `error.rs` (no unsafe), `lib.rs` (crate-wide
  `deny(unsafe_op_in_unsafe_fn)`), nonce reads in `system.rs`
  (length-verified align-1 casts).

### Batch 1 findings

- **P3 fixed (this commit)** `hopper-native/raw_input.rs:142-160` — the
  trailing `while slot < frame.account_count` loop in `deserialize_accounts`
  advanced `offset`/`slot` past the MAX-clamped accounts but neither value was
  used afterward (instruction data and program id already come from
  `scan_instruction_frame`). Dead work; burned CU whenever a transaction
  passed more accounts than the entrypoint's MAX. Loop deleted.
- **accepted-risk** `hopper-native/raw_input.rs` (struct derefs) —
  `(*raw).data_len` and other `RuntimeAccount` field reads assume the loader
  input buffer is 8-aligned. This is a documented Solana loader guarantee
  (`BPF_ALIGN_OF_U128` padding is part of the wire format), and the record
  offsets are maintained 8-aligned by construction (`align_offset(8)` each
  iteration). Scalar reads were hardened to `read_unaligned` at fixed@0061fe0;
  whole-struct derefs retain the loader invariant. No action.
- **verified sound** `hopper-native/entrypoint.rs` — `from_raw_parts(ptr,
  count)` in both entrypoint macros is bounded: `deserialize_accounts` clamps
  `count = frame.account_count.min(MAX)` (raw_input.rs:93) and initializes
  every slot below `count` (duplicate markers must reference earlier,
  already-initialized slots or the parser traps). The SIMD-0321 fast path's
  `ptr::read(... as *const Address)` is sound because `Address` is
  `repr(transparent)` over `[u8; 32]` (align 1). `BumpAllocator` keeps the
  cursor word intact and null-returns on exhaustion; `no_allocator!` aborts
  rather than returning null (upholds `GlobalAlloc`).
- **verified sound** `hopper-native/raw_account.rs` — `RuntimeAccount` is
  `repr(C)`, size compile-asserted to 88; field offsets keep `lamports`
  and `data_len` 8-aligned within the record. The duplicate-marker byte is
  repurposed as `borrow_state` (0xFF == NOT_BORROWED matches the loader's
  canonical-account marker), documented at the type.
- **verified sound** `hopper-native/raw_input.rs::deserialize_accounts` —
  forward/self-referencing duplicate markers trap via
  `malformed_duplicate_marker` (the pre-audit silent-fallback aliasing bug
  stays fixed).
- **P1 fixed (this commit)** `hopper-native/account_view.rs::close` — a *safe*
  fn that memset the entire data region and rewrote the header without
  checking `borrow_state`. A live `Ref<[u8]>`/`RefMut<[u8]>` from
  `try_borrow(_mut)` would be mutated underneath — UB reachable from safe
  code (`close_unchecked` documents exactly this requirement; `close` didn't
  enforce it). Fix: `close()` now fails with `AccountBorrowFailed` while any
  data borrow is outstanding. Regression test
  `close_refuses_while_data_borrow_is_live` (hopper-runtime). Callers
  (native `batch.rs`, runtime `native_boundary::close`) all propagate
  `ProgramResult`, so the guard is non-breaking. `zero_data` (the `close_to`
  path) was already safe — it takes `try_borrow_mut` first.
- **verified sound** `hopper-native/account_view.rs` borrow tracking —
  `borrow_state`: 0xFF = free (loader marker), 0 = exclusive, 1..=254 =
  shared count; increments are capped at 254 so the count can never wrap
  into either sentinel; `try_borrow`/`try_borrow_mut`/`segment_ref`/
  `segment_mut` transitions are all guarded. `resize`/`resize_raw` bound
  growth by `original_len + MAX_PERMITTED_DATA_INCREASE` reconstructed from
  `resize_delta`; the delta invariant is maintained by the resize family
  itself. `Send`/`Sync` impls are gated to `target_os = "solana"`
  (single-threaded SVM).
- **accepted-risk** `hopper-native/account_view.rs::resize` under live
  borrows — growth memsets only `[old_len, new_len)`, which cannot overlap a
  live borrow's `[0, old_len)` slice; shrink only rewrites header fields and
  leaves the live slice's memory valid (loader realloc reserve). No overlap,
  no action.
- **DOC fixed (this commit)** `hopper-native/account_view.rs::
  segment_ref_unchecked` / `segment_mut_unchecked` — safety contracts listed
  bounds preconditions but omitted the aliasing one (no overlapping live
  borrow for the returned lifetime; these methods perform no borrow
  tracking). Both `# Safety` sections extended.
- **P3 open** `hopper-native/account_view.rs::raw_ref`/`raw_mut` — declared
  `unsafe fn` but delegate to the fully-checked `segment_ref`/`segment_mut`;
  their `# Safety` text is vacuous. Either drop `unsafe` or document the
  actual (semantic, not memory) invariant.
- **verified sound** `hopper-native/instruction.rs` — `CpiAccount::from`
  packed-u32 flag masks are little-endian-dependent (documented; BPF and all
  supported hosts are LE). `Seed`/`Signer` raw-pointer + `PhantomData`
  lifetimes match the `sol_invoke_signed_c` ABI and are only constructible
  from valid slices. P3 note: field pointers use `&(*raw).field as *const _`
  where `core::ptr::addr_of!` would avoid materializing intermediate
  references (provenance hygiene; fields are disjoint so not unsound).
- **P1 fixed (this commit)** `hopper-native/mem.rs::zero_account_data` —
  same class as the `close()` finding: a *safe* fn that memset the entire
  data region with no borrow check, mutating memory a live `Ref`/`RefMut`
  still points at. No in-tree callers existed, so the signature change to
  `Result<(), ProgramError>` with a `check_borrow_mut()` guard is
  non-breaking.
- **DOC fixed (this commit)** `hopper-native/mem.rs::copy_bytes` — doc said
  "Err if lengths differ"; behavior accepts a longer `dst` (prefix copy) and
  rejects only `dst` shorter than `src`. Doc corrected.
- **verified sound** `hopper-native/borrow.rs` — `Ref`/`RefMut` drop paths
  restore `borrow_state` correctly (last shared borrow -> NOT_BORROWED,
  otherwise decrement; exclusive -> NOT_BORROWED). Underflow is reachable
  only via already-broken invariants (constructors are `pub(crate)` or
  `unsafe` with documented contracts). `into_raw_parts` leaks the borrow
  count by design (leak-safe, not memory-unsafe); `new_external` null-state
  guards no-op on drop (registry-leased segments release externally); the
  raw-pointer field keeps both guards `!Send`/`!Sync`.
- **verified sound** `hopper-native/mem.rs` syscall wrappers — memcpy /
  memmove / memset / memcmp contracts documented; host fallbacks match SVM
  semantics; safe wrappers bounds-check before delegating.
- **verified sound** `hopper-native/pod.rs` — the Pod/ValuePod split holds
  the audit's safety line: `Pod` (overlay) is implemented only for
  alignment-1 types (u8/i8/arrays/unit); multi-byte integers are
  `Zeroable`+`ValuePod` only, so `&u64` overlays are compile-time rejected;
  `read_unaligned_value` is bounds-checked and alignment-independent.
- **P1 fixed (this commit)** `hopper-native/project.rs` + `lens.rs` — the
  reference-returning projection/lens surface (`project`, `project_safe`,
  `project_slice`, `project_hopper`, `lens::read_field`,
  `lens::read_field_pod`, `lens::read_address`, `lens::read_bytes`) returned
  bare `&T`/`&[u8]` into account data with **no borrow tracking**. Safe
  `project::<u8>()` + safe `try_borrow_mut()` could therefore hold a shared
  reference and `&mut [u8]` over the same bytes simultaneously — aliasing UB
  with zero `unsafe` in user code (the `unsafe impl Projectable` contract
  covers layout, not aliasing; the lens module docs even claimed "never
  cause UB"). Fix: all eight functions now take a shared data borrow via a
  new `AccountView::acquire_shared()` (the `try_borrow` state transition,
  254-cap included) and return `Ref` guards that release on drop, making
  projection and exclusive borrows mutually exclusive. By-value lenses
  (`read_le_u64/u32/u16`, `read_u8`, `read_bool`, `field_eq`) copy through
  raw pointers, form no references, and are unchanged. Only one in-tree
  caller existed (`lens::read_field` → `project`), so the `&T` → `Ref<T>`
  signature change is contained; `Ref` derefs to `&T` for call-site
  compatibility. Regression tests:
  `projection_takes_a_shared_borrow_and_blocks_exclusive`,
  `project_bounds_and_disc_checks_run_before_borrowing`. Full workspace
  builds clean.
- **audited** `hopper-native/lens.rs` — module doc's "never cause UB" claim
  corrected to state the borrow-guard semantics; by-value paths verified
  sound (raw-pointer copies, bounds-checked, no reference formation).
- **P2 fixed (this commit)** `hopper-native/wire.rs` — the native `Le*` wire
  types implemented `Projectable` only, **not** the substrate
  `Pod`/`Zeroable`, so the Pod-bounded native APIs (`lens::read_field_pod`,
  `segment_ref`) rejected the crate's own alignment-1 wire types —
  `read_field_pod`'s own doc example (`&LeU64`) did not compile. All wire
  types (`LeU64/32/16`, `LeI64/32/16`, `LeBool`, `LeU128`) now implement
  `Zeroable` + `Pod` (contract holds: `repr(transparent)` over `[u8; N]`,
  align 1, no padding, all bit patterns valid). Compile-proof test
  `wire_types_satisfy_substrate_pod`.
- **DOC fixed (this commit)** `hopper-native/wire.rs` module doc — claimed
  "checked arithmetic by default: `+`, `-`, `*` return `Option`", but
  `__wire_arith_ops!` implements native-mirroring operators (panic on
  overflow in debug, wrap in release; its own doc says so). Module doc now
  describes the real semantics and points balance math at the explicit
  `checked_*`/`saturating_*`/`wrapping_*` methods.
- **P1 fixed (this commit)** `hopper-native/pda.rs::find_program_address`
  and `hopper-runtime/pda.rs::find_program_address` — on failure (or on any
  non-SVM host) both silently returned `(Address::default(), 0)`, i.e. the
  **all-zero System Program address** presented as the caller's PDA. A
  program (or macro-generated `#[account(seeds=...)]` check — four call
  sites in `context.rs`) that trusted the result would compare against or
  derive the wrong key. Now panics on no-viable-bump and on the host
  fallback, matching upstream `Pubkey::find_program_address`; the fallible
  path remains `based_try_find_program_address`. Host tests must exercise
  PDA paths through the SVM harness.
- **P2 fixed (this commit)** `hopper-native/pda.rs` seed-count truncation —
  `create_program_address` did `seeds.len().min(16)` and
  `verify_pda_with_bump`/`verify_pda_from_stored_bump` did `.min(15)`,
  silently dropping seeds past the cap and deriving a *different* PDA than
  the caller specified. All now reject `seeds.len() > MAX_SEEDS` with
  `InvalidSeeds` (bump helpers size their buffer `MAX_SEEDS + 1`), matching
  the explicit checks the other pda.rs functions already had.
- **P2/DOC fixed (this commit)** `hopper-native/pda.rs` bump
  canonicalization — `verify_pda_with_bump` / `verify_pda_strict` accept a
  caller-supplied bump; without canonicalization multiple addresses verify
  for the same seed set (the classic Solana bump vuln). Added explicit
  guidance to both pointing at `verify_pda_from_stored_bump` (reads the
  account's own recorded bump) as the safe default; renamed the confusing
  `on_curve`/`if on_curve != 0` local (nonzero actually means *off*-curve /
  valid PDA) to `curve_rc` with a comment.
- **P1 fixed (this commit)** `hopper-native/pda.rs::find_program_address` +
  `hopper-runtime/pda.rs::find_program_address` — on failure (no viable
  bump, >MAX_SEEDS, or the non-SVM host path) both silently returned
  `(Address::default(), 0)`: the all-zero address is the **System
  Program**, handed to callers as if it were their PDA. Both now panic
  with a clear message, matching upstream `Pubkey::find_program_address`
  semantics; `based_try_find_program_address` remains the fallible variant.
- **P1 fixed (this commit)** `hopper-native/pda.rs::verify_pda_with_bump` /
  `verify_pda_from_stored_bump` — seeds were truncated at `min(15)`: a
  legitimate 16-seed PDA (MAX_SEEDS) mis-derived and failed verification,
  and larger seed sets silently derived over the wrong subset instead of
  erroring. Both now reject `> MAX_SEEDS` explicitly and copy the full set
  (the stack array hol
