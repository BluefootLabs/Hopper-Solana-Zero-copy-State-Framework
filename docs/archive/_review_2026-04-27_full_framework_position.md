# Hopper framework position review - 2026-04-27

Local-only working note. This file is under `docs/_*` and is not intended for commit.

## 1. Scope

Reviewed:

- Hopper umbrella workspace: `d:\tmp\Hopper-Solana-Zero-copy-State-Framework`
- Hopper sister repos: `hopper-runtime`, `hopper-core`, `hopper-macros`, `hopper-derive`, `hopper-spl`, `hopper-cli`, `hopper-bench`
- Quasar local checkout: `d:\tmp\quasar`
- Pinocchio local checkout: `d:\tmp\pinocchio`
- Blueshift public GitHub org repo list (59 public repos as of 2026-04-27)
- Anchor zero-copy account model and account constraints concepts

Goal: make Hopper the easiest high-performance zero-copy Solana framework to write, while preserving original code, zero-copy/no-heap execution, and Hopper-only innovation.

## 2. Blueshift / Quasar repo architecture observed on GitHub

Relevant public repos in the Blueshift org:

| Repo | Role | Takeaway for Hopper |
| --- | --- | --- |
| `blueshift-gg/quasar` | Main framework workspace: `lang`, `derive`, `spl`, `cli`, `idl`, `schema`, `profile`, `metadata`, examples, tests, Kani runner | Strong integrated DX and proof/testing posture. |
| `blueshift-gg/quasar-docs` | Dedicated docs site | Docs are treated as a product surface, not an afterthought. |
| `blueshift-gg/quasar-svm` | SVM execution engine with FFI/bindings/npm | End-to-end local testing matters for onboarding. |
| `blueshift-gg/zeropod` | Alignment-1 zero-copy Pod types + derive | Alignment-safe wire types are a first-class brand pillar. Hopper already has this via `Wire*`; keep it highly visible. |
| `blueshift-gg/wincode` | In-place encoding/serialization project with derive/fuzz/audits | There is room for Hopper to own no-alloc encoding stories around dynamic tails and client payloads. |
| `blueshift-gg/codama-rs` | Rust Codama node/IDL extraction stack | IDL/codegen ecosystem integration is strategic. Hopper already has `hopper-schema`; keep Codama output excellent. |
| `blueshift-gg/mollusk` | SVM program test harness | Smooth test harness UX is a DX moat. Hopper has `hopper-svm`; expose it through `hopper test` clearly. |
| `blueshift-gg/beethoven` | CPI interface/client routing | Interface-based CPI discovery is an ecosystem-level pattern. Hopper's `hopper_interface!` and typed CPI should be documented and scaffolded. |
| Toolchain/forks (`pinocchio`, `agave`, `solana-sdk`, `llvm-project`, `sbpf-*`) | Low-level stack control | They invest below the framework layer. Hopper should keep backend pluggability and SBF build discipline sharp. |

## 3. Hopper current strengths

### DX surface

- `hopper init` already exists with interactive and non-interactive project scaffolding.
- `hopper add -i/-s/-e` already exists and wires instructions/state/errors idempotently.
- `#[hopper::state]`, `#[hopper::context]`, `#[derive(Accounts)]`, `#[hopper::program]`, `#[hopper::args]`, `#[hopper::event]`, `#[hopper::error]`, `#[hopper::constant]`, `#[hopper::dynamic]` provide a broad proc-macro surface.
- `hopper-schema` already supports manifest/IDL/Codama-style projections and TS/Kotlin/Rust client generation.
- `hopper-cli` already has inspect/explain/schema/client/profile/verify/lint/doctor/build/test/deploy/clean/keys/config/expand flows.

### Performance / zero-copy

- Runtime and core are designed around direct account-data overlays, alignment-1 wire types, fixed-capacity collections, and no heap on-chain.
- `no_allocator!()` and `nostd_panic_handler!()` exist and examples use them.
- Segment-level borrow registry is a genuine differentiator: byte-range borrows on a single account, not only whole-account access.
- PDA verification supports stored-bump fast path and backend pluggability.
- CPI helpers use stack-bounded account/data buffers.

### Safety

- Sealed zero-copy trait path prevents arbitrary unsafe casts.
- Layout fingerprints in account headers enable strict loading and cross-program compatibility checks.
- Context macro enforces account constraints in deterministic order.
- Receipts, policies, invariants, migration planning, compatibility checks, and virtual state are Hopper-only differentiation.

## 4. What Quasar / Pinocchio / Anchor still teach us

| Area | External lesson | Hopper response |
| --- | --- | --- |
| Onboarding | Quasar makes scaffold/add/test/profile feel like one workflow. | Hopper has init/add; tighten docs, generated templates, lint/doctor output, and test harness messaging. |
| Proof posture | Quasar uses Kani/Miri around unsafe/CPI paths. | Add formal proof jobs for `MaybeUninit` CPI builders, segment borrow registry, and layout casts. |
| Zero-alloc guarantee | Pinocchio exposes `no_allocator!` as a visible contract. | Hopper has macro support; CLI should detect missing markers in program crates. |
| Raw CU discipline | Pinocchio keeps account parsing lazy and feature gates narrow. | Keep `hopper_lazy_entrypoint!`, add docs/examples, and ensure templates can opt into lazy parsing. |
| Zero-copy ergonomics | Anchor's `AccountLoader` is familiar; Quasar's account syntax is concise. | Keep Anchor-spelled aliases and improve lint/compiler diagnostics around context attributes. |
| Dynamic data | Quasar-style compact/fixed capacity fields are easy to explain. | Hopper has dynamic tail + collections; add simpler docs and maybe alias attributes later. |
| Testing UX | Mollusk/quasar-svm present test harness as a first-class command. | `hopper test` should surface backend/test harness choice and clean diagnostics. |
| Docs | Quasar has a dedicated docs repo. | Hopper needs a docs/product pass, especially quickstart and cookbook recipes. |

## 5. Current gap ranking

### P0 - should land now or soon

1. **Lint support for new Metaplex context attributes.** Context macro now parses `metadata::*` / `master_edition::*`, but `hopper lint` needs matching diagnostics.
2. **Zero-allocation lint.** Program crates that define an entrypoint should warn if they do not call `no_allocator!()` and `nostd_panic_handler!()`.
3. **Current review doc.** Keep the real status matrix local and accurate so future work does not chase stale gaps.

### P1 - next competitive layer

1. Add Kani/Miri targets for unsafe/CPI/borrrow invariants.
2. Improve `hopper test` messaging around SVM backends and generated templates.
3. Add a docs quickstart path mirroring the shortest Quasar flow: init → add instruction → test → inspect → generate client.
4. Add optional lazy-entrypoint template or init flag.
5. Add CLI profile output with artifact-size delta and benchmark delta summaries.

### P2 - innovation polish

1. Dynamic-tail authoring sugar for String/Vec-like fields with fixed-capacity and compact modes.
2. Go/Python client generation if ecosystem demand appears.
3. More Token-2022 extension attribute coverage: group pointer, group member pointer, confidential transfer.
4. Batch CPI helpers where high-throughput token flows benefit.

## 6. Selected implementation for this pass

Implement P0.1 + P0.2 in `hopper-cli`:

- Teach `hopper lint` to understand `metadata::*` and `master_edition::*` constraints.
- Emit errors for partial/incoherent Metaplex attribute sets before users hit proc-macro failures.
- Emit warnings when a program-like crate has no `no_allocator!()` or no `nostd_panic_handler!()` marker.
- Add unit coverage inside the lint module.

This is small, high-value, and directly supports the goal that Hopper programs are easy to author while staying zero-copy/zero-heap by default.

## 7. Landed in this pass

- `hopper-cli/src/cmd/lint.rs` now tracks `metadata::*` and `master_edition::*` keys while scraping context attributes.
- `hopper lint` now emits direct errors for partial metadata data declarations, incomplete metadata CPI helper declarations, mixed metadata/master-edition keywords on one field, and incomplete master-edition helper declarations.
- `hopper lint` now detects program-like crates and warns when `no_allocator!()` or `nostd_panic_handler!()` markers are absent.
- ASCII and JSON lint reports expose crate-level marker status; JSON field entries expose Metaplex key sets for downstream tooling.
- Added focused unit tests in the lint module for entrypoint detection, metadata key scraping, partial metadata diagnostics, and complete master-edition declarations.
- Added `hopper-cli` local sister-repo patches for `hopper-core` and `hopper-runtime` so standalone CLI validation resolves the local workspace graph.

Validation note: `cargo test --bin hopper cmd::lint::tests -- --nocapture` passed in `hopper-cli` (4 passed, 0 failed). Remaining output was pre-existing warnings in `hopper-runtime`, `hopper-schema`, and unrelated CLI modules.
