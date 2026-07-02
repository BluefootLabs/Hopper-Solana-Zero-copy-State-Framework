# Hopper vs Quasar vs Anchor zero-copy vs Pinocchio

This is a feature-by-feature comparison of Hopper against the three frameworks
it positions against. Every Hopper row cites the concrete file and symbol that
implements it, so the claim can be checked against the tree rather than taken on
faith.

Legend: **Yes** = first-class, implemented and tested. **Partial** = present but
narrower than the leading option, or behind a feature gate. **No** = not
provided by the framework (the dev writes it by hand). **N/A** = out of scope
for that framework's design.

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

## Segment-level borrows (the differentiator)

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Borrow disjoint byte ranges of one account as independent typed refs | Yes | No | No (whole-account only) | No (manual) | `crates/hopper-runtime/src/account.rs::segment_ref` / `segment_mut` / `segment_ref_typed` |
| Runtime aliasing guard across segment borrows | Yes | N/A | N/A | No | segment borrow registry (`crates/hopper-runtime/src/segment_borrow.rs`, `segment_lease.rs`); conflict tests cover overlap/adjacent/release |
| Const-offset typed segments (zero runtime offset math) | Yes | No | No | No | `segment_ref_typed::<T, const OFFSET>` in `account.rs` / `context.rs` |
| Instruction touch maps (cumulative per-ix `(account, range, R/W)` footprint) | Yes (`touch-map` feature) | No | No | No | `segment_borrow.rs` touch log; `Context::for_each_touch` / `touch_map_len` / `touch_map_overflowed` |
| Field-level write policies (declared write-set enforced at borrow acquire) | Yes (`strict_writes`) | No | No | No (Sealevel account-level `writable` only, all frameworks) | `#[hopper::context(strict_writes)]` → `static WritePolicy` installed in `bind()`; runtime gate in `context.rs::check_write_policy` over `write_policy.rs` |

## Upgradeable state contracts (no competitor has this first-class)

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Schema-versioned accounts (`VERSION` / schema epoch in header) | Yes | No | No | No | `LayoutContract::VERSION` + `SCHEMA_EPOCH` (`layout.rs`); 16-byte header written by `write_header_with_epoch` |
| In-place migration edges | Yes | No | No | No | `crates/hopper-runtime/src/migrate.rs::MigrationEdge`, `LayoutMigration`, `apply_pending_migrations`; `#[hopper::migrate]` macro |
| Migration composition / chain application | Yes | No | No | No | `apply_pending_migrations` walks edges epoch-by-epoch; `hopper::layout_migrations!` composes them |
| Manifest-level migration compatibility analysis | Yes | No | No | No | `crates/hopper-schema/src/lib.rs::is_append_compatible` / `requires_migration` / `is_backward_readable` |

## Receipts & policy

| Capability | Hopper | Quasar | Anchor zc | Pinocchio | Hopper implements |
|---|---|---|---|---|---|
| Structural receipts proving an ix touched a segment/version | Yes | No | No | No | `crates/hopper-core/src/receipt.rs::StateReceipt<SNAP_SIZE>`, `DecodedReceipt` |
| Receipt decode / explain for off-chain consumers | Yes | No | No | No | `receipt.rs::ReceiptExplain`, `ReceiptNarrative`, `ReceiptIndexRecord` |
| Declarative policy graph evaluated before dispatch | Yes | No | No | No | `crates/hopper-runtime/src/policy.rs::HopperProgramPolicy`, `HopperInstructionPolicy` |

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

## Honest gaps

- **Cross-framework CU comparison rows** are intentionally kept out of this
  document. They are only release-grade when tied to the benchmark repository's
  reproducibility envelope (lockfile, raw logs, toolchain). The artifact sizes
  and single-program on-chain CU above were produced directly in this devnet
  pass; the same-lockfile competitor matrix lives in `hopper-bench`. See
  `BENCHMARKS.md` and `AUDIT.md` R2/RSK-4.
