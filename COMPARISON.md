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
self-described beta/unaudited; no Quasar ref has a commit after `d981ac8`
(2026-08-02), rechecked 2026-09-19. This Hopper tree is unpublished 0.3.0
development source; registry availability remains a release-time check. Anchor v2 remains
self-described Alpha/unaudited: `anchor-lang` 2.0.0-rc.1 and tag
`v2.0.0-rc.1` were published 2026-08-12 and are still the newest v2 release
as of 2026-09-19, while the unreleased v2 branch (`abacd0e`, 2026-09-18) has
landed about thirty correctness commits since 2026-09-05. Anchor 1.2.0 is the
stable line. Pinocchio 0.11.2 remains its newest release.
Use the pinned
[2026-08-15 audit](docs/ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md) and the
[2026-09-19 refresh](docs/COMPETITIVE_REFRESH_2026-09-19.md) rather than
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
| No `solana-program` dependency in runtime hot path | Yes | Yes | 1.x: No; v2: Pinocchio-backed | Yes | `crates/hopper-runtime/Cargo.toml`; raw input parsing lives in `crates/hopper-native/src/raw_input.rs` |
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
bug classes were found and fixed in that surface during May to June 2026
(anchor #4603 "Pad shrunken serialized account tails", 2026-05-27; #4616
"Prevent Slab read aliases during mutable borrows", 2026-06-02); Hopper's
competitor suites now pin both classes (`anchor_4616_*`, `anchor_4603_*`
tests). Hopper ships a broader set of eight collection types, each with
hostile-metadata fuzz coverage; breadth is the claim, not exclusivity.

Update 2026-09-19: the unreleased v2 branch (`abacd0e`, 2026-09-18) landed
about thirty correctness commits after 2026-09-05. Five were checked against
Hopper's tree. Close to a non-writable destination (#4886), CPI-handle
validation (#5043), and cfg-gated discriminator collisions (#5015) did not
apply. Slab post-shrink length (#4906) led to a hardening:
`Slab::from_bytes_mut` now refuses a stored count above capacity. The
tail-slab minimum-length class (#4888) did apply: `safe_realloc` could shrink
an account below the length its own layout needs to load, which bricks it
with the rent locked. It was fixed with `safe_realloc_bounded`
(`crates/hopper-core/src/account/lifecycle.rs`) and a `required_len()` floor
in every generated `realloc_<field>()` accessor, and pinned in the regression
suite on 2026-09-19 (`anchor_4886_*`, `anchor_5043_*`, `anchor_4906_*`,
`anchor_4888_*` tests; #5015 was assessed as not applicable by construction
and has no pinned test). Hopper versions before that commit shared the #4888
class.

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
| Full System program incl. `CreateAccountAllowPrefund`, `*WithSeed`, and the durable-nonce family | Yes | Partial | Via SDK; v1 master (#5057) and v2 (#4945) adopted `CreateAccountAllowPrefund` for account creation on 2026-09-17 | Yes | `crates/hopper-native/src/system.rs` (`CreateAccountAllowPrefund`, System instruction 13; `CreateAccountWithSeed`, `TransferWithSeed`, `Advance/Withdraw/Initialize/Authorize/UpgradeNonceAccount`, typed `NonceState`). Since 2026-09-19 `hopper_init!` (`init` and `init_if_needed`) creates every account with one `CreateAccountAllowPrefund` CPI, replacing the CreateAccount branch and the Transfer, Allocate, Assign fallback; account creation is no longer a difference from Anchor |
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
| CLI scaffold / manifest gen / inspect / lint / profile / verify | Yes | Yes: init/build/test/deploy/verify/lint/profile/IDL/client surface | Yes (`anchor` CLI) | No framework CLI | `tools/hopper-cli/src/cmd/*` |
| ELF size regression gate with absolute and relative thresholds | Yes | CU/binary budget gates on the 0.1 release line | Not assessed here | No | `hopper profile elf --baseline <folded> --fail-on-growth <bytes> --fail-on-growth-pct <pct>` (`tools/hopper-cli/src/cmd/profile.rs`): exit 2 when `.text` grew by more than both thresholds |
| Release-bound upgrade-authority diff (declared permissions that widen between two manifests fail the release) | Yes | Not found at the 2026-09-19 pin | Not found at the 2026-09-19 pin; the separate Ratchet 0.4.0 IDL tool answers client compatibility and scores a signer or writable flag relaxing to false as additive | No | `crates/grillo-manifest/src/authority.rs::AuthorityDiff`; `grillo authority-diff old new`; `hopper verify --authority-baseline old` with `--baseline-so` / `--baseline-program` and `--candidate-buffer` / `--candidate-program` (`tools/hopper-cli/src/cmd/verify.rs`); exit 2 on an unapproved widening, 3 on an unapproved review item. It compares declarations, not bytecode |
| Client codegen | Six SDKs plus Hopper public IDL, Codama JSON, and conditional Solana IDL: 9 interop formats; full manifest/lowered Rust are separate | Stable Rust/Kit/Web3 plus preview Python/Go/C at the pin | Mature IDL/TS ecosystem; v2 evolving | No | `crates/hopper-schema/src/*client*.rs`, Codama and Solana-IDL emitters |
| In-process SVM integration test harness | Yes | Yes: QuasarSVM Rust/Node/Python | Anchor test tooling; v2 runtime lanes | No framework harness | `crates/hopper-svm`; compiled-SBF lanes are separate evidence |

## Maturity, soundness record, and benchmark culture

Release signals below were verified in the pinned 2026-08-15 audit. Recheck
registries and upstream status before using them in release copy.

| Capability | Hopper | Quasar 0.1 release line | Anchor 1.x / v2 alpha | Pinocchio | Hopper implements / evidence |
|---|---|---|---|---|---|
| Release package | Public release 0.2.1; this checkout is unpublished 0.3.0 development source | Default package v0.0.0; 0.1 release branch not tagged/released at 2026-09-19 | Stable 1.2.0 is published; v2 has `anchor-lang` 2.0.0-rc.1 (still newest at 2026-09-19) | Published (0.11.2 newest at 2026-09-19; `main` unreleased) | [crates.io/crates/hopper-lang](https://crates.io/crates/hopper-lang) |
| Audit posture | Internal review and executable evidence trail; no completed independent framework audit | Self-described "Beta … not audited" | Ecosystem audits; scope varies by version/component | Published Neodyme and Zellic review records | `docs/UNSAFE_INVARIANTS.md` and `audit/readiness.json`; external-audit preparation is not an independent review |
| Builds on stable Rust | Yes (pinned 1.96.0) | Yes on the 0.1 release line (Rust 1.89) | Yes | Yes | `rust-toolchain.toml` |
| Soundness/correctness record | Hopper's internally found classes are regression-pinned; Anchor v2's #4888 realloc-below-minimum class applied to Hopper too and was fixed and pinned on 2026-09-19 | The July snapshot recorded #234, #238, #239, #240, and #242; consult the pinned current audit for disposition | v2 Slab classes #4603/#4616 were fixed May to June 2026; the unreleased v2 branch landed about thirty further correctness commits between 2026-09-05 and 2026-09-18 (#4886, #5043, #5015, #4906, #4888 among them) | Consult the pinned upstream review records | Hopper pins named classes in `crates/hopper-runtime/tests/competitor_bug_classes.rs` + `crates/hopper-core/tests/competitor_bug_classes.rs` (22 tests at 2026-09-19); this row is not a claim that any live tracker has zero issues |
| Competitor-bug-class regression suite (bug class → structural guard → pinned test) | Yes | No | No | No | the two `competitor_bug_classes.rs` suites above; authoring the suite found and fixed Hopper's own `safe_close` aliased-destination bug, and extending it on 2026-09-19 found and fixed Hopper's own instance of the #4888 realloc-below-minimum class |
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
is 67/48 CU. The Hopper binary in this archive was built with
`crate-type = ["cdylib", "lib"]`, which keeps LTO off for the on-chain
artifact; program crates moved to `["cdylib"]` alone on 2026-09-19 (see the
dated note in `BENCHMARKS.md`). The archived figures stand as archived
evidence and are not restated here. See `BENCHMARKS.md` for the complete
method, two-way rows, source pins, and claim boundary.

### Hopper rows in pina's fixtures (2026-09-21)

pina's `benchmarks/framework-comparison` measures a hello world and a PDA
counter under one release recipe and one Mollusk verifier with post-state
checks. `bench/framework-comparison/` reproduces that recipe for Hopper and
rebuilds pina's pinocchio fixtures as the cross-check, which reproduced pina's
published numbers exactly (3,160 B / 111 CU; 6,512 B / 1,490 / 1,721 CU).

| Fixture | Hopper (substrate) | Hopper (macro) | Pinocchio | Pina | Quasar | Anchor v2 |
|---|---:|---:|---:|---:|---:|---:|
| hello, bytes / CU | 1,656 / 116 | 2,376 / 186 | 3,160 / 111 | 4,680 / 145 | 2,520 / 115 | 1,880 / 127 |
| counter, bytes / init / increment | 8,448 / 1,681 / 1,762 | 11,408 / 3,207 / 1,748 | 6,512 / 1,490 / 1,721 | 13,024 / 3,301 / 1,753 | 7,808 / 3,488 / 330 | 8,696 / 3,458 / 2,117 |

The substrate counter is the like-for-like row (10-byte compact account,
plain `CreateAccount`, PDA re-derived on `increment`). The macro counter
carries Hopper's 16-byte header (25-byte account) like Anchor's 24-byte one,
and its `initialize` includes a live Rent sysvar read. Quasar's `increment`
skips the PDA re-derivation. Hopper does not win everywhere: the macro hello
costs 70 CU over the substrate and the macro counter binary is larger than
Anchor's and Quasar's; see `bench/framework-comparison/results/RESULTS.md`
and `docs/COMPETITIVE_REFRESH_2026-09-21.md`.

---

## Where Hopper is differentiated in the pinned comparison

The following five first-class capabilities were not found in the pinned peer
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
5. **Release-bound authority diff** (added 2026-09-19): a declared-permission
   widening between two manifests fails `hopper verify --authority-baseline`,
   and either side can be bound to a deployed ProgramData or Buffer ELF.
   Searches of the repositories named in the 2026-09-19 refresh found no
   public tool with that polarity; Ratchet scores relaxations as additive.
   That is a scoped finding, not a proof that none exists.
   (`crates/grillo-manifest/src/authority.rs`.)

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
that balance during final deployment. See `BENCHMARKS.md` for the
slot-pinned Cicada calculation and its 2026-09-19 rent note (5,080 lamports
per byte since 2026-09-11). Compare historical artifacts by byte size unless
all loader, rent, headroom, and fee inputs are recorded.

## Honest gaps

- **Cross-framework CU rows** are release evidence only when tied to the
  benchmark repository's lockfile, raw logs, toolchain, clean source pins, and
  archived checksums. The most recent five-way archive is summarized above.
  It covers Hopper `8696640`, not later source changes, and its Hopper
  binary was built with the dual crate type that kept LTO off, so the final
  release requires a fresh clean rerun before its numbers are called current. Older
  vault and router runs remain historical in `BENCHMARKS.md`.
