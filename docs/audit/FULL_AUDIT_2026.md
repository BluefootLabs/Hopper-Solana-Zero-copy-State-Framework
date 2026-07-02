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

### Batch 1 — hopper-native (33 files, ~9.2k lines) — IN PROGRESS

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
- [ ] wire.rs
- [ ] pda.rs
- [ ] cpi.rs
- [ ] syscalls.rs
- [ ] sha256.rs / hash.rs
- [ ] batch.rs / budget.rs / capability.rs / error.rs / expert.rs /
      introspect.rs / lazy.rs / lens.rs / lib.rs / log.rs / return_data.rs /
      safe.rs / system.rs / sysvar.rs / token.rs / verify.rs

### Batch 2 — hopper-runtime (46 files, ~20.3k lines)

- [ ] account.rs, compact.rs, tail.rs, segment_borrow.rs, segment_lease.rs,
      layout.rs, zerocopy.rs, account_wrappers.rs, borrow*.rs, address.rs,
      cpi.rs, token*.rs, remaining files

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
