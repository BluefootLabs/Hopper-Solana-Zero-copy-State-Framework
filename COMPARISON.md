# Hopper vs Quasar vs Anchor zero-copy vs Pinocchio

This is a feature-by-feature comparison of Hopper against the three frameworks
it positions against. Every Hopper row cites the concrete file and symbol that
implements it, so the claim can be checked against the tree rather than taken on
faith.

Legend: **Yes** = first-class, implemented and tested. **Partial** = present but
narrower than the leading option, or behind a feature gate. **No** = not
provided by the framework (the dev writes it by hand). **N/A** = out of scope
for that framework's design.

A note on target status: the original matrix was verified 2026-07-07 and the
peer cells below were corrected against the pinned 2026-08-15 source audit.
Quasar's default branch/crate was still v0.0.0, but its active
`0.1.0-release` branch is substantially ahead, uses stable Rust, and remains
self-described beta/unaudited. This Hopper tree is unpublished 0.3.0
development source; registry availability remains a release-time check. Anchor v2 remains
self-described Alpha/unaudited, but `anchor-lang` 2.0.0-rc.1 and tag
`v2.0.0-rc.1` were published 2026-08-12; Anchor 1.2.0 is the stable line as of
2026-09-06.
Use the pinned
[2026-08-15 audit](docs/ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md) rather than
repeating this dated matrix as a permanent ranking. In the tables, “Anchor
1.x / v2” deliberately separates stable 1.x behavior from the v2 release
candidate; an unqualified statement about “Anchor” is not evidence about both.

## Reading the "Hopper implements" column

Symbols are given as `path::Symbol`. Where a capability is delivered by a proc
macro, the macro entry point is in `crates/hopper-macros-proc/src/` and the
runtime it lowers to is in `crates/hopper-runtime/src/` or
`crates/hopper-core/src/`.

---

## Core model

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| `no_std`, no heap on hot path | Yes | Yes | 1.x: No; v2: Yes | Yes | `crates/hopper-runtime` default features = `[]`; verified by `cargo check -p hopper-runtime --no-default-features` |
| No `solana-program` dependency in runtime hot path | Yes | Yes | 1.x: No; v2: Pinocchio-backed | Yes | `crates/hopper-runtime/Cargo.toml` (raw input parsing in `raw_input.rs`-equivalent native backend) |
| Pointer-cast account access (no Borsh, no copies) | Yes | Yes | 1.x: opt-in `AccountLoader`; v2: default mapped accounts | Yes | `crates/hopper-runtime/src/account.rs::AccountView::load` |
| Single-byte instruction discriminator | Yes (1 byte default, multi-byte opt-in) | Yes | Supported through Anchor's custom variable-length discriminators; default remains 8 bytes | Yes | `crates/hopper-macros-proc/src/program.rs` dispatch; `profile = "tiny"` enforces 1-byte |
| Accounts up to 10 MB | Yes | Yes | Yes | Yes | zero-copy path is size-agnostic; no per-byte deserialize |

## Casting & verification

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Verified cast (layout + header + disc + version checked before typed ref) | Yes | Generated account validation around direct views; no Hopper header fingerprint | 1.x/v2 validate owner/discriminator around their mapped account forms, not Hopper's version/layout header | No (manual) | `crates/hopper-runtime/src/account.rs::AccountView::load` / `load_mut`, validated via `LayoutContract` (`layout.rs`) |
| Explicit unchecked/raw hot-path boundary | Yes | Direct-view and internal unchecked boundaries differ from Hopper's tier names | Anchor exposes distinct zero-copy/unsafe or guardrail choices by version | N/A (manual substrate) | Hopper escape symbols are documented in `account.rs`, `context.rs`, and the unsafe ledger; compare guarantees, not matching method names |
| Compile-time Pod / alignment-1 / non-padded enforcement | Yes | Partial | Partial | No | `crates/hopper-macros-proc/src/pod.rs` (`#[hopper::pod]`), `state.rs`; trybuild guards in `tests/compile_fail/` |
| Layout fingerprint (`LAYOUT_ID`) to catch shape drift | Yes | ABI hash in wire tooling, not Hopper's account-header ID | Account discriminator, not a shape fingerprint | No | `LayoutContract::LAYOUT_ID`; cross-program load checks it in `account.rs` |
| Proof-carrying account markers (type-level evidence a check ran) | Yes | No | No | No | `crates/hopper-runtime/src/proof.rs::AccountProof<P>` with `OwnerChecked` / `SignerChecked` / `LayoutChecked<T>` markers; downstream APIs can require the proof instead of hoping a macro emitted the check |

## Segment-level borrows (the differentiator)

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Borrow disjoint byte ranges of one account as independent typed refs | Yes | No | No (whole-account only) | No (manual) | `crates/hopper-runtime/src/account.rs::segment_ref` / `segment_mut` / `segment_ref_typed` |
| Runtime aliasing guard across segment borrows | Yes | N/A | N/A | No | segment borrow registry (`crates/hopper-runtime/src/segment_borrow.rs`, `segment_lease.rs`); conflict tests cover overlap/adjacent/release |
| Const-offset typed segments (zero runtime offset math) | Yes | No | No | No | `segment_ref_typed::<T, const OFFSET>` in `account.rs` / `context.rs` |
| Instruction touch maps (cumulative per-ix `(account, range, R/W)` footprint) | Yes (`touch-map` feature) | No | No | No | `segment_borrow.rs` touch log; `Context::for_each_touch` / `touch_map_len` / `touch_map_overflowed` |
| Field-level write policies (declared write-set enforced at borrow acquire) | Yes (`strict_writes`) | No | No | No (Sealevel account-level `writable` only, all frameworks) | `#[hopper::context(strict_writes)]` → `static WritePolicy` installed in `bind()`; runtime gate in `context.rs::check_write_policy` over `write_policy.rs` |

## On-chain zero-copy collections

Update 2026-09-03: Anchor v2 (published as `anchor-lang` 2.0.0-rc.1, still
self-described Alpha/unaudited) now
ships `Slab<H, T>`, a typed header plus length-prefixed Pod tail, and
bounded `PodVec<T, MAX>`, with `#[kani::proof]` coverage over relevant
capacity arithmetic. Two
bug classes were found and fixed in that surface during May–June 2026
(anchor #4603 "Pad shrunken serialized account tails", 2026-05-27; #4616
"Prevent Slab read aliases during mutable borrows", 2026-06-02); Hopper's
competitor suites now pin both classes (`anchor_4616_*`, `anchor_4603_*`
tests). Hopper ships a broader set of eight collection types, each with
hostile-metadata fuzz coverage; breadth is the claim, not exclusivity.

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Zero-copy collections over account bytes (vec, sorted vec, ring, slab, slot map, packed map, journal, bitset) | Yes (8 families) | Bounded fields and migration views; dynamic mutation repacks its compact tail | 1.x: fixed mapped body; v2: `Slab` and `PodVec` | No | `crates/hopper-core/src/collections/*`; compact-tail aliases in `collections/compact_tail.rs` |
| Corruption-hardened: stored metadata (len/head/count/free lists) validated at construction, rejected when inconsistent | Yes | n/a | n/a | n/a | parse-don't-validate constructors across `collections/*`; slab occupancy/cycle guards |
| Adversarial property harness (arbitrary account bytes → clean `Err`, never panic/OOB) | Yes | Kani/Miri/fuzz workflow; not the same collection-specific claim | v2 has Miri/fuzz/Kani sources; Kani CI disabled at the pin | No framework layer | `collections::hostile_metadata_proptests` (proptest, pinned regression seeds) |
| Element-size honesty proven at compile time (`SIZE == size_of`, non-ZST) | Yes | n/a | n/a | n/a | `FixedLayout::_SIZE_IS_HONEST` (self-proving trait) + `assert_zero_copy_element` |

## Upgradeable state contracts in the pinned comparison

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Schema-versioned accounts (`VERSION` / schema epoch in header) | Yes | Partial: typed migration identity, narrower than Hopper's epoch/fingerprint header | No comparable graph at the pin | No | `LayoutContract::VERSION` + `SCHEMA_EPOCH` (`layout.rs`); 16-byte header written by `write_header_with_epoch` |
| In-place migration edges | Yes | Yes: typed same-size, grow, and shrink migration | App-level; v2 APIs are evolving | No | `crates/hopper-runtime/src/migrate.rs::MigrationEdge`, `LayoutMigration`, `apply_pending_migrations`; `#[hopper::migrate]` macro |
| Migration composition / chain application | Yes | No | No | No | `apply_pending_migrations` walks edges epoch-by-epoch; `hopper::layout_migrations!` composes them |
| Manifest-level migration compatibility analysis | Yes | Partial: wire IDL plus ABI hash | IDL/discriminators, no comparable evolution graph | No | `crates/hopper-schema/src/lib.rs::is_append_compatible` / `requires_migration` / `is_backward_readable` |
| Manifest-backed foreign (cross-program) lenses with 4-way ABI-drift detection (owner, disc, wire fingerprint, schema-epoch range) | Yes | No | No | No | `crates/hopper-runtime/src/foreign.rs::ForeignManifest`; competitors either version-lock on the foreign crate or read blind offsets |

## Receipts & policy

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Bounded receipts summarizing configured snapshot, diff, segment, and version data | Yes | No | No | No | `crates/hopper-core/src/receipt.rs::StateReceipt<SNAP_SIZE>`, `DecodedReceipt`; completeness is limited to supplied scope |
| Receipt decode / explain for off-chain consumers | Yes | No | No | No | `receipt.rs::ReceiptExplain`, `ReceiptNarrative`, `ReceiptIndexRecord` |
| Declarative policy graph evaluated before dispatch | Yes | No | No | No | `crates/hopper-runtime/src/policy.rs::HopperProgramPolicy`, `HopperInstructionPolicy` |
| Per-field lifecycle behavior primitives with declared write-sets and typed successful-check markers | Partial (explicit API; attribute/codegen integration remains proposed) | Partial (side-effect hooks only, no accountability) | No | No | `crates/hopper-runtime/src/behavior.rs::HopperBehavior` exposes `WRITES` and returns `BehaviorChecked<B, O>`; callers must currently incorporate those ranges into policy and invoke the helper explicitly |

## Anchor-parity context ergonomics

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| `#[derive(Accounts)]` analogue | Yes | Yes | Yes | No | `crates/hopper-macros-proc/src/lib.rs::derive_accounts` → `context.rs::expand_for_derive` |
| Constraints: `init`, `mut`, `signer`, `seeds`, `bump`, `has_one`, `owner=`, `address=`, `realloc`, `close`, `constraint` | Yes | Yes | Yes | No | `crates/hopper-macros-proc/src/context.rs` constraint lowering |
| `token::*`, `mint::*`, `associated_token::*`, Token-2022 extension gates | Yes | Partial | Yes | No | `context.rs` + `crates/hopper-spl/*`, `crates/hopper-runtime/src/token_2022_ext.rs` |
| Error model: `#[error_code]`-style derive → `From<E> for ProgramError(Custom(u32))` | Yes | Partial | Yes | No | `crates/hopper-macros-proc/src/error.rs` (`#[hopper::error_code]`); tested in `tests/error_derive_integration.rs` |

## CPI

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Heap-free CPI with const-generic max accounts | Yes | Yes | 1.x: No; v2: typed Pinocchio-backed CPI handles | Yes | `crates/hopper-runtime/src/cpi.rs::invoke_with_bounds::<MAX_ACCOUNTS>` / `invoke_signed_with_bounds` (stack `MaybeUninit` array) |
| Checked CPI wrappers | Yes | Partial | Yes | No | `cpi.rs::invoke_checked` / `invoke_signed_checked` |
| `_unchecked` CPI for hot paths with documented invariants | Yes | N/A | No | Manual | `cpi.rs::invoke_unchecked` / `invoke_signed_unchecked` |
| Typed CPI surface generated from a manifest | Yes | Yes (`declare_program` from wire IDL) | Yes (IDL) | No | `crates/hopper-macros-proc/src/declare_program.rs` (`hopper::declare_program!`) |

## Native substrate surface

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Full System program incl. `*WithSeed` + durable-nonce family | Yes | Partial | Via SDK | Yes | `crates/hopper-native/src/system.rs` (`CreateAccountWithSeed`, `TransferWithSeed`, `Advance/Withdraw/Initialize/Authorize/UpgradeNonceAccount`, typed `NonceState`) |
| Generalized sysvar access (`sol_get_sysvar`, SlotHashes, StakeHistory) | Yes | Partial | Via SDK | Partial | `crates/hopper-native/src/sysvar.rs` (`get_sysvar_into`, `slot_hashes_latest`, `stake_history_latest`, `get_epoch_stake`) |
| secp256r1 / passkey precompile introspection | Yes | No | No | No | `crates/hopper-native/src/introspect.rs::require_secp256r1_instruction`; `crates/hopper-runtime/src/crypto.rs` |
| Token-2022 `ExtraAccountMetaList` resolver (transfer hooks), `no_std`/no-alloc | Yes | No | No | No | `crates/hopper-spl/hopper-token-2022/src/hook.rs::ExtraAccountMetaList::resolve_into` |
| Opt-in bump allocator *and* trap-on-alloc, both first-class | Yes | Partial | N/A | Yes | `default_allocator!` / `no_allocator!`; `crates/hopper-native/src/entrypoint.rs::BumpAllocator` |
| Compile-config ↔ cluster feature-gate check (SIMD-0321) | Yes | No | No | No | `tools/hopper-cli/src/cmd/feature_gate.rs` (`hopper feature-gate`) |

## Schema / IDL

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Machine-readable Hopper manifest and public IDL surfaces | Yes | Yes (wire IDL plus ABI hash) | Yes (IDL) | No | `ProgramManifest` is Hopper's declaration; `ProgramIdl` and the Solana IDL projection are narrower public interfaces, not supersets of one another |
| Zero-copy layouts, segment maps, and upgrade chains | Yes; errors and constants use separate descriptor surfaces | Partial: wire layouts, ABI, CPI/client metadata; no Hopper-equivalent segment/evolution graph | Anchor IDL covers program/account API, not Hopper's graph | No | `LayoutManifest` + `ManifestRegistry`; error/constant descriptors are not fields in `ProgramManifest` or the current CLI's manifest projection |
| Current Solana IDL v0.1 emission | Conditional and fail-closed when the Hopper surface is losslessly representable | Own wire IDL/ABI format | Yes | No | `anchor_idl.rs`; Hopper's u16-bounded vectors and dynamic remaining-account contracts are not silently rewritten |
| On-chain Program Metadata publication | IDL projection only; no generic Hopper manifest/effect registry publisher | No | Not assessed here | No | `hopper publish-idl`; `HopperSchemaPointer` is a data type, not a shipped publication protocol |

## Tooling

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| CLI scaffold / manifest gen / inspect / lint / profile | Yes | Yes: init/build/test/deploy/verify/lint/profile/IDL/client surface | Yes (`anchor` CLI) | No framework CLI | `tools/hopper-cli/src/cmd/*` |
| Client codegen | Six SDKs plus Hopper public IDL, Codama JSON, and conditional Solana IDL: 9 interop formats; full manifest/lowered Rust are separate | Stable Rust/Kit/Web3 plus preview Python/Go/C at the pin | Mature IDL/TS ecosystem; v2 evolving | No | `crates/hopper-schema/src/*client*.rs`, Codama and Solana-IDL emitters |
| In-process SVM integration test harness | Yes | Yes: QuasarSVM Rust/Node/Python | Anchor test tooling; v2 runtime lanes | No framework harness | `crates/hopper-svm`; compiled-SBF lanes are separate evidence |

## Maturity, soundness record, and benchmark culture

Release signals below were verified in the pinned 2026-08-15 audit. Recheck
registries and upstream status before using them in release copy.

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements / evidence |
|---|---|---|---|---|---|
| Release package | Public release 0.2.1; this checkout is unpublished 0.3.0 development source | Default package v0.0.0; 0.1 release branch not tagged/released at 2026-09-06 | Stable 1.2.0 is published; v2 has `anchor-lang` 2.0.0-rc.1 | Published | [crates.io/crates/hopper-lang](https://crates.io/crates/hopper-lang) |
| Audit posture | Internal review and executable evidence trail; no completed independent framework audit | Self-described "Beta … not audited" | Ecosystem audits; scope varies by version/component | Published Neodyme and Zellic review records | `docs/UNSAFE_INVARIANTS.md` and `audit/readiness.json`; external-audit preparation is not an independent review |
| Builds on stable Rust | Yes (pinned 1.96.0) | Yes on the 0.1 release line (Rust 1.89) | Yes | Yes | `rust-toolchain.toml` |
| Soundness/correctness record | Hopper's internally found classes are regression-pinned | The July snapshot recorded #234, #238, #239, #240, and #242; consult the pinned current audit for disposition | v2 Slab classes #4603/#4616 were fixed May–June 2026; active remediation continued at the pinned snapshot | Consult the pinned upstream review records | Hopper pins named classes in `crates/hopper-runtime/tests/competitor_bug_classes.rs` + `crates/hopper-core/tests/competitor_bug_classes.rs`; this row is not a claim that any live tracker has zero issues |
| Competitor-bug-class regression suite (bug class → structural guard → pinned test) | Yes | No | No | No | the two `competitor_bug_classes.rs` suites above; authoring the suite also found and fixed Hopper's own `safe_close` aliased-destination bug |
| Reproducible cross-framework CU benchmark with pinned provenance | Yes: clean five-way same-behavior archive for Hopper `8696640` and benchmark source `af5bc95` | Included as the pinned 0.1 snapshot; no upstream comparative artifact was found at the audit pin | A pre-RC Anchor v2 snapshot is included; its own planned comparison was not published at the audit pin | Pinocchio 0.11.2 is included; no upstream framework comparison artifact was found at the audit pin | [`audit/framework-matrix-2026-08-16.json`](audit/framework-matrix-2026-08-16.json) + `BENCHMARKS.md`; fixture-specific evidence, not a universal ranking |

### Clean same-behavior benchmark snapshot

The 2026-08-16 strict run used clean committed Hopper and benchmark trees,
fresh SBF artifacts, one program id, one state/seed contract, 8 samples, and
passed all 30 rejection gates. The five-way rows are:

| Framework | Deposit CU | Withdraw CU | Binary bytes |
|---|---:|---:|---:|
| Hopper | 1,578 | 424 | 9,032 |
| Quasar 0.1 snapshot | 1,755 | 593 | 5,784 |
| Anchor v2 pre-RC snapshot | 1,785 | 615 | 6,432 |
| Pinocchio 0.11.2 | 3,697 | 2,542 | 7,512 |
| Star Frame 0.30 snapshot | 3,837 | 2,624 | 83,216 |

The archive SHA-256 is
`c64af2460bcbfc0a9a3b8e5a7d8ecdbaa73ff34b7b5d20b0f17e89e44a84f747`.
These numbers describe this vault fixture only. Quasar emits the smallest
binary in the matrix, and the separate Hopper/Pinocchio missing-signature row
is 67/48 CU. See `BENCHMARKS.md` for the complete method, two-way rows, source
pins, and claim boundary.

---

## Where Hopper is differentiated in the pinned comparison

The following four first-class capabilities were not found in the pinned peer
snapshots. This is a dated comparison, not a claim about every framework or a
claim that the ideas cannot be copied:

1. **Segment-level borrows**: disjoint typed `&mut` views into one account.
   Anchor zero-copy only hands you the whole account; Pinocchio leaves it to
   manual pointer math. (`account.rs::segment_mut`, registry in
   `segment_borrow.rs`.)
2. **Upgradeable state contracts**: versioned schemas with composable
   migration edges and manifest-level compatibility analysis.
   (`migrate.rs`, `schema/src/lib.rs::requires_migration`.)
3. **Receipts** - structured summaries over configured snapshots and metadata
   that later code can decode and check. They are not transaction-complete
   proofs without complete supplied scope. (`receipt.rs::StateReceipt`.)
4. **Policy graphs**: authority/capability rules evaluated at the verification
   step, not scattered through handlers. (`policy.rs`.)

## Where parity is the goal (table stakes)

Casting safety, `#[derive(Accounts)]` constraints, the error model, CPI
ergonomics, and schema/IDL emission provide comparable framework-level
workflows. The error model gap (lowering a derived error into
`ProgramError::Custom`) was the most recent parity item closed; see
`crates/hopper-macros-proc/src/error.rs` and `tests/error_derive_integration.rs`.

## Historical devnet records

The deployments below predate the 0.3.0 release source and do not attest it.
They are retained as historical records from authority
`HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT`:

| Example | Devnet program id | SBF bytes |
|---|---|---:|
| counter | `D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F` | 4 688 |
| escrow | `5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN` | 18 736 |
| versioned-state | `EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt` | 25 664 |
| orderbook | `CK3XYYsbFducx9UEEWWLGAVnSAhGkMtM1TKLe8PDP6dJ` | 18 408 |
| smoke | `2YPBvKJ8h37bUEFBrmytzNuKfUJ5Q2o2tkTiqRCZdjme` | 20 280 |

In that historical run, `hopper explain` decoded an escrow `make` transaction against the
checked-in manifest (1 761 CU on devnet), and `hopper migrate` drove a
`LayoutMigration` upgrade against the versioned-state program. The
`smoke` program ran an `initialize` to `deposit` to `withdraw` sequence on
devnet (init writes the layout-fingerprint header and reads the Clock
sysvar; deposit CPIs System `Transfer` and emits a typed event; withdraw
debits program-owned lamports under a `has_one` check). See
`examples/hopper-smoke/README.md` for the confirmed transaction
signatures. See `BENCHMARKS.md` for sizes and the measured CU figure.
The old SOL conversions for these binaries used a one-account
`(bytes + 128) × 6,960` shortcut and are retired. A fresh loader-v3 deployment
locks `R(36) + R(45 + max_len)`, where `R` is the target cluster's live
rent-exemption query. Its temporary Buffer has `37 + ELF_len` data bytes, but
the stock CLI funds it at the ProgramData requirement and loader v3 recycles
that balance during final deployment. See `BENCHMARKS.md` for the current,
slot-pinned Cicada calculation. Compare historical artifacts by byte size unless
all loader, rent, headroom, and fee inputs are recorded.

## Honest gaps

- **Cross-framework CU rows** are release evidence only when tied to the
  benchmark repository's lockfile, raw logs, toolchain, clean source pins, and
  archived checksums. The most recent five-way archive is summarized above.
  It covers Hopper `8696640`, not later source changes, so the final release
  requires a fresh clean rerun before its numbers are called current. Older
  vault and router runs remain historical in `BENCHMARKS.md`.
