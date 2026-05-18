# Parity Matrix Audit â€” Hopper vs Quasar vs Pinocchio

**Date:** 2026-04-27
**Status:** **Local only â€” gitignored under `docs/_audit_*`. Do not commit.**
**Sources:** local checkouts at `d:\tmp\quasar` (blueshift-gg/quasar) and `d:\tmp\pinocchio` (anza-xyz/pinocchio) cross-referenced against Hopper sister repos and the umbrella workspace.
**Prior passes:** `AUDIT.md` (2026-04-24, R1â€“R10), CHANGELOG entries through 2026-04-26.

---

## 0. Repo-structure parity

| Aspect | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| Repo layout | Single workspace | Single workspace | 7 sister repos + 1 umbrella workspace (15 crates) |
| Members | `lang`, `derive`, `pod`, `spl`, `idl`, `profile`, `cli`, `examples/*`, `tests/*` | `sdk`, `programs/{system,token,token-2022,ata,memo}` | runtime, core, macros, derive, spl (token+2022+ata+metaplex), cli, bench + native, system, schema, solana, finance, lending, staking, vesting, distribute, multisig, anchor, manager, sdk, svm, svm-ffi |
| Equivalent of "core lang" | `lang` | `sdk` | `hopper-runtime` + `hopper-core` + `hopper-native` |
| Equivalent of "derive" | `derive` | (none â€” manual) | `hopper-derive` (proc) + `hopper-macros` (decl) |
| Equivalent of "pod" | `pod` | (in-line in `sdk`) | embedded in `hopper-runtime::pod` + `hopper-native::wire` |
| Equivalent of "idl" | `idl` | (none) | `hopper-schema` (Anchor IDL + Codama + manifest) |
| Equivalent of "spl" | `spl` (single crate) | `programs/{token,token-2022,ata,memo}` (4 crates) | `hopper-token`, `hopper-token-2022`, `hopper-associated-token`, `hopper-metaplex` (4 crates) â€” **no `hopper-memo`** |
| Profiler | `profile` (standalone, flamegraph, gist) | (none) | `cu-trace`, `cu_baselines.toml`, `hopper profile bench` (no flamegraph UI) |
| CLI | `cli` (init/build/test/deploy/idl/profile/dump/keys) | (none â€” uses `cargo build-sbf`) | `hopper-cli` (much wider â€” schema, inspect, explain, plan, receipt, manager, fetch, interactive, client gen) |
| Test harness | `tests/suite` + `quasar-svm` | (none â€” relies on mollusk/litesvm) | `hopper-svm` + `hopper-svm-ffi` |
| Off-chain SDK | TS via `quasar idl` | (none) | `hopper-sdk` (Rust) + `hopper-cli client gen --ts` |

### 0.1 Findings on structure
- **Hopper's split is wider than Quasar's by design.** Sister-repo extraction was an explicit choice (per the umbrella `Cargo.toml` comment) and gives independent versioning + smaller publishable units. This is **not a regression**; the Quasar monorepo style is convenient for them because their surface is ~10Ã- narrower.
- **Single ergonomic gap from the split:** end users add many lines of `Cargo.toml` deps. Mitigated by the umbrella `hopper` crate re-exporting common paths. Verify the umbrella's `prelude` covers >90% of Day-One symbols.
- **Concrete missing helper crate:** `hopper-memo` (Pinocchio has it; SPL parity).
- Everything else in Hopper's split is value-add over Quasar: domain primitives (finance/lending/â€¦), `hopper-anchor` interop, SVM harness + FFI, schema/manifest tooling.

---

## 1. Surface parity matrix

Legend: âœ… = first-class; ~ = present but partial / different shape; âŒ = missing; **bold** = unique to that framework.

### 1.1 Account / state model

| Capability | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| Zero-copy `#[repr(C)]` overlays | âœ… via `#[account]` ZC companion | ~ (DIY) | âœ… `#[hopper::state]` + `hopper_layout!` |
| Discriminator | âœ… explicit (any byte length) + all-zero rejection | âŒ | âœ… explicit (`disc: u8` + 8-byte Anchor-compat) |
| **All-zero discriminator rejected at compile time** | âœ… | n/a | **VERIFY** â€” likely missing |
| **Discriminator collision detection at macro time** | âœ… | n/a | **VERIFY** â€” may be partial |
| Layout fingerprint / type-id | âŒ | âŒ | âœ… **layout_id (SHA-256 / FNV-1a)** |
| Account header (`HopperHeader`) | âŒ | âŒ | âœ… |
| Segmented account body | âŒ | âŒ | âœ… **segment_map + TypedSegment** |
| Segment-level borrow registry (disjoint mut + read on same account) | âŒ | âŒ | âœ… **`SegmentBorrowRegistry`** |
| Dynamic inline fields (`String<P,N>`, `Vec<T,P,N>`, tail) | âœ… all three with configurable prefix | âŒ | ~ `tail::TailCodec` (single tail), no inline `String<P,N>`/`Vec<T,P,N>` shape |
| Layout migration / versioning | âŒ | âŒ | âœ… **`MigrationEdge` / `LayoutMigration`** |
| Virtual state (N-account logical entity) | ~ via `Interface<T>` (single-account variant) | âŒ | âœ… **`VirtualState<N>`** |
| Policy-aware capabilities | âŒ | âŒ | âœ… **`Capability` / `InstructionPolicy`** |
| State receipts (64-byte mutation summary) | âŒ | âŒ | âœ… **`StateReceipt`** |
| Diff snapshots | âŒ | âŒ | âœ… |
| Frame phased execution (resolve/validate/borrow/mutate/emit) | âŒ | âŒ | âœ… **`Frame`** |

### 1.2 Account wrappers

| Wrapper | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| `Account<T>` | âœ… | n/a | âœ… |
| `Program<T>` | âœ… | n/a | âœ… |
| `Signer` | âœ… | n/a | âœ… |
| `SystemAccount` | âœ… | n/a | âœ… (`SystemId`) |
| `Sysvar<T>` | âœ… | âœ… via `Sysvar::get()` | âœ… via `sysvar` module |
| `UncheckedAccount` | âœ… | (everything is unchecked) | ~ (`HopperRefOnly` + `AccountView`) |
| **`Interface<T>` (multi-program polymorphism)** | âœ… | âŒ | **âŒ â€” gap** |
| **`InterfaceAccount<T>` (Token + Token-2022)** | âœ… | âŒ | **âŒ â€” gap** (workaround: explicit owner check) |

### 1.3 Derive / attribute surface

| Attribute | Anchor 0.31 | Quasar | Hopper |
|---|---|---|---|
| `#[program]` | âœ… | âœ… | âœ… `#[hopper::program]` |
| `#[account]` | âœ… | âœ… (with ZC companion gen) | âœ… `#[hopper::state]` |
| `#[derive(Accounts)]` | âœ… | âœ… | ~ `#[hopper::context]` (attribute, not derive) |
| `#[event]` | âœ… | âœ… + `emit_cpi!` self-CPI | âœ… `#[hopper::event]` |
| `#[error_code]` / errors | âœ… | âœ… | âœ… `#[hopper::error]` |
| `#[instruction]` (handler attr) | ~ (positional) | âœ… explicit disc | ~ via `#[hopper::program]` body |
| `#[interface]` (CPI peer) | âœ… | ~ (`Interface<T>`) | ~ `declare_program!` |
| **`#[access_control(expr)]`** | âœ… | âŒ | **âŒ â€” gap** |
| **`#[constant]`** | âœ… | âŒ | **âŒ â€” gap** |
| `#[view]` / query | âœ… (0.31 beta) | âŒ | âŒ |
| `#[derive(InitSpace)]` | âœ… | n/a | âœ… `#[hopper::init_space]` |
| `#[crank]` | âŒ | âŒ | âœ… **`#[hopper::crank]`** |
| `#[migrate]` | âŒ | âŒ | âœ… **`#[hopper::migrate]`** |
| `#[invariant]` | âŒ | âŒ | âœ… **`hopper_invariant!`** |
| `#[policy]` | âŒ | âŒ | âœ… **`hopper::policy::*`** |
| Account constraints DSL (`mut`, `signer`, `init`, `init_if_needed`, `close`, `payer`, `space`, `has_one`, `constraint`, `address`, `seeds`, `bump`, `realloc`, `sweep`, `token::*`, `associated_token::*`, `mint::*`, `metadata::*`, `master_edition::*`) | âœ… | âœ… entire set | ~ partial â€” verify token::* / associated_token::* / mint::* / metadata::* coverage in `#[hopper::context]` |

### 1.4 CPI

| Feature | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| Const-generic stack-allocated CPI | âœ… `CpiCall<A,D>` | âŒ (uses `solana_instruction_view`) | ~ Hopper has `dyn_cpi`/`invoke`/`invoke_signed`; **no const-generic stack-only builder** |
| Variable-length builder | âœ… `BufCpiCall` | âŒ | âœ… |
| System program CPI helpers | âœ… | âœ… (full set, 14 instructions) | âœ… via `hopper-system` |
| Token program CPI helpers | âœ… | âœ… | âœ… via `hopper-token` |
| Token-2022 CPI helpers | âœ… | âœ… | âœ… via `hopper-token-2022` |
| ATA helpers | âœ… | âœ… | âœ… via `hopper-associated-token` |
| **Memo helpers** | âŒ | âœ… | **âŒ â€” gap** |
| Stack-friendly Borsh encoder (`BorshString`/`BorshVec`) | âœ… | âŒ | âŒ |
| **Verified CPI (snapshot+verify)** | âŒ | âŒ | âœ… **`LamportSnapshot` + `DataFingerprint`** (in `hopper-native::verify`) |
| Typed CPI return data (`invoke_and_read::<T>`) | ~ via `set_return_data` | via `set_return_data` | âœ… **`hopper-native::return_data`** |

### 1.5 Macros / runtime helpers

| Helper | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| `require!` / `require_eq!` / `require_keys_eq!` | âœ… | âŒ (DIY) | âœ… + `require_neq`, `require_keys_neq`, `require_gte`, `require_gt`, `require_lt`, `require_lte` |
| `emit!` (log) | âœ… ~100 CU | n/a | âœ… |
| `emit_cpi!` (self-CPI, spoof-resistant) | âœ… | n/a | ~ `cpi_event.rs` exists â€” **VERIFY parity with `emit_event_cpi()`** |
| Discriminator-based dispatch macro | âœ… `dispatch!` | âœ… (manual match) | âœ… `hopper_dispatch!` + `hopper_dispatch_lazy!` |
| Lazy entrypoint | âŒ | âœ… `lazy_program_entrypoint!` | âœ… `hopper_lazy_entrypoint!` (eager too) |
| `set_return_data` syscall | âœ… | âœ… | âœ… |
| `sol_curve_validate_point` PDA fast-path (~544 CU) | âœ… | ~ (uses `find_program_address`) | **VERIFY â€” `hopper-native::pda`** |
| Bump-from-account-data optimization | âœ… (auto via `bump: u8` field detection) | âŒ | ~ `hopper_verify_pda!` exists, may need bump-field auto-detection |

### 1.6 IDL / schema / TS

| Feature | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| IDL JSON | âœ… via `quasar idl` | âŒ | âœ… `hopper-schema::anchor_idl` |
| Codama emitter | âŒ | âŒ | âœ… **`hopper-schema::codama`** |
| TS client gen | âœ… | âŒ | âœ… `hopper client gen --ts` |
| Discriminator collision detection at IDL-emit | âœ… | n/a | **VERIFY** |
| On-chain manifest PDA | âŒ | âŒ | âœ… **`MANIFEST_SEED` / `MANIFEST_MAGIC`** |

### 1.7 Tooling & test harness

| Feature | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| In-process SVM | âœ… `quasar-svm` | âŒ | âœ… `hopper-svm` |
| BPF execution | âœ… | âŒ | âœ… Phase-2 (feature-gated) |
| FFI bindings | âŒ | âŒ | âœ… **`hopper-svm-ffi`** |
| Static binary profiler / flamegraph | âœ… **`quasar profile` (gist publish)** | âŒ | ~ `cu_baselines.toml` + `hopper profile bench` (no flamegraph) |
| Watch mode (`build --watch`, `test --watch`) | âœ… | n/a | ~ verify `hopper-cli` |
| Cross-framework vault bench | ~ (single repo) | âŒ | âœ… `hopper-bench` (anchor + pinocchio + quasar + hopper) |

### 1.8 Domain primitives

| Primitive | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| Finance (CP-AMM, slippage) | âŒ | âŒ | âœ… |
| Lending (collateral, health, liquidation) | âŒ | âŒ | âœ… |
| Staking (MasterChef-style) | âŒ | âŒ | âœ… |
| Vesting (linear, cliff, periodic) | âŒ | âŒ | âœ… |
| Distribute (largest-remainder dust-safe) | âŒ | âŒ | âœ… |
| Multisig (M-of-N) | âŒ (example only) | âŒ | âœ… |

### 1.9 Off-chain & introspection

| Feature | Quasar | Pinocchio | Hopper |
|---|---|---|---|
| Symmetric off-chain SDK | âŒ | âŒ | âœ… **`hopper-sdk`** |
| Receipt narration | âŒ | âŒ | âœ… |
| Segment-aware partial readers | âŒ | âŒ | âœ… |
| Manifest-driven instruction builders | âŒ | âŒ | âœ… |

---

## 2. DX / Performance / Safety / Innovation roll-up

### 2.1 DX
- **Quasar wins** on (a) onboarding-velocity for an Anchor refugee due to `#[derive(Accounts)]` + `#[account(...)]` constraint DSL covering all token/mint/ATA/metadata lifecycle attrs in one place, (b) `quasar profile` integration, (c) the polymorphic `Interface<T>` / `InterfaceAccount<T>` for Token+Token-2022 in a single declaration.
- **Hopper wins** on (a) declarative-only path via `hopper_layout!`/`hopper_dispatch!` (no proc macros required), (b) richer CLI (schema/inspect/explain/plan/receipt/interactive), (c) symmetric off-chain SDK, (d) domain primitive crates.
- **Hopper gaps**: missing `Interface<T>` polymorphism, missing `#[access_control]` and `#[constant]`, no flamegraph profiler. Some `#[hopper::context]` constraints (`token::mint=`, `associated_token::*`, `mint::decimals=`, `metadata::*`, `master_edition::*`) coverage needs verification.

### 2.2 Performance
- **Quasar wins** on (a) const-generic stack-allocated CPI (`CpiCall<A,D>`), (b) ~544-CU PDA path via direct `sol_curve_validate_point`, (c) header-as-u32 single-compare account validation, (d) inline-Borsh encoders for CPI data without temp buffers, (e) bump-from-account-data auto-derivation.
- **Hopper wins** on (a) lazy entrypoint pattern (parses on demand), (b) segment-level concurrent disjoint borrows reducing redundant overlay work, (c) typed CPI return data, (d) `cu-trace` budget snapshots, (e) zero-copy collections that avoid heap entirely.
- **Action items**: confirm `hopper-native::pda` uses the `sol_curve_validate_point` direct path; if not, port. Add a `CpiCall<const A, const D>` const-generic stack builder mirroring Quasar's pattern.

### 2.3 Safety
- **Quasar wins** on compile-time guarantees: all-zero discriminator rejected at proc-macro expansion, discriminator collision detected across instructions+accounts, dynamic-field ordering enforced (fixed-then-dynamic, no nested dynamics, single tail), alignment-1 sentinels.
- **Hopper wins** on runtime safety: layout fingerprints catch wrong-type loads, segment borrow registry prevents aliasing, frame-phased execution forbids validate-after-mutate, state receipts give post-hoc audit, policy capabilities prevent missing-guard bugs, **verified CPI** at the substrate level (LamportSnapshot + DataFingerprint).
- **Hopper gaps**: confirm `#[hopper::state]` rejects all-zero discriminators at expansion; confirm collision detection across program's account+event+instruction surface.

### 2.4 Innovation
- **Hopper-only**: Frame phases, segment borrows, virtual state, layout migrations, state receipts, policy capabilities, hopper-sdk receipt narration, verified CPI substrate, three-backend pluggability, on-chain manifest PDAs, hopper-svm + ffi, domain primitives, codama emitter alongside Anchor IDL.
- **Quasar-only**: Static binary profiler with web flamegraph + gist publish, const-generic stack-only CPI, `Interface<T>` polymorphism (only single-account flavour; Hopper's `VirtualState` is broader but doesn't cover Token/Token-2022 case-by-case), inline `String<P,N>` / `Vec<T,P,N>` ZC types with configurable prefix.
- **Pinocchio-only**: Pure dependency-zero core, `lazy_program_entrypoint!` (Hopper has equivalent via `hopper_lazy_entrypoint!`), Memo helper crate.

---

## 3. Concrete fix list (post-verification pass)

Each candidate gap was source-verified before listing. Items confirmed already-shipped were dropped.

### Already shipped (no action â€” audit row was wrong)

- âœ… **`#[hopper::access_control(expr)]`** â€” `hopper-derive/src/program.rs:54,702-713,873-888`. Wraps handlers with ANDed boolean gates, returns `MissingRequiredSignature` on false.
- âœ… **All-zero discriminator rejection** â€” `hopper-derive/src/state.rs:196,314` (`validate_discriminator_not_zero`).
- âœ… **Per-program discriminator collision detection** â€” `hopper-derive/src/program.rs:194` (exact-match collision) and **prefix-shadow detection** at `:208` (rejects 1-byte tag that's a prefix of an 8-byte tag). Stricter than Quasar's same-length-only check.
- âœ… **`init_if_needed` / `close = dest` / `realloc = new_size` / `realloc_payer` / `realloc_zero`** â€” `hopper-derive/src/context.rs:43-71`.
- âœ… **`token::mint` / `token::authority` / `token::token_program`** â€” `hopper-derive/src/context.rs:108-125`.
- âœ… **`mint::decimals` / `mint::authority` / `mint::freeze_authority`** â€” `hopper-derive/src/context.rs:127-137`.
- âœ… **`associated_token::mint/authority/token_program`** â€” `hopper-derive/src/context.rs:138-145`.
- âœ… **PDA fast verify path** â€” `hopper-runtime/src/pda.rs:115-200` does a sha256-only verify loop (skips `sol_curve_validate_point`) for the verify case. Costs ~200 CU at bump=255 vs Quasar's ~544 CU `sol_curve_validate_point` â€” **Hopper is faster than Quasar on the verify path** (Quasar still calls curve_validate; Hopper exploits "we already know the address, just compare hashes").

### P0 â€” Real, confirmed gaps

1. **`hopper-memo` SPL helper crate.** Pinocchio ships a `programs/memo` crate; Quasar's SPL helper is monolithic and skips memo. Hopper has no Memo-program helper. Add a tiny new crate at `crates/hopper-memo` modeled on `pinocchio-memo`: one `Memo<'a>` instruction builder, variable signer slice, raw memo bytes. **Cost:** ~80 LOC.

2. **`InterfaceAccount<TokenInterface>` polymorphism.** Quasar's headline DX win is `InterfaceAccount<Token>` accepting either SPL Token or Token-2022 owner via a single owner check. Hopper has separate `token.rs` (165-byte SPL layout) + `token2022_ext.rs` (extension screening) + ATA derivation per program but no unified wrapper. Land in `hopper-solana` as a thin parser-shaped wrapper: validate owner âˆˆ {`TOKEN_PROGRAM_ID`, `TOKEN_2022_PROGRAM_ID`}, expose `mint() / owner() / amount() / state()` reader methods that work on both. **Cost:** ~150 LOC + tests.

### P1 â€” DX parity, smaller wins

3. **`metadata::*` / `master_edition::*` constraints** in `#[hopper::context]`. Quasar covers `metadata::name/symbol/uri/seller_fee_basis_points` + `master_edition::max_supply`. Hopper covers token/mint/ATA/Token-2022 lifecycle but not Metaplex. Plumb through `hopper-derive::context` and lower to `hopper-metaplex` builder calls. **Cost:** medium. Defer this pass â€” Metaplex builders already exist in `hopper-metaplex`; just need attribute plumbing.

4. **`#[hopper::constant]` IDL exporter.** Mark module-level `pub const` items so `hopper-schema::anchor_idl` includes them in IDL/manifest output. Anchor parity, NOT in Quasar. **Cost:** small. Defer â€” touches both `hopper-derive` and `hopper-schema`.

5. **`hopper::prelude` audit.** Confirm umbrella prelude covers the Day-One symbol set. **Cost:** trivial. Defer â€” verify after items 1+2 are in.

### P2 â€” Performance polish

6. **Const-generic stack CPI builder.** Quasar's `CpiCall<const ACCTS, const DATA>` pattern. Hopper's existing `invoke`/`invoke_signed` are already stack-shaped (no Vec); add a const-generic builder layer for the Anchor-style `Cpi<'_, MyProgram>::transfer(..)` ergonomic. **Cost:** medium. Defer.

7. **Inline-Borsh CPI encoder.** `BorshString<'a>` / `BorshVec<'a>`. Real but niche; defer.

8. **Static binary profiler / flamegraph.** Quasar's `quasar profile`. Largest surface, lowest marginal value vs `cu_baselines.toml`. Defer indefinitely.

### P2 â€” Innovation backlog (Hopper-only, not parity)

9. **`#[hopper::view]`** read-only query handler. Anchor 0.31-parity, NOT in Quasar. Defer.
10. **`hopper-pod` crate extraction.** Cosmetic; current shape fine. Defer.

---

## 4. This-pass implementation

Land P0.1 (`hopper-memo`) and P0.2 (`InterfaceAccount<TokenInterface>`). Both are localized, additive, and address the most-cited Quasar/Pinocchio parity gaps.

Order:
1. **P0.1 â€” `hopper-memo` crate.** Self-contained; smallest blast radius.
2. **P0.2 â€” `InterfaceAccount<TokenInterface>`** in `hopper-solana::token`. Adds a polymorphic reader struct + owner-check helper.

P1 + P2 stay in this audit doc as the next-pass roadmap. No commits â€” local only.

---

## 5. Landed in this pass

### 5.1 `hopper-memo` (P0.1) â€” DONE

New crate at [`crates/hopper-memo`](../crates/hopper-memo/Cargo.toml). 113 LOC of `lib.rs`, no dependencies beyond `hopper-runtime`. Exports:
- `MEMO_PROGRAM_ID` (Memo v2) and `v1::MEMO_V1_PROGRAM_ID` (legacy), both compile-time decoded via `hopper_runtime::address!`.
- `MAX_MEMO_SIGNERS = 16` (matches Pinocchio's `MAX_STATIC_CPI_ACCOUNTS`).
- `Memo<'a, 'b, 'c> { signers, memo, program_id: Option<&Address> }` builder with `.invoke()` / `.invoke_signed(&[Signer])`.
- Stack-allocated CPI via `MaybeUninit<InstructionAccount>` + `invoke_signed_with_bounds::<MAX_MEMO_SIGNERS>`. Heap-free.

Wired into the umbrella workspace `Cargo.toml` `[workspace.members]` and `[workspace.dependencies]`. `cargo check -p hopper-memo` clean.

### 5.2 `InterfaceTokenAccount` / `InterfaceMint` (P0.2) â€” DONE

New module at [`crates/hopper-solana/src/interface.rs`](../crates/hopper-solana/src/interface.rs). 240 LOC including 3 unit tests. Exports:
- `TokenProgramKind { Spl, Token2022 }` with `program_id() -> &'static Address`, `from_owner(&Address) -> Result<Self, ProgramError>`, and `for_account(&AccountView) -> Result<Self, ProgramError>` (uses safe `owned_by` rather than the `unsafe owner()` accessor).
- `InterfaceTokenAccount<'a> { data, kind }` with `from_data(&[u8], TokenProgramKind) -> Result<Self, ProgramError>` constructor and reader methods (`mint()`, `owner()`, `amount()`, `state()`) plus assertion helpers (`assert_initialized`, `assert_owner`, `assert_mint`).
- `InterfaceMint<'a> { data, kind }` analogous shape (`supply()`, `decimals()`, `authority()`, `freeze_authority()`, `assert_initialized`).
- `interface_transfer_checked` / `interface_transfer_checked_signed` polymorphic CPI helpers that pick the right program id from the source account's owner and forward through the standard checked-CPI path. Instruction layout (`[12, amount: u64, decimals: u8]`) is shared between SPL Token and Token-2022.

Wired into [`crates/hopper-solana/src/lib.rs`](../crates/hopper-solana/src/lib.rs) under `pub mod interface;`. `cargo check -p hopper-solana` clean. Three unit tests pass:
- `token_program_kind_from_owner_matches_known_programs`
- `token_program_kind_from_owner_rejects_other_programs`
- `token_program_kind_program_id_is_stable`

### 5.3 P1.5 prelude audit â€” DONE

`hopper-memo` and `hopper-solana::interface` now flow through the umbrella prelude.

- Wired `hopper-memo` into the umbrella `Cargo.toml` `[dependencies]` and into all three backend feature lists (`hopper-native-backend`, `legacy-pinocchio-compat`, `solana-program-backend`).
- [`src/prelude.rs`](../src/prelude.rs) re-exports `hopper_memo` plus `Memo`, `MEMO_PROGRAM_ID`, `MAX_MEMO_SIGNERS`, and `hopper_solana::interface::{InterfaceMint, InterfaceTokenAccount, TokenProgramKind, interface_transfer_checked, interface_transfer_checked_signed}`.
- `cargo check -p hopper` clean (only pre-existing unrelated unused-import warnings).

### 5.4 `#[hopper::constant]` IDL surface (P1.4) â€” DONE

Anchor-parity `#[constant]` plus an explicit IDL bridge.

- New [`hopper-derive/src/constant.rs`](../../hopper-derive/src/constant.rs): `expand` preserves the original `pub const` and emits a sibling `pub const __HOPPER_CONST_<NAME>: hopper_schema::ConstantDescriptor` capturing name, stringified type, stringified initializer, and concatenated doc-comment text.
- Registered `#[proc_macro_attribute] hopper_constant` + short alias `constant` in [`hopper-derive/src/lib.rs`](../../hopper-derive/src/lib.rs); umbrella re-exports both under the `proc-macros` feature.
- New [`crates/hopper-schema/src/lib.rs`](../crates/hopper-schema/src/lib.rs) `ConstantDescriptor { name, ty, value, docs }` (additive â€” no change to existing `ProgramIdl` / `ProgramManifest` field layouts).
- New [`crates/hopper-schema/src/anchor_idl.rs`](../crates/hopper-schema/src/anchor_idl.rs) `AnchorIdlWithConstants` and `AnchorIdlFromManifestWithConstants` wrappers that take a `&'a [ConstantDescriptor]` slice alongside the IDL and emit a `"constants"` array between events and errors.
- End-to-end test [`tests/constant_integration.rs`](../tests/constant_integration.rs) â€” 4/4 passing: original const usable, descriptor metadata correct, IDL JSON renders constants array, empty slice renders `"constants": []`.

### 5.5 `metadata::*` / `master_edition::*` context constraints (P1.3) â€” DONE

Metaplex context attributes now lower through the existing `hopper-metaplex` builders instead of being a manual-programming gap.

- [`hopper-derive/src/context.rs`](../../hopper-derive/src/context.rs) parses `metadata::{name,symbol,uri,seller_fee_basis_points,is_mutable,mint,mint_authority,payer,update_authority,system_program,rent}` and `master_edition::{max_supply,mint,metadata,update_authority,mint_authority,payer,token_program,system_program,rent}`.
- Metadata validators construct `hopper_metaplex::DataV2::simple(...)` and call `validate_for_context()` so oversized Metaplex strings fail before CPI.
- Master-edition validators accept both `u64` and `Option<u64>` via `IntoMasterEditionMaxSupply`, so `max_supply = 0` and `max_supply = None::<u64>` are both valid surfaces.
- Generated bound-context helpers invoke `CreateMetadataAccountV3` / `CreateMasterEditionV3` from the declared sibling account fields, threading `#[instruction(...)]` args through the helper signatures.
- [`hopper-spl/hopper-metaplex/src/instructions.rs`](../../hopper-spl/hopper-metaplex/src/instructions.rs) exposes `DataV2::validate_for_context()` and `IntoMasterEditionMaxSupply` for macro-generated code.
- [`tests/metaplex_context_integration.rs`](../tests/metaplex_context_integration.rs) â€” 2/2 passing under `--features "proc-macros,metaplex"`.

### 5.6 Items NOT landed this pass (deferred)

- **P2 items** â€” performance polish (const-generic CPI, inline-Borsh, profiler) â€” backlog.

Audit refreshed through P1.3. No commits â€” this file lives at `docs/_audit_*` per `.gitignore`.
