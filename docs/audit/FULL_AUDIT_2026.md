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
- [x] account.rs (audited incrementally across the session: loaders,
      compact + compact-dynamic, cross-program, close, checks, segments,
      split, flags/fused-validation, resize, extension helpers, init_layout)
- [x] token.rs (pattern-verified with spot checks — see findings)
- [x] borrow.rs (host-repr provenance fix — see findings)
- [x] memory.rs
- [x] zerocopy.rs
- [x] segment_lease.rs (+ account.rs::split_segments_mut ordering fix)
- [x] tail.rs
- [x] layout.rs
- [x] compact.rs (fully read + extended with `CompactDynamicLayout` and
      tests during this session's compact-dynamic feature work)
- [x] account_wrappers.rs
- [x] cpi.rs
- [x] context.rs
- [x] borrow_registry.rs (host global race fixed — see findings)
- [x] remaining.rs
- [x] native_boundary.rs
- [x] crypto.rs
- [x] syscalls.rs
- [x] lib.rs
- [x] policy.rs (verified sound — see findings; seeded I12)
- [x] write_policy.rs (new this session — audited at authoring, I12)
- [x] address.rs, instruction.rs, interop.rs, pod.rs, audit.rs,
      compute.rs, crank.rs, field_map.rs, log.rs, proof.rs, ref_only.rs,
      rent.rs (DOC fixed), result.rs, return_data.rs, syscall.rs,
      system.rs (all 13 wire encodings verified), utils.rs
- [x] segment.rs (P2 fixed: end() u64 widening)
- [x] migrate.rs (P2 fixed: overshoot refusal; DOC fixed: atomicity)
- [x] option_byte.rs (P3 fixed: test-only OOB referent + layout pin)
- [x] dyn_cpi.rs (P1 fixed: builder had no invoke)
- [x] token_2022_ext.rs (verified sound; DOC added: OptionalNonZeroPubkey)
- [x] foreign.rs (verified sound — manifest lens four-step pinned)

**BATCH 2 COMPLETE ✅ — 47/47 files.** Together with Batch 1, the entire
unsafe-bearing substrate (hopper-native 33 + hopper-runtime 47 files) is
now line-by-line audited.

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
- **verified sound** `hopper-runtime/cpi.rs` — the checked CPI layer
  validates address identity, signer requirements **with a PDA-signer
  derivation fallback** (`signer_matches_pda` via
  `create_program_address`), writable flags, per-account borrow
  compatibility, and **rejects duplicate writable accounts** before the
  syscall (test-pinned: `duplicate_writable_accounts_are_rejected_before_cpi`)
  — validation beyond both Pinocchio (no checked layer) and Anchor's
  `CpiContext`. `MaybeUninit` account arrays are count-gated; the host-only
  system-transfer emulation uses checked lamport math behind the same
  signer/duplicate validation. Innovation note: "the only zero-copy
  framework whose checked CPI proves borrow-compatibility and
  duplicate-writable safety pre-syscall" belongs in COMPARISON.md.
- **verified sound** `hopper-runtime/context.rs` — the instruction-context
  core: account access is index-checked; the whole segment family
  (`segment_ref/mut`, `_const`, `_typed`, `split_segments_mut`) ties guard
  lifetimes to the ctx borrow through the audited registry path;
  `as_mut_ptr` requires writable and transfers alias-safety explicitly;
  `as_ptr` really does run `check_borrow()` (doc claim verified, line 525;
  the residual TOCTOU is covered by the deref-side unsafe contract);
  `read_data`/`data_slice` are overflow- and bounds-checked;
  `ScopedContext` correctly narrows lifetimes so generated contexts cannot
  widen account references through the raw escape hatch. Duplicate-account
  audits (`require_unique_writable/signer_accounts`) give one-line
  Sealevel-attack mitigations (logged as innovation I6).
- **DOC fixed (this commit)** `hopper-runtime/context.rs::raw_unchecked` —
  claimed to "bypass segment borrow tracking" but delegates to `raw_mut`
  → the fully *checked* `segment_mut(0, …)` path. Behavior is safer than
  documented; doc rewritten as a legacy alias pointing power users at
  `as_mut_ptr` for genuinely untracked access. Also fixed `read_data`'s
  SAFETY comment naming `Pod` where the bound is `ValuePod`.
- **P3 open** `hopper-runtime/context.rs` — `Context.program_id` and
  `Context.instruction_data` are `pub` fields (accessors already exist)
  while `accounts` is private. A handler holding `&mut ctx` can reassign
  `ctx.program_id`, subverting every downstream
  `check_owned_by(ctx.program_id())` in generated validation.
  Self-inflicted only (not attacker-controlled), but the asymmetry is a
  footgun: privatize both fields behind the existing accessors
  (breaking change — schedule with the next API-break window).
- **innovation (second pass)** `hopper-runtime/context.rs` — the embedded
  segment registry doubles as a free **instruction touch map**: by
  end-of-handler it has recorded every `(account, offset, size, R/W)` the
  instruction touched, and `for_each` already exposes it. Logged as
  INNOVATION_IDEAS I7 (wire into receipts + `hopper_test::Trace` +
  `hopper explain`). Also logged for COMPARISON.md: the four-mode
  remaining-accounts taxonomy (strict/passthrough/typed/lazy).
- **P2 fixed (this commit)** `hopper-runtime/borrow_registry.rs` — the
  **non-test host** registry was a `static UnsafeCell` global with a bare
  `unsafe impl Sync` and *no synchronization*: any multithreaded host
  process (fuzzer, downstream host binary) touching accounts from two
  threads raced `&mut` access to the global — the `Sync` claim was
  unjustified. Now guarded by a `core`-only acquire/release atomic
  spinlock (`with_lock`), so the `Sync` impl is justified and a re-entrant
  closure deadlocks loudly instead of aliasing `&mut`. Test builds
  (thread-local `RefCell`) and on-chain builds (ZST no-ops over the native
  borrow byte) were already sound. The registry's conflict matrix itself
  verified correct (shared blocked by mutable, mutable blocked by
  anything, `checked_add` on the shared count, tolerant release).
- **verified sound** `hopper-runtime/remaining.rs` — the four-mode
  remaining-accounts taxonomy (innovation I6): strict-mode `get()` scans
  both the declared prefix and earlier remaining slots for address
  aliases (bounded by `MAX_REMAINING_ACCOUNTS`); `signers::<N>` enforces
  the signer role through the audited `Signer::try_new`;
  `account_views::<N>` bounds by `N`; `take_group`/`assert_sorted_by`/
  `assert_no_duplicates`/`assert_empty` all safe over audited primitives.
  The only unsafe is the standard test-helper account constructor.
- **verified sound** `hopper-runtime/native_boundary.rs` — the
  native↔runtime bridge: every transparent cast is backed by verified
  layout guarantees (runtime `AccountView` is `repr(transparent)` over
  the backend view with compile-time size + `needs_drop` asserts at
  account.rs:69/76; both `Address` types are `repr(transparent)`
  `[u8; 32]`); `wrap_account_slice`'s contract is accurate;
  `account_owner` carries the owner-invalidation caveat with `read_owner`
  as the safe alternative; the entrypoint bridge macro forwards the
  loader contract verbatim. Middle sections (close/zero_data/lamports/
  resize/PDA forwarding) were audited during this session's fixes.
- **verified sound** `hopper-runtime/crypto.rs` — a uniform, rc-checked
  syscall-wrapper pattern over fixed-width buffers for the full crypto
  surface: sha256/keccak/blake3 (slice-descriptor convention), secp256k1
  recover (+ Ethereum-address derivation), curve25519 validate/group/
  multiscalar ops (null-output = validate-only documented; `rc == 0` =
  on-curve), poseidon, and alt_bn128 add/mul/pairing (BE + LE aliases).
  Innovation note for COMPARISON.md: no competitor wraps this complete
  crypto syscall surface behind safe typed APIs.
- **verified sound** `hopper-runtime/syscalls.rs` — extern shims with
  explicit `link_name`s and host fallbacks; declarations only, same
  audited pattern as the native syscall table.
- **verified sound** `hopper-runtime/lib.rs` — crate-wide
  `deny(unsafe_op_in_unsafe_fn)`; the `hopper_unsafe_region!` macro gives
  auditors a single greppable name for every raw reinterpretation site
  (feeds innovation I2's `verify-unsafe` tooling); remainder is
  re-exports/docs.
- **verified sound** `hopper-runtime/token.rs` — the SPL builder/validator
  layer. The single non-test unsafe (`assemble_and_invoke`'s
  count-gated `MaybeUninit` arrays) is bounds-checked
  (`FIXED + multisig ≤ MAX_STATIC_CPI_ACCOUNTS`, checked_add) and —
  importantly — **routes every builder through the fully-validated
  `cpi::invoke_signed_with_bounds`** (borrow-compat, duplicate-writable,
  PDA-signer checks). This **mitigates the Batch-1 P2-open** on the
  native-layer token/system helpers: the primary surface programs use via
  `hopper::token` is the validated one. The `require_token_*` /
  `require_mint_*` validators use `try_borrow` (borrow-checked reads),
  explicit length bounds, and by-copy field extraction at the documented
  SPL offsets. Remaining unsafe is the standard test-helper constructor.
- **account.rs fully closed** — the gap-scan confirmed the last unaudited
  sections (`fields`/`field` inspection, `extension_range`/`bytes`/
  `bytes_mut` — offset-presence + bounds before `slice_from`, borrow-
  tracked — and `init_layout`) are sound; every other section was audited
  during this session's fixes and features.
- **verified sound** `hopper-runtime/policy.rs` — pure compile-time const
  policy levers (`HopperProgramPolicy` STRICT/SEALED/RAW,
  `HopperInstructionPolicy` overrides, program profiles); no runtime
  state, no unsafe operations (the grep hits are docs discussing
  `allow_unsafe`); all invariants test-pinned. Synthesis with the
  touch-map work produced innovation **I12** (field-level write policies
  enforced by the borrow ledger).
- **I7 limitation (logged as I12 prerequisite)** — the shipped touch map
  records only *segment-registry-mediated* access: whole-account borrows
  (`try_borrow_mut`/`load_mut`) use the account-level borrow byte and
  never reach the registry, so they are invisible to the map. Routing
  whole-account write borrows through the same ledger completes the map
  and is required for sound I12 enforcement.
- **I12 SHIPPED (core)** — new `hopper-runtime/src/write_policy.rs`
  (audited at authoring: pure const data + u64-widened containment, no
  unsafe, edge-pinned tests incl. u32-boundary wrap); `Context` write
  acquires gated (`segment_mut*`, `split_segments_mut`, `load_mut`,
  `raw_mut`, `as_mut_ptr`) with a gate-ordering test proving refusal
  fires before header validation or borrow acquisition;
  `Context::load_mut` now `&mut self` and records `(0, data_len)` in the
  touch map (I7 blind-spot closure); macro `strict_writes` option
  compiles `mut` / `mut(seg, ...)` / lifecycle declarations into the
  installed static policy using the same const arithmetic as the
  generated accessors. Enforcement boundary documented: raw
  `AccountView` access via `ctx.account(i)` is outside the governed
  surface (same tier as the raw-pointer escape hatches; `hopper doctor`
  lint queued). Workspace 140 suites green both lanes; 5 trybuild stderr
  snapshots re-blessed (rustc candidate-list sampling shifted — cosmetic
  only, verified line-by-line).

#### Batch 2 tail findings (2026-07-01 sweep, 21 files)

- **P1 fixed** `dyn_cpi.rs` — `DynCpi` was a CPI builder **with no way to
  submit**: the pushed `accounts`/`writable`/`signer` arrays were
  write-only (nothing consumed them) and the module docs promised
  signer-threaded invocation that did not exist. Added
  `invoke()`/`invoke_signed(signers)` assembling the pushed metas into an
  `InstructionView` and routing through the **validated**
  `cpi::invoke_signed_with_bounds` path (address/flag agreement,
  PDA-signer resolution, live-borrow checks, duplicate-writable
  rejection), plus `account_views()` prefix accessor. Tests pin the
  build→submit chain end-to-end: flag-mismatch surfaces
  `MissingRequiredSignature` from the *built* metas, duplicate writable
  refused, host validate+no-op submit.
- **P2 fixed** `segment.rs` — `Segment::end()` was `offset + size` on
  `u32`: near-`u32::MAX` offsets wrapped, making `contained_in` falsely
  TRUE (a bounds-escape shape) and `overlaps` falsely false. Widened
  `end()` to `u64` (same lesson the registry's `ranges_overlap` learned);
  `TypedSegment::end()` matched; wrap-pinning test added. Public-API
  break is a return-type widening only.
- **P2 fixed** `migrate.rs` — a misdeclared edge overshooting
  `SCHEMA_EPOCH` (e.g. chain 1→3 with target 2) was applied silently:
  the header got stamped with a **future** epoch and `Ok` returned,
  bricking the account for every subsequent typed load. Now refused
  (`InvalidAccountData`) before any byte is written; test with a real
  account fixture. **DOC fixed** — the "never a hybrid" atomicity claim
  was false for migrators that error after partial body writes; the
  contract now states epoch-bump-after-success + errors MUST propagate
  to instruction failure (tx rollback is what makes it safe).
- **P3 fixed** `option_byte.rs` — tests cast a 9-byte buffer to
  `&OptionByte<u64>` (size **16**: repr(C) puts the value at offset 8,
  not 1) — an out-of-bounds referent (validity-rule UB, Miri-flaggable)
  with a comment mis-modeling the layout. Fixed to full-size aligned
  buffers with honest SAFETY comments + a layout-pinning test
  (`OptionByte<[u8;32]>` is 33 bytes align 1 — the overlay-facing form).
- **DOC fixed** `rent.rs` — claimed `AccountNotRentExempt` routes
  through `Custom` / builtin 29; it is builtin 14 in Hopper's ABI.
- **verified sound** `system.rs` — all 13 System Program builders
  checked against the interface wire format (tags 0–12, bincode field
  order, u64-len seed strings, account meta shapes); every builder
  routes through the validated CPI path, so address/flag agreement and
  duplicate-writable rejection apply. (The earlier P2-open about CPI
  helpers bypassing validation concerned hopper-**native**'s helpers,
  not these.)
- **verified sound** `instruction.rs` — `StoredInstruction` bounds
  against `MAX_CPI_ACCOUNTS` and buffer `N` with initialized-prefix
  exposure; `CpiAccount` C-ABI carries `PhantomData` lifetime pinning;
  `Seed`/`Signer` raw-pointer FFI pairs are lifetime-pinned the same way.
- **verified sound** the rest of the tail: `address.rs` (Pod/seal impls
  correct for transparent [u8;32]; 4×u64 unaligned fast-eq),
  `interop.rs` (unsafe `TransparentAddress` marker, both impls genuinely
  transparent), `pod.rs` (doc-carrier re-export + compile-fail
  doctests), `audit.rs` (O(n²) scan fine at Solana account counts),
  `return_data.rs` (truncation surfaced via `is_truncated`; P3 noted:
  ~1 KiB by-value snapshot on 4 KiB SBF frames — document stack cost),
  `compute.rs`, `crank.rs`, `field_map.rs`, `log.rs` (StackWriter
  truncates silently — acceptable for logs), `proof.rs` (P3 noted:
  `assume_token_extensions_checked` is the only unverified proof grant —
  honestly named; Seeds/HasOne markers granted macro-side), `ref_only.rs`
  (sealed, grep-receipt claim matches), `result.rs`, `syscall.rs`,
  `utils.rs`.

#### Batch 2 closing findings (final two files)

- **verified sound** `token_2022_ext.rs` — the TLV walk is
  overflow-safe (`cursor + 4` / `data_start + len` cannot wrap at
  Solana account sizes; run-past-buffer returns `None`, never panics);
  the type-0 stop matches spl-token-2022's uninitialized-marker
  semantics; unknown types are walked over (forward-compatible);
  region slicing mirrors `validate_account_type` (AccountType at 165,
  TLV at 166, permissive kind==0 for init sequencing — safe because a
  zeroed region terminates on first header read); all six documented
  extension layouts verified against spl-token-2022 (TransferHook,
  MetadataPointer, TransferFeeConfig, InterestBearing,
  DefaultAccountState, MintCloseAuthority/PermanentDelegate). The test
  suite pins the *real* 82+83+1 mint layout and documents a historical
  two-wrongs-align bug. **DOC added:** authority comparators now warn
  that Token-2022 authorities are `OptionalNonZeroPubkey` (all-zero =
  unset), so an all-zero `expected` would "match" an unset authority.
- **verified sound** `foreign.rs` — `ForeignLens::open` implements the
  audit's page-14 manifest contract exactly: owner match → header load
  (disc/version/length via the authored path) → wire-fingerprint
  matched against BOTH the manifest and `T::WIRE_FINGERPRINT` → epoch
  range; borrow-guard ordering is correct (shared+shared during header
  re-read, explicit drop before handing out the lens).
  `ForeignLens::field::<F, OFFSET>` overflow- and bounds-checks against
  the body before the cast; `F: ZeroCopy` supplies the every-bit-valid
  and align-1 contract. `ExternalLens` is offset-overflow-checked;
  `ExternalAccount::new_unchecked` carries a proper unsafe contract;
  the snapshot-hash / `assert_unchanged_after` oracle-consistency
  primitives are the I11 cluster's strongest members.

#### Phase 3 benchmark findings (2026-07-02 four-framework run)

- **First measured Anchor row** (anchor-lang 0.31.1):
  5017/2284/5156/7150/5108 CU at 190.11 KiB vs Hopper's
  466/107/564/1713/488 at 7.46 KiB. Results tracked in
  `hopper-bench/results/framework-vaults-2026-07-02-post-i10/`; bench
  repo `5da482a` (fixed the removed backend-selection feature forwards
  and the anchor-vault `declare_id!` placeholder that aborted local
  Mollusk runs with `DeclaredProgramIdMismatch`).
- **P2-perf RESOLVED (bisected 2026-07-02) — the +13…+44 CU delta was
  the removal of an unsound optimization, not a regression.** Method:
  control run first (May framework commit `300797d` + today's runner
  reproduces 431/72/551/1669/453 bit-for-bit, exonerating the harness),
  then automated `git bisect run` over `300797d..411790f` in a
  throwaway worktree (build parity vault per candidate, measure, skip
  broken-at-commit states from the history rewrite). **First bad
  commit: `8899e99`** — the SIMD-0321 feature-gating of
  `fast_entrypoint!`. Pre-`8899e99` the parity vault entered through
  the two-argument `r2` entrypoint unconditionally; SIMD-0321 is not
  active on any public cluster, so that path reads an uninitialized
  register on mainnet and only worked under local SVMs. The May
  numbers were therefore unachievable on real clusters; the current
  numbers are the honest ones, and the ~30–40 CU scanning cost returns
  as savings when the SIMD-0321 gate activates. Corollaries proven by
  the bisect endpoints: **I10, I7, and I12 cost exactly 0 CU** (parity
  vault identical at `411790f` and current head — the original
  suspicion of I10's fallback shape was wrong), and the wider
  auth-fail gap to Pinocchio is the same entrypoint story. Suspects
  (1)–(3) above: all cleared by measurement.

### Batch 3 — hopper-core (69 files, ~18.6k lines) — IN PROGRESS

Wave 1 (unsafe-dense core, 2026-07-02):

- [x] frame/mod.rs (P2 fixed: shared-provenance write pointers ×2)
- [x] frame/phase.rs (P2 fixed: sticky borrow bitmask removed)
- [x] frame/args.rs (verified; updated for the phase.rs field removal)
- [x] cpi/mod.rs (P1 fixed ×2: release-mode transmute UB, zeroed
      references; P2 fixed: silent signer/seed truncation)
- [x] account/pod.rs (verified sound — bounds-checked casts, honest
      Tier C contracts)
- [x] account/segment.rs (P3 hardened: MAX_SEGMENTS enforced, checked
      offset accumulation in init)
- [x] abi/* (typed_address, integers, boolean, field_ref, mod)
- [x] collections/* (slab, ring_buffer, journal, packed_map, fixed_vec,
      sorted_vec, bit_set, slot_map, compact_tail)
- [ ] account/* remainder (registry, lifecycle, dynamic, verified,
      realloc_guard, cursor, header, segment_role, reader, overlay),
      manifest.rs, receipt.rs, check/*, policy.rs, virtual_state/*,
      diff/*, math/*, segment_map.rs, dispatch/*, accounts/*, event/*,
      invariant/*, migrate/*, sysvar/*, state/*, time/*, lib.rs

#### Batch 3 Wave 1 findings

- **P1 fixed** `cpi/mod.rs` — both const-generic CPI builders
  (`HopperCpi`, `HopperCpiBuf`) guarded their MaybeUninit→initialized
  reference-array transmute with only a `debug_assert`: on SBF
  (release) the assert compiles out, so an under-filled builder
  transmuted **uninitialized `&AccountView` references** — validity UB
  the moment they exist. Now a real runtime check
  (`NotEnoughAccountKeys`). Second latent UB in the same functions:
  `core::mem::zeroed()` for `[InstructionAccount; N]` materialized
  **null `&Address` references** (invalid on creation even though
  overwritten before use; invisible to host tests because the whole
  path is `cfg(target_os = "solana")`). Now initialized from a valid
  template (`InstructionAccount::readonly(program_id)`), removing the
  unsafe entirely. The `Signer`/`Seed` zeroed arrays stay (raw-pointer
  pairs — all-zero is a valid value) with honest SAFETY comments.
- **P2 fixed** `cpi/mod.rs` — PDA signer seeds were **silently
  truncated** (`.min(4)` signers, `.min(16)` seeds): a dropped seed
  produces a *wrong PDA signature*, not a clean error. Now refused
  (`InvalidArgument` / `MaxSeedLengthExceeded`).
- **P2 fixed** `frame/mod.rs` — `segment_mut` and
  `segment_mut_unchecked` derived their **write** pointer from a shared
  reborrow (`(&*data) as *const [u8] as *mut [u8]`): shared-tagged
  provenance, writes through it are UB under Stacked Borrows — the same
  class fixed in `borrow.rs::RefMut::from_backend` during Batch 2. Now
  derived via `RefMut::as_bytes_mut_ptr` (a `&mut [u8]` reborrow) as
  the final `&mut` use before `project` consumes the guard.
- **P2 fixed** `frame/phase.rs` — `ExecutionContext::borrow_mut` set a
  per-index borrow bit that the returned `RefMut` **never cleared on
  drop** (unlike `FrameAccountMut`, which releases via Drop): legal
  sequential re-borrows failed for the rest of the execute phase — the
  sticky-ledger bug class again. The bitmask was also strictly *weaker*
  than the account borrow byte it duplicated (per-index bits treat
  Solana duplicate account metas as distinct; the byte is per-account).
  Removed the redundant layer entirely; the byte-level guard is
  authoritative. Tests pin both semantics: RAII re-borrow succeeds,
  duplicate-meta double-borrow is refused.
- **P3 hardened** `account/segment.rs` — `MAX_SEGMENTS` was declared
  but never enforced (now checked in both table constructors), and
  `SegmentTableMut::init`'s u32 offset accumulation could wrap with
  large specs, placing later segments **on top of** earlier ones
  (overlapping data regions, silent logical corruption). Now
  checked-add. Everything else verified sound: descriptor casts are
  bounds-checked align-1, mutable slice bounds use capacity, swap is
  byte-wise in-bounds.
- **verified sound** `account/pod.rs` — length-checked align-1 casts,
  unaligned read/write copies, Tier C escapes carry real caller
  contracts.

Wave 2 (abi/* + collections/*, 2026-07-02):

- [x] abi/{typed_address,boolean,integers,field_ref}.rs, abi/mod.rs
- [x] collections/{slab,ring_buffer,journal,packed_map,fixed_vec,
      sorted_vec,bit_set,slot_map,compact_tail}.rs

#### Batch 3 Wave 2 findings

- **P1 fixed (OOB write) `collections/slab.rs`** — `alloc` used the
  `free_head` read from account bytes as a slot index **without a
  capacity bound**: a `free_head` between `capacity` and `NO_FREE - 1`
  (attacker-influenceable, since account data is untrusted) computed an
  out-of-bounds `slot_offset` and the unchecked `copy_nonoverlapping`
  wrote `T::SIZE` bytes past the account buffer. Added `idx >= capacity`
  guard, **plus** an occupancy-bitmap check: a free list rewired onto an
  *allocated* slot (or into a cycle) would otherwise hand live data out
  for silent overwrite — the bitmap is the ground truth, and requiring
  the popped head to be free defeats both shapes. Count updates now
  saturate (matching `free`). New test suite (was untested): roundtrip,
  double-free, full, corrupt-free_head OOB, and rewired-to-live-slot
  regressions.
- **P1 fixed (OOB write) `collections/ring_buffer.rs`** — `push` wrote
  at `slot_offset(head)` with `head` read from account bytes and **no**
  bound (unlike `get`, which reduces its physical index `% cap`). A
  corrupted `head >= capacity` was an OOB write; the I13 harness then
  caught a second hit in `get` (`cap - (count - head)` underflow-panic on
  a corrupt-high count). Final design goes past point-guards to
  **parse-don't-validate**: `from_bytes` validates `head`/`count`
  against the (cached) capacity once and **rejects** inconsistent
  geometry with `InvalidAccountData` — the pattern `Slab`/`Journal`
  already used, and notably the two collections that did *not* have
  these bugs. Methods then operate on a proven-consistent view (sound
  because the handler holds `&mut [u8]` exclusively), with cheap
  backstop guards retained. Both unsafe blocks additionally prove their
  **exact touched range** (`offset .. offset + size_of::<T>()`) locally
  instead of relying on the geometry chain by convention.
- **P1 fixed (compile-time) `collections/*` element contract** — every
  collection did its offset/capacity math in units of
  `FixedLayout::SIZE` while the actual `read_unaligned`/
  `write_unaligned` moved `size_of::<T>()` bytes; the two are equal only
  by convention (`SIZE` is a hand-written const with no language-level
  tie to the type). A mismatched impl made bounds math and write width
  disagree (OOB write past a proven-in-bounds offset), and a zero `SIZE`
  made `capacity()`'s division panic. Neither is reachable by the
  byte-fuzzing harness (they are properties of the *type parameter*).
  New `assert_zero_copy_element::<T>()` — a `const`-block assertion
  (`SIZE == size_of::<T>() && SIZE > 0`) invoked in every constructor —
  turns both into per-monomorphization **build errors** at zero runtime
  cost.
- **P1 fixed (OOB read)** `collections/{packed_map,fixed_vec,sorted_vec}.rs`
  — all three trusted the element **count** read from account
  bytes unclamped, then looped or indexed on it: `PackedMap::find` /
  `read_key`, `FixedVec::{get,pop,swap_remove,clear}`, `SortedVec::
  {binary_search,remove,max}` would read (and `clear` panic-index) past
  the slot region on a corrupted count. Fixed at the source by clamping
  `len()` to `capacity()` — a no-op for well-formed containers (stored
  count is always `<= capacity`), a hard bound for malformed ones. Added
  a clamp regression test to `FixedVec`.
- **P2 fixed (mod-by-zero panic) `collections/journal.rs`** — a
  zero-capacity circular journal (buffer sized for the header only) hit
  `head %= self.capacity` → divide-by-zero panic (transaction abort /
  DoS). Guarded `capacity == 0` at the top of `append`. Added strict,
  circular-wrap, and zero-capacity tests (was untested).
- **P3 fixed `abi/integers.rs`** — the `Add`/`Sub`/`Mul` (and `*Assign`)
  operator impls on every wire integer used native `+`/`-`/`*`, which
  **wrap silently in release** — `balance += amount` compiling to
  wrapping arithmetic on mainnet is the classic overflow-exploit shape.
  Now overflow-checked (panic → transaction abort, a loud fail-safe);
  the recoverable path remains the `checked_*_assign` helpers. Panic
  tests pin the new behavior.
- **P3 fixed `abi/boolean.rs`** — `WireBool` derived `PartialEq`, so it
  compared raw bytes: `WireBool([0xFF]) != WireBool::TRUE` despite both
  projecting to `true` under the type's own "non-zero = true" rule
  (a foreign writer's `0xFF` would spuriously mismatch). Hand-wrote
  `PartialEq`/`Eq` over the boolean projection; lawful-Eq test added.
- **verified sound** `abi/{typed_address,field_ref}.rs`,
  `collections/{bit_set,slot_map,compact_tail}.rs` — SlotMap
  bounds-checks every key index against `capacity()` and its generation
  counter defeats ABA; BitSet bounds-checks every bit against
  `data.len()`; `compact_tail` is a thin bridge routing through the
  now-hardened constructors; `field_ref`/`typed_address` are
  length-checked align-1 projections.
- **I13 SHIPPED — hostile-metadata property harness**
  (`collections::hostile_metadata_proptests`): all 8 collections fuzzed
  with fully arbitrary buffer contents through their whole API, required
  to return clean `Err`s — never panic, never leave bounds. Earned its
  keep immediately by catching the ring `get` underflow that the manual
  point-fixes had missed. Root-cause synthesis: five findings, one
  cause — *collections trusting their own stored metadata, which is
  attacker-writable account bytes between instructions*. Follow-up
  queued: roll `parse-don't-validate` to
  `fixed_vec`/`sorted_vec`/`packed_map`/`slot_map` (currently
  clamp-on-read, which is memory-safe but silently tolerates corrupt
  headers instead of refusing them).
- **Competitor note:** Quasar/Anchor/Pinocchio ship **no** on-chain
  zero-copy collections at all — this whole surface is Hopper-only, now
  corruption-fuzzed. Quasar's one discipline worth importing here: an
  adversarial Miri suite (`lang/tests/miri.rs`, Tree-Borrows flags, a
  documented findings table) — logged as I14.

Wave 3 (account/* remainder, 2026-07-02):

- [x] account/{registry,lifecycle,dynamic,verified,realloc_guard,cursor,
      reader,header,segment_role}.rs

#### Batch 3 Wave 3 findings

- **P0 fixed (OOB read / UB) `account/reader.rs`** — `address_at(offset)`
  bounds-checked with `offset + 32 > data.len()`, an add that **wraps**
  for a near-`usize::MAX` offset: the wrapped small value passes the
  bound, then the raw `data.as_ptr().add(offset)` is out-of-bounds
  pointer arithmetic (UB) reading 32 bytes from a wild address. Public
  method — if `offset` is ever derived from instruction data it is an
  exploitable OOB read. Fixed with `checked_add`; `u64_at` hardened the
  same way for consistency (its array reads panic-not-UB, but the check
  is now honest). Regression test with `usize::MAX - 16`.
- **P1 fixed (panic + silent overlap) `account/registry.rs`** —
  `SegmentRegistryMut::init` (1) wrote the header and entry table
  without checking `data.len() >= entries_offset + count*16`, so an
  undersized buffer **panicked** at the slice index (the "verified
  above" SAFETY note was never actually enforced); and (2) accumulated
  each segment's data offset with unchecked `current_offset += size`
  (hopper-core opts out of the overflow-checks profile), so a wrapped
  offset placed a later segment **inside** an earlier one — overlapping,
  silent corruption. Both fixed (up-front bound + `checked_add`); two
  regression tests.
- **verified sound** `account/lifecycle.rs` — checked lamport
  arithmetic throughout; `safe_realloc` does all funding validation
  *before* the resize and fails fast on outstanding borrows;
  `safe_close_with_sentinel` zeroes the whole account **and** stamps the
  `0xFF` revival-attack sentinel at byte 0 (stronger than Quasar's
  zero-the-discriminator close). Thoroughly tested.
- **verified sound** `account/realloc_guard.rs` (all-checked cumulative
  growth budget, u32-boundary tested), `account/cursor.rs` (every
  advance length-checked, `pos <= len` invariant), `account/verified.rs`
  (all overlays size-checked; `overlay_at` uses `checked_add`),
  `account/dynamic.rs` (parse validates every range; cached-offset
  accessors are a clean parse-don't-validate — offsets were bounds-proven
  at parse and the view is immutable), `account/header.rs` (all reads
  bounds-checked; `FixedLayout` simplified to the I15 empty body),
  `account/segment_role.rs` (pure const enum, no unsafe).

Wave 4 (the two big files + validation core, 2026-07-02):

- [x] receipt.rs (1214), manifest.rs (1806)
- [x] check/{fast,mod,guards,trust,modifier,graph}.rs, policy.rs (660)

#### Batch 3 Wave 4 findings

- **verified sound** `receipt.rs` — **no `unsafe` anywhere**; the whole
  serializer writes into a compile-time `[u8; RECEIPT_SIZE=72]` at fixed
  offsets, and `DecodedReceipt::from_bytes` gates untrusted input at
  `RECEIPT_SIZE_LEGACY` before reading, only touching the extended
  failure payload when `len >= RECEIPT_SIZE`. **P3 hardened:**
  `commit_with_segments` computed `offset + size` unchecked — a wrapped
  (malformed) segment spec would make `offset > end` and panic the
  `[offset..end]` slice. Now `checked_add`, with overflow folding into
  the existing "extends past buffer" arms. (Segment specs are
  `#[hopper::state]` code constants, not instruction data, so this was a
  latent-not-reachable panic; hardened for defense-in-depth.)
- **verified sound** `manifest.rs` — `ProgramManifestView::parse` and
  `entry(i)` are exemplary: header cast is length-checked align-1 Pod,
  entry access uses `.get(start..start+SIZE)?` (no panic), and the
  entry count is `u16`-bounded so `registry_len` can't overflow;
  `write_registry` bounds every write against a pre-checked `total`.
  The two `FixedLayout` impls already dropped to the I15 empty body
  (Wave 2). **P3 fixed:** `AccountDescriptor::validate` read `data[0]`
  after only a `data.len() < min_size` check — a degenerate
  `min_size == 0` entry passes that on an *empty* account, so the bare
  index would panic. Now `data.first()`.
- **P2 fixed `check/fast.rs`** — `read_account_header` reinterprets
  `&AccountView` as `*const *const u8` and dereferences it, sound only
  because `AccountView` is `#[repr(C)]` with a `*mut RuntimeAccount` as
  its sole non-ZST field. The module docs claimed a compile-time size
  assertion "below will fail" if that layout changed, but **none
  existed**. This is a security-critical fast path (gates
  signer/writable; a wrong read could false-accept a non-signer), so
  the missing guard mattered. Added `const _: () =
  assert!(size_of::<AccountView>() == size_of::<*const u8>())` — a
  future hopper-native layout change now fails to compile instead of
  silently reading the wrong bytes.
- **verified sound** `check/mod.rs` (Tier-1 checks delegate to
  `AccountView` accessors), `check/guards.rs` (checked lamport
  conservation, bounds-checked snapshots, one correct align-1
  `Address→[u8;32]` cast), `check/trust.rs` (every foreign-account
  header read bounds-checked before indexing; all three trust levels
  validate owner+size+layout_id), `check/modifier.rs` (thin wrappers
  delegating to the audited `check::*` / `VerifiedAccount` paths),
  `check/graph.rs` (`add` bounds-checks against `N`, `run` iterates
  within `count`, `Option<ValidateFn>` storage — no `MaybeUninit`
  risk), and `policy.rs` (capability→requirement resolver: `when`
  const-asserts `count < N`, `resolve` iterates within `count`, pure
  bitmask logic, no unsafe).

Wave 5 (diff/receipt engine, virtual_state, math, small files, 2026-07-02):

- [x] diff/mod.rs (393), receipt.rs truncation propagation, math/mod.rs,
      segment_map.rs, dispatch/mod.rs, virtual_state/mod.rs,
      accounts/{segmented,hopper_account,program_account,unchecked}.rs,
      event/mod.rs, and the no-unsafe logic files (invariant, migrate,
      sysvar, state, time)

#### Batch 3 Wave 5 findings

- **P1 fixed (silent audit-trail hole) `diff/mod.rs` + `receipt.rs`** —
  the diff engine underpins `StateReceipt` (the I2 "provable
  audit-trail moat"). `StateSnapshot` carries a `truncated` flag for
  accounts larger than its stack buffer, but `diff()` **dropped it**:
  `StateDiff` set `old_full_len` to the *capped* snapshot length, so
  every downstream query silently operated on only the first `SIZE`
  bytes. For an account bigger than the window, a mutation confined to
  the tail made `has_changes()` return **false**, `changed_byte_count()`
  return 0, and `restore_into()` do a **silent partial rollback** that
  returned `Ok`. A receipt that misses a mutation is worse than no
  receipt — this is a soundness hole in the moat. Fixes: (1) `StateDiff`
  now carries `truncated`, exposes `is_complete()` / `was_truncated()`,
  and `has_changes()` is conservative (returns `true` when it cannot see
  the whole account rather than falsely reporting "unchanged"); (2)
  `restore_into` **refuses** a truncated snapshot (`InvalidAccountData`)
  with an explicit `restore_head_into` for a deliberate partial restore;
  (3) `StateReceipt` gains a `snapshot_truncated` flag, set on `commit`
  and serialized as flag **bit 5** (backward-compatible), decoded into
  `DecodedReceipt` — so an off-chain auditor can tell a complete receipt
  from an incomplete one. Also removed a dead always-false branch in
  `StateDiff::has_changes`. Regression tests pin the once-silent
  tail-mutation case and the full commit→wire→decode roundtrip.
- **P2 fixed (div-by-zero) `virtual_state/mod.rs`** —
  `ShardedAccess::new` accepted an empty `shard_indices`, making
  `shard_count == 0` and `shard_for_key`'s `hash % self.shard_count` a
  divide-by-zero panic (DoS). Rejected at construction. Tests added.
  `VirtualState` itself verified sound (all slot/account accesses
  bounds-checked; builder methods const-assert `slot < N`).
- **P3 hardened `diff/mod.rs`** — `range_changed` / `field_changed` /
  `field_diff_mask` used unchecked `offset + size`; wrapped to
  `checked_add`, folding overflow into each "out-of-bounds = changed"
  arm (offsets are layout constants, so latent-not-reachable, hardened
  for consistency).
- **verified sound** `math/mod.rs` (every op checked, `c == 0` guarded
  before both mul-div variants, u128 intermediates, safe narrowing),
  `segment_map.rs` (compile-time const layout metadata + an
  `assert_segment_field_alignment` isomorphism check), `dispatch/mod.rs`
  (all readers bounds-check before slicing; macros guard the event-CPI
  prefix with `len >= 2`), `accounts/segmented.rs` (the `entry()`
  unsafe read is bounded by `from_account`'s table-fits validation;
  `segment_data` checked-adds), `event/mod.rs` (the
  `MaybeUninit::uninit().assume_init()` is the correct `uninit_array`
  idiom — `from_raw_parts` exposes only the `count` initialized
  elements, unlike the cpi/mod.rs bug; `data_len > 1024` guards the
  copy), and the `accounts/*` `owner()` unsafes (thin delegations to the
  audited `AccountView` accessor). The no-unsafe logic files
  (invariant, migrate, sysvar, state, time) are pure composition.

### Batch 4 — macros (hopper-macros-proc 15 files ~12.2k + hopper-macros ~1.9k) — COMPLETE ✅

- [x] state.rs (headered + compact paths, fingerprint algorithm, tests)
- [x] pod.rs, init_space.rs, constant.rs (verified sound)
- [x] error.rs, event.rs, args.rs (all three fixed — see findings)
- [x] migrate.rs, crank.rs, dynamic.rs (verified sound; crank P3 note)
- [x] context.rs (the 4.2k-line core — `zero`, close/sweep targets,
      PDA-init all fixed; full constraint surface read)
- [x] program.rs (verified sound — dispatch ordering, prefix-shadow,
      duplicate detection, policy lowering all check out)
- [x] declare_program.rs (P3 noted), dynamic_account.rs (verified sound)
- [x] hopper-macros/src/lib.rs (declarative macros; hopper_init! fixed,
      fingerprint-split DOC finding)
- [x] hopper-macros-proc/src/lib.rs (entry points + doc examples)

**Validation:** full-workspace `cargo test --workspace --locked` green
(140 test-result lines, 0 failures) including the receipt wire-format
change from Batch 3 Wave 5 — the owed workspace confirmation is done.
Proc-macro crate: 51 unit tests green including new fingerprint pins.

#### Batch 4 findings

- **P0 fixed `args.rs`** — the generated `parse()` cast
  attacker-controlled instruction bytes to `&Self` with **no
  compile-time fence at all**: (a) no alignment-1 requirement, so the
  macro's own doc example (`pub amount: u64`, align 8) produced a
  misaligned `&Self` whenever the args region follows a 1-byte
  discriminator — instant UB; (b) no padding check, so `PACKED_SIZE`
  (sum of field sizes) could be **smaller** than `size_of::<Self>()`
  and the length check under-validated the buffer → the returned
  reference spanned out-of-bounds bytes; (c) no per-field Pod proof, so
  a `bool` field materialized from an arbitrary byte was
  invalid-value UB. The one macro whose entire job is parsing hostile
  input was the only overlay-emitting macro without the fence every
  other path has. Fixed: `#[repr(C)]` requirement + the standard
  three-assert fence (align-1 / no-padding / non-zero) + per-field
  `__ArgsFieldPodProof`, and an honest SAFETY comment tied to the
  fence. No in-tree consumers used native-int args, so nothing broke.
- **P0-class fixed `event.rs`** — the docs promised "the pod assertion
  block below compile-errors if the struct violates the Pod contract"
  and `as_bytes`'s SAFETY comment relied on it, but the emitted
  "assertion block" was **an empty `const _: () = {}` with a comment
  inside**. A padded event struct compiled fine and `as_bytes` leaked
  uninitialized padding bytes into the program log (an info leak, and
  nondeterministic wire bytes for indexers). Fixed: `#[repr(C)]`
  requirement + align-1/no-padding asserts + per-field Pod proof;
  SAFETY comment now true. (Cross-ref: Quasar's `#[event]` carries the
  size==sum assert and a closed field vocabulary — Hopper now checks
  strictly more.)
- **P1 fixed `state.rs::canonical_wire_stem`** — the `hopper:wire:v2`
  fingerprint stripped **all** generic arguments from path types on the
  assumption they are phantom-only. True for `TypedAddress<T>`, false
  in general: a size-bearing generic Pod overlay (`Pack<WireU64,
  WireU32>` vs `Pack<WireU32, WireU64>` — hand-sealed via the
  documented `HopperZeroCopySealed` opt-out) produced **identical
  fingerprints for different wire shapes at identical total size**,
  defeating exactly the drift-detection the foreign-lens 4-point check
  (owner+disc+fingerprint+size) exists to provide. Fixed:
  `TypedAddress` keeps the phantom exemption (fingerprints unchanged
  for all in-tree layouts); every other generic argument folds its
  stem (and const args their literal) into the fingerprint.
  Over-sensitivity for user phantom types is the safe direction — a
  loud false alarm, never a silent collision. Regression tests pin
  both directions.
- **P1 fixed `context.rs` (`zero` was a silent no-op)** — the parser
  accepted `#[account(zero)]`, stored the flag… and **no emission path
  ever read it**. The Anchor-parity re-initialization guard the user
  declared simply did not exist; worse, the typed-layout load emitted
  for the same field would reject a genuinely zeroed account, making
  the combination unusable in the one scenario it exists for. Fixed:
  `zero` now emits a first-byte-is-zero check (every `#[hopper::state]`
  layout compile-asserts `DISC != 0`, so byte 0 == 0 proves
  "no layout stamped here"; empty accounts rejected — `zero` means
  pre-allocated), returns `AccountAlreadyInitialized`, and the typed
  load is skipped for `zero` fields (owner pin kept).
- **P1 fixed `context.rs` + `hopper-macros::hopper_init!` (PDA init
  could never succeed)** — `#[account(init, seeds = [...], bump)]`
  validated the PDA address and then generated an `init_<field>()`
  whose System-Program CPI used the **unsigned** `invoke()`. The
  System Program requires the created (or allocated+assigned) account
  to sign; a PDA can only do that via `invoke_signed` with its seeds —
  so every seeded init compiled, validated, and failed at runtime.
  Latent (no in-tree example uses init+seeds) but fatal for the
  primary Anchor-porting pattern. Fixed end-to-end: `hopper_init!`
  gained a `signers = ...` arm threading `invoke_signed` through
  CreateAccount / Allocate / Assign (unsigned arms delegate with
  `&[]`, byte-identical behavior); the generated helper builds
  `[declared seeds…, bump]` signer seeds using `self.bumps.<field>`
  (gathered during `bind()` before any lifecycle helper can run);
  instruction args are threaded through seeded init helpers so seed
  expressions can reference them; and `init` + `seeds::program` is now
  a compile error (a foreign program's PDA can never sign here).
- **P2 fixed `error.rs`** — auto-coded variants (no explicit `= N`)
  got SHA-derived codes in `code()`/`CODE_TABLE`/`From<T> for u32`,
  but the re-emitted enum kept Rust's sequential discriminants — so
  the natural `MyError::Variant as u32` silently disagreed with the
  wire code in `ProgramError::Custom`. Fixed: derived codes are
  written back as explicit discriminants, so `as u32`, `code()`, and
  the wire agree by construction, and rustc now rejects a collision
  between a derived and an explicit code at compile time (previously
  silent). (Cross-ref: Quasar avoids the class by using `e as u32`
  directly with sequential codes — consistent but not reorder-stable;
  Hopper now has both properties.)
- **P2 fixed `context.rs` (lamport-receiving targets)** — `close =
  target` and `sweep = target` drain lamports into the target, and the
  SVM rejects lamport changes on non-writable accounts — but the
  generated validation never checked the *target's* writability, so a
  read-only destination failed the whole transaction at commit instead
  of at validate. Both now emit `check_writable` on the target. Also
  `sweep`'s documented "implies `mut` on the source" contract was not
  enforced (the parser did not set `is_mut`) — now it does.
- **P3 fixed `state.rs` compact path** — `expand_compact` stamped
  `Zeroable`/`Pod` but not `HopperZeroCopySealed`, so compact layouts
  (equally macro-authored, with identical field proofs) were excluded
  from the `ZeroCopy` blanket that headered/`#[hopper::pod]`/
  `hopper_layout!` types all get. One-line seal added.
- **DOC/P2 open — fingerprint algorithm split** — declarative
  `hopper_layout!`/`hopper_interface!` compute `LAYOUT_ID` with the
  legacy `hopper:v1` stringify-based algorithm (the audit-flagged
  source-spelling-dependent one; unavoidable in `macro_rules!`, which
  cannot normalize types), while `#[hopper::state]` uses
  `hopper:wire:v2` canonical stems. Consequence: a `hopper_interface!`
  view can **never** match a `#[hopper::state]`-authored account (and
  vice versa) — cross-path interop silently fails at the layout_id
  compare with no hint. Within one path everything is consistent.
  Action queued: doc warnings at both macros, a `hopper doctor` lint
  (I4), and evaluate a v3 unification (const-eval SHA over canonical
  descriptors is now feasible since `__sha256_const` exists).
- **P2 open (mitigated) — compact auto-disc collisions** — compact
  layouts' only type identity is the 1-byte disc
  (`validate_compact` = length + disc, documented Tier-1 design), and
  the auto-derived disc (first non-zero LAYOUT_ID byte) can collide
  between two same-size compact layouts in one program
  (birthday-paradox over 255 values). Mitigations exist but are
  opt-in: `hopper_register_discs!` (compile-time) and `hopper verify`
  (CLI). Headered layouts are immune (header carries layout_id).
  Action queued for I4: doctor lint that walks a program's layouts and
  requires a disc-uniqueness registration when ≥2 compact layouts
  exist.
- **P3 open `declare_program.rs`** — a manifest whose `size` disagrees
  with its `canonical_type` (e.g. `"u64"` with `size: 2`) generates a
  builder that panics at call time (slice OOB on the const-sized data
  array) instead of failing at expansion. Hopper-generated manifests
  are internally consistent, so latent; cheap fix is a macro-time
  width-vs-size consistency check.
- **P3 note `crank.rs`** — the zero-value-args rule counts every input
  after index 0 but never verifies input 0 *is* a context parameter; a
  handler whose first param is a value arg slips the gate. Cosmetic
  (dispatch codegen rejects it later).
- **verified sound** `program.rs` — longest-prefix-first dispatch
  ordering with exact-duplicate *and* prefix-shadow compile errors
  (neither Anchor nor Quasar detects shadowing); single-byte fast path
  preserves the jump-table `match`; `deny(unsafe_code)` lowering for
  sealed programs; decoder skips exactly `disc_len` bytes and
  `finish()` enforces full consumption. `dynamic_account.rs` — all
  tail decode paths delegate to the audited `TailCodec` runtime with
  checked arithmetic end-to-end; raw-final-tail must be last field,
  singular; UTF-8 validated on `TailStr` writes. `pod.rs` /
  `init_space.rs` / `constant.rs` / `migrate.rs` (forward-only edges,
  fn-pointer-typed migrator) / `dynamic.rs` (metadata-only). The
  declarative `hopper_layout!` five-tier loader family matches the
  audited runtime check semantics tier-for-tier; `hopper_accounts!`
  parses with per-kind checks and correct account-count fencing.

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
