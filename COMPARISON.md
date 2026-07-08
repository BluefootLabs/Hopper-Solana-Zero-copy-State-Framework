# Hopper vs Quasar vs Anchor zero-copy vs Pinocchio

This is a feature-by-feature comparison of Hopper against the three frameworks
it positions against. Every Hopper row cites the concrete file and symbol that
implements it, so the claim can be checked against the tree rather than taken on
faith.

Legend: **Yes** = first-class, implemented and tested. **Partial** = present but
narrower than the leading option, or behind a feature gate. **No** = not
provided by the framework (the dev writes it by hand). **N/A** = out of scope
for that framework's design.

A note on the comparison targets' status (verified 2026-07-07): Quasar is
v0.0.0 — no tags, no releases, not on crates.io, self-described "Beta … not
audited", nightly-only toolchain. Pinocchio is audited (Neodyme and Zellic,
2025-06) and production-proven. Anchor has years of mainnet mileage. Hopper is
published (hopper-lang 0.2.1 on crates.io) on stable Rust with a line-by-line
audit trail (`AUDIT.md`, `docs/UNSAFE_INVARIANTS.md`). Weight the "Yes" cells
accordingly.

## Reading the "Hopper implements" column

Symbols are given as `path::Symbol`. Where a capability is delivered by a proc
macro, the macro entry point is in `crates/hopper-macros-proc/src/` and the
runtime it lowers to is in `crates/hopper-runtime/src/` or
`crates/hopper-core/src/`.

---

## Core model

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| `no_std`, no heap on hot path | Yes | Yes | No | Yes | `crates/hopper-runtime` default features = `[]`; verified by `cargo check -p hopper-runtime --no-default-features` |
| No `solana-program` dependency in runtime hot path | Yes | Yes | No | Yes | `crates/hopper-runtime/Cargo.toml` (raw input parsing in `raw_input.rs`-equivalent native backend) |
| Pointer-cast account access (no Borsh, no copies) | Yes | Yes | Yes (marked accounts only) | Yes | `crates/hopper-runtime/src/account.rs::AccountView::load` |
| Single-byte instruction discriminator | Yes (1 byte default, multi-byte opt-in) | Yes | No (8-byte sighash) | Yes | `crates/hopper-macros-proc/src/program.rs` dispatch; `profile = "tiny"` enforces 1-byte |
| Accounts up to 10 MB | Yes | Yes | Yes | Yes | zero-copy path is size-agnostic; no per-byte deserialize |

## Casting & verification

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Verified cast (layout + header + disc + version checked before typed ref) | Yes | Codegen-time only | Pod + disc only | No (manual) | `crates/hopper-runtime/src/account.rs::AccountView::load` / `load_mut`, validated via `LayoutContract` (`layout.rs`) |
| `_unchecked` hot-path escape hatch, `#[inline(always)]` | Yes | N/A | No | N/A (all manual) | `account.rs::borrow_unchecked` / `borrow_unchecked_mut`; `context.rs::raw_unchecked`; each with a `// SAFETY:` invariant |
| Compile-time Pod / alignment-1 / non-padded enforcement | Yes | Partial | Partial | No | `crates/hopper-macros-proc/src/pod.rs` (`#[hopper::pod]`), `state.rs`; trybuild guards in `tests/compile_fail/` |
| Layout fingerprint (`LAYOUT_ID`) to catch shape drift | Yes | No | No | No | `LayoutContract::LAYOUT_ID`; cross-program load checks it in `account.rs` |
| Proof-carrying account markers (type-level evidence a check ran) | Yes | No | No | No | `crates/hopper-runtime/src/proof.rs::AccountProof<P>` with `OwnerChecked` / `SignerChecked` / `LayoutChecked<T>` markers; downstream APIs can require the proof instead of hoping a macro emitted the check |

## Segment-level borrows (the differentiator)

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Borrow disjoint byte ranges of one account as independent typed refs | Yes | No | No (whole-account only) | No (manual) | `crates/hopper-runtime/src/account.rs::segment_ref` / `segment_mut` / `segment_ref_typed` |
| Runtime aliasing guard across segment borrows | Yes | N/A | N/A | No | segment borrow registry (`crates/hopper-runtime/src/segment_borrow.rs`, `segment_lease.rs`); conflict tests cover overlap/adjacent/release |
| Const-offset typed segments (zero runtime offset math) | Yes | No | No | No | `segment_ref_typed::<T, const OFFSET>` in `account.rs` / `context.rs` |
| Instruction touch maps (cumulative per-ix `(account, range, R/W)` footprint) | Yes (`touch-map` feature) | No | No | No | `segment_borrow.rs` touch log; `Context::for_each_touch` / `touch_map_len` / `touch_map_overflowed` |
| Field-level write policies (declared write-set enforced at borrow acquire) | Yes (`strict_writes`) | No | No | No (Sealevel account-level `writable` only, all frameworks) | `#[hopper::context(strict_writes)]` → `static WritePolicy` installed in `bind()`; runtime gate in `context.rs::check_write_policy` over `write_policy.rs` |

## On-chain zero-copy collections (no competitor ships any)

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Zero-copy collections over account bytes (vec, sorted vec, ring, slab, slot map, packed map, journal, bitset) | Yes (8) | No | No | No | `crates/hopper-core/src/collections/*`; compact-tail aliases in `collections/compact_tail.rs` |
| Corruption-hardened: stored metadata (len/head/count/free lists) validated at construction, rejected when inconsistent | Yes | n/a | n/a | n/a | parse-don't-validate constructors across `collections/*`; slab occupancy/cycle guards |
| Adversarial property harness (arbitrary account bytes → clean `Err`, never panic/OOB) | Yes | No | No | No | `collections::hostile_metadata_proptests` (proptest, pinned regression seeds) |
| Element-size honesty proven at compile time (`SIZE == size_of`, non-ZST) | Yes | n/a | n/a | n/a | `FixedLayout::_SIZE_IS_HONEST` (self-proving trait) + `assert_zero_copy_element` |

## Upgradeable state contracts (no competitor has this first-class)

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Schema-versioned accounts (`VERSION` / schema epoch in header) | Yes | No | No | No | `LayoutContract::VERSION` + `SCHEMA_EPOCH` (`layout.rs`); 16-byte header written by `write_header_with_epoch` |
| In-place migration edges | Yes | No | No | No | `crates/hopper-runtime/src/migrate.rs::MigrationEdge`, `LayoutMigration`, `apply_pending_migrations`; `#[hopper::migrate]` macro |
| Migration composition / chain application | Yes | No | No | No | `apply_pending_migrations` walks edges epoch-by-epoch; `hopper::layout_migrations!` composes them |
| Manifest-level migration compatibility analysis | Yes | No | No | No | `crates/hopper-schema/src/lib.rs::is_append_compatible` / `requires_migration` / `is_backward_readable` |
| Manifest-backed foreign (cross-program) lenses with 4-way ABI-drift detection (owner, disc, wire fingerprint, schema-epoch range) | Yes | No | No | No | `crates/hopper-runtime/src/foreign.rs::ForeignManifest`; competitors either version-lock on the foreign crate or read blind offsets |

## Receipts & policy

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Structural receipts proving an ix touched a segment/version | Yes | No | No | No | `crates/hopper-core/src/receipt.rs::StateReceipt<SNAP_SIZE>`, `DecodedReceipt` |
| Receipt decode / explain for off-chain consumers | Yes | No | No | No | `receipt.rs::ReceiptExplain`, `ReceiptNarrative`, `ReceiptIndexRecord` |
| Declarative policy graph evaluated before dispatch | Yes | No | No | No | `crates/hopper-runtime/src/policy.rs::HopperProgramPolicy`, `HopperInstructionPolicy` |
| Per-field lifecycle behaviors that contribute write-sets and return proof tokens | Yes | Partial (side-effect hooks only, no accountability) | No | No | `crates/hopper-runtime/src/behavior.rs::HopperBehavior` — `WRITES` feeds the `strict_writes` policy; checks return `BehaviorChecked<B, O>` |

## Anchor-parity context ergonomics

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| `#[derive(Accounts)]` analogue | Yes | Yes | Yes | No | `crates/hopper-macros-proc/src/lib.rs::derive_accounts` → `context.rs::expand_for_derive` |
| Constraints: `init`, `mut`, `signer`, `seeds`, `bump`, `has_one`, `owner=`, `address=`, `realloc`, `close`, `constraint` | Yes | Yes | Yes | No | `crates/hopper-macros-proc/src/context.rs` constraint lowering |
| `token::*`, `mint::*`, `associated_token::*`, Token-2022 extension gates | Yes | Partial | Yes | No | `context.rs` + `crates/hopper-spl/*`, `crates/hopper-runtime/src/token_2022_ext.rs` |
| Error model: `#[error_code]`-style derive → `From<E> for ProgramError(Custom(u32))` | Yes | Partial | Yes | No | `crates/hopper-macros-proc/src/error.rs` (`#[hopper::error_code]`); tested in `tests/error_derive_integration.rs` |

## CPI

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Heap-free CPI with const-generic max accounts | Yes | Yes | No | Yes | `crates/hopper-runtime/src/cpi.rs::invoke_with_bounds::<MAX_ACCOUNTS>` / `invoke_signed_with_bounds` (stack `MaybeUninit` array) |
| Checked CPI wrappers | Yes | Partial | Yes | No | `cpi.rs::invoke_checked` / `invoke_signed_checked` |
| `_unchecked` CPI for hot paths with documented invariants | Yes | N/A | No | Manual | `cpi.rs::invoke_unchecked` / `invoke_signed_unchecked` |
| Typed CPI surface generated from a manifest | Yes | No | Yes (IDL) | No | `crates/hopper-macros-proc/src/declare_program.rs` (`hopper::declare_program!`) |

## Native substrate surface

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Full System program incl. `*WithSeed` + durable-nonce family | Yes | Partial | Via SDK | Yes | `crates/hopper-native/src/system.rs` (`CreateAccountWithSeed`, `TransferWithSeed`, `Advance/Withdraw/Initialize/Authorize/UpgradeNonceAccount`, typed `NonceState`) |
| Generalized sysvar access (`sol_get_sysvar`, SlotHashes, StakeHistory) | Yes | Partial | Via SDK | Partial | `crates/hopper-native/src/sysvar.rs` (`get_sysvar_into`, `slot_hashes_latest`, `stake_history_latest`, `get_epoch_stake`) |
| secp256r1 / passkey precompile introspection | Yes | No | No | No | `crates/hopper-native/src/introspect.rs::require_secp256r1_instruction`; `crates/hopper-runtime/src/crypto.rs` |
| Token-2022 `ExtraAccountMetaList` resolver (transfer hooks), `no_std`/no-alloc | Yes | No | No | No | `crates/hopper-spl/hopper-token-2022/src/hook.rs::ExtraAccountMetaList::resolve_into` |
| Opt-in bump allocator *and* trap-on-alloc, both first-class | Yes | Partial | N/A | Yes | `default_allocator!` / `no_allocator!`; `crates/hopper-native/src/entrypoint.rs::BumpAllocator` |
| Compile-config ↔ cluster feature-gate check (SIMD-0321) | Yes | No | No | No | `tools/hopper-cli/src/cmd/feature_gate.rs` (`hopper feature-gate`) |

## Schema / IDL

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Machine-readable schema manifest (superset of Anchor IDL) | Yes | No | Yes (IDL only) | No | `crates/hopper-schema/src/lib.rs::LayoutManifest`, `ProgramManifest`, `ProgramIdl` |
| Covers zero-copy layouts, segment maps, upgrade chains, errors, constants | Yes | No | No | No | `LayoutManifest` + `ManifestRegistry`; constants via `#[hopper::constant]` |
| Anchor-compatible IDL emission | Yes | No | Yes | No | `clientgen.rs` / `rust_client.rs` / `python_client.rs` emitters |
| On-chain schema publication (manifest stored in account) | Yes | No | No | No | `hopper-schema/src/lib.rs` manifest account format (header + JSON payload, optional zlib) |

## Tooling

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| CLI scaffold / manifest gen / inspect / lint / profile | Yes | Partial | Yes (`anchor` CLI) | No | `tools/hopper-cli/src/cmd/*` |
| Client codegen (TS / Kotlin / Python / Rust) | Yes | Partial | Yes (TS) | No | `crates/hopper-schema/src/{clientgen,rust_client,python_client}.rs` |
| In-process SVM integration test harness | Yes | Partial | Yes | No | `crates/hopper-test/src/lib.rs::LiteSvmHarness` (mollusk-backed); used by the gated devnet tests |

## Maturity, soundness record, and benchmark culture

Facts verified 2026-07-07 against public trackers and registries; see
`docs/audit/GAP_CLOSURE_AND_INNOVATION_2026.md` section 2 for sources.

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements / evidence |
|---|---|---|---|---|---|
| Published release on crates.io | Yes (0.2.1) | No (v0.0.0, no tags or releases) | Yes | Yes | [crates.io/crates/hopper-lang](https://crates.io/crates/hopper-lang) |
| Audit posture | Line-by-line internal audit trail | Self-described "Beta … not audited" | Ecosystem audits | Audited (Neodyme, Zellic 2025-06) | `AUDIT.md`, `docs/UNSAFE_INVARIANTS.md` |
| Builds on stable Rust | Yes (pinned 1.96.0) | No (nightly-only bespoke toolchain) | Yes | Yes | `rust-toolchain.toml` |
| Open soundness/correctness issues on tracker (2026-07) | None open; classes regression-pinned | 5 (blueshift-gg/quasar #234, #238, #239, #240, #242) | tracked upstream | none open | Hopper pins those classes in `crates/hopper-runtime/tests/competitor_bug_classes.rs` + `crates/hopper-core/tests/competitor_bug_classes.rs` (13 tests) |
| Competitor-bug-class regression suite (bug class → structural guard → pinned test) | Yes | No | No | No | the two `competitor_bug_classes.rs` suites above; authoring the suite also found and fixed Hopper's own `safe_close` aliased-destination bug |
| Publishes reproducible cross-framework CU benchmark with pinned provenance | Yes | No (no published numbers) | No (otter-sec/anchor #4355 planned, unpublished) | No | `hopper-bench` results + provenance blocks in `BENCHMARKS.md` |

---

## Where Hopper wins outright

The four rows no competitor offers first-class are the thesis of the framework:

1. **Segment-level borrows** — disjoint typed `&mut` views into one account.
   Anchor zero-copy only hands you the whole account; Pinocchio leaves it to
   manual pointer math. (`account.rs::segment_mut`, registry in
   `segment_borrow.rs`.)
2. **Upgradeable state contracts** — versioned schemas with composable
   migration edges and manifest-level compatibility analysis.
   (`migrate.rs`, `schema/src/lib.rs::requires_migration`.)
3. **Receipts** — structural proof tokens an instruction emits and a later
   instruction verifies. (`receipt.rs::StateReceipt`.)
4. **Policy graphs** — authority/capability rules evaluated at the verification
   step, not scattered through handlers. (`policy.rs`.)

## Where parity is the goal (table stakes)

Casting safety, `#[derive(Accounts)]` constraints, the error model, CPI
ergonomics, and schema/IDL emission exist to remove any reason to reach for
Anchor or Quasar. The error model gap (lowering a derived error into
`ProgramError::Custom`) was the most recent parity item closed; see
`crates/hopper-macros-proc/src/error.rs` and `tests/error_derive_integration.rs`.

## Devnet evidence (this pass)

The framework now backs the comparison with programs running on devnet,
deployed from authority `HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT`:

| Example | Devnet program id | SBF bytes |
|---|---|---:|
| counter | `D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F` | 4 688 |
| escrow | `5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN` | 18 736 |
| versioned-state | `EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt` | 25 664 |
| orderbook | `CK3XYYsbFducx9UEEWWLGAVnSAhGkMtM1TKLe8PDP6dJ` | 18 408 |
| smoke | `2YPBvKJ8h37bUEFBrmytzNuKfUJ5Q2o2tkTiqRCZdjme` | 20 280 |

`hopper explain` decodes a real escrow `make` transaction against the
checked-in manifest (1 761 CU on devnet), and `hopper migrate` drove a
`LayoutMigration` upgrade against the versioned-state program. The
`smoke` program ran a live `initialize → deposit → withdraw` sequence on
devnet (init writes the layout-fingerprint header and reads the Clock
sysvar; deposit CPIs System `Transfer` and emits a typed event; withdraw
debits program-owned lamports under a `has_one` check) — see
`examples/hopper-smoke/README.md` for the confirmed transaction
signatures. See `BENCHMARKS.md` for sizes and the measured CU figure.
At the network's `(bytes + 128) × 6,960` lamport rent formula, the
4,688-byte counter costs about **0.034 SOL** of rent-exempt deploy rent;
an Anchor-class 190 KiB artifact costs ~1.36 SOL (`BENCHMARKS.md`,
deploy-cost economics).

## Honest gaps

- **Cross-framework CU comparison rows** are intentionally kept out of this
  document. They are only release-grade when tied to the benchmark repository's
  reproducibility envelope (lockfile, raw logs, toolchain). The artifact sizes
  and single-program on-chain CU above were produced directly in this devnet
  pass; the same-lockfile competitor matrix lives in `hopper-bench` — current
  release-facing runs are the 2026-07-07 vault four-way
  (`hopper-bench/results/framework-vaults-2026-07-07-post-ep/`) and router
  three-way (`hopper-bench/results/router-parity-2026-07-07-post-review/`). See
  `BENCHMARKS.md` and `AUDIT.md` R2/RSK-4.
