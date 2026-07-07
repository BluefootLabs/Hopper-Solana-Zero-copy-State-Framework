# Research coverage: zero-copy unification & Anchor v2 adaptation

This note tracks the recommendations from the two research passes
(`HOPPER_UNIFIED_RESEARCH.md` and `ANCHOR_V2_HOPPER_ADAPTATION.md`) against
what is actually implemented in the tree, and where. It is the audit companion
to [`HOPPER_UNIFIED_ZERO_COPY.md`](HOPPER_UNIFIED_ZERO_COPY.md).

Every item below derives from the one `AccountDescriptor` source of truth and
stays `no_std` / `const` / no-alloc unless noted. The compact `[disc][body]`
hot path is unchanged: all additions are off-hot-path projections, off-chain
emitters, or CLI tooling.

## Status legend

- **Done** — implemented and unit-tested.
- **Partial** — core landed; a downstream consumer is still a follow-up.
- **Out of scope** — intentionally not copied from Anchor (rationale given).

## Commit surface

| Commit | Theme |
|--------|-------|
| `e455ce2` | `AccountDescriptor` + `LayoutDescriptor` + `diff_descriptors_vs_registry` (one source of truth) |
| `d6e3673` | `fingerprint`, loaded-data budgeting, tail-aware diff, cost lint, `idl_node` |
| `d9018a3` | `DescriptorMetadata` / `decode_allowed` / JSON emitters; `SchemaExport::descriptor()` |
| *this commit* | `DescriptorGroup` (composite), `AccountExpectation` (typed checks), group JSON emitter, CLI fail-closed `layout_id` guard |

## Unified zero-copy research (`HOPPER_UNIFIED_RESEARCH.md`)

| Recommendation | Status | Where |
|----------------|--------|-------|
| Single developer-facing model (declare a layout once → loader + registry + offsets + schema + upgrade gate) | Done | `manifest.rs` `AccountDescriptor` / `LayoutDescriptor`; macro emits the impl |
| One-source-of-truth `AccountDescriptor` const | Done | `manifest.rs` `AccountDescriptor` (`compact`/`headered`/`with_dynamic_tail`/`deprecated`) |
| Hot-path validation with zero registry reads, zero CU regression | Done | `AccountDescriptor::validate` (`#[inline(always)]`, len+disc only) |
| Governed upgrade gate (generated descriptors vs on-chain registry) | Done | `diff_descriptors_vs_registry` + `ManifestProfile::permits_upgrade` |
| Client decode fingerprint (per-type identity) | Done | `AccountDescriptor::fingerprint` → `LayoutFingerprint` (excludes `deprecated` bit) |
| `setLoadedAccountsDataSizeLimit` sourced from descriptors | Done | `min_loaded_data_size` / `recommend_loaded_data_limit` |
| Dynamic-tail-aware registry diff (capacity bump ≠ migration) | Done | `classify_entry_change` / `diff_descriptors_vs_registry_detailed` |
| CoW / account-data-cost layout lint | Done | `cost_profile` / `cost_lint` / `SizeClass` |
| IDL / Codama export hook from the descriptor | Done | `idl_node` → `DescriptorIdlNode` |
| Off-chain metadata export wired to the emitter | Done | `hopper_schema::DescriptorMetadata` + `codama::DescriptorMetadataJson` / `DescriptorMetadataSetJson` |
| Per-language client generators consume the JSON | Partial | JSON is emitted; generators embedding `fingerprint` + `recommend_loaded_data_limit` remain a follow-up |

## Anchor v2 adaptation (`ANCHOR_V2_HOPPER_ADAPTATION.md`)

### P0

| # | Recommendation | Status | Where / rationale |
|---|----------------|--------|-------------------|
| 1 | Inputs-only account view + `setLoadedAccountsDataSizeLimit` hint in client codegen | Partial | `recommend_loaded_data_limit` + `loadedDataSizeRecommendation` JSON field are the hint source; codegen embedding is the remaining step |
| 2 | Externally-implementable account-wrapper trait that still emits `LayoutDescriptor` | Done | `LayoutDescriptor` is a plain trait with provided methods (`registry_entry`/`validate_hot`/`fingerprint`/`idl_node`/`cost_profile`); any type can implement it |
| 3 | `Nested<T>`-style shared validation as a composite descriptor with a composite fingerprint | **Done (this commit)** | `DescriptorGroup` — ordered members, `fingerprint()` folds each member in order with a length prefix, `validate_all` positional check, `min_loaded_data_size`/`recommend_loaded_data_limit`; JSON via `codama::DescriptorGroupJson` |
| 4 | Anchor v2 → Hopper migration doc + v2-compat feature | Partial | `MIGRATION_FROM_ANCHOR.md` exists; a dedicated v2-compat feature flag is not added (no consumer yet) |

### P1

| # | Recommendation | Status | Where / rationale |
|---|----------------|--------|-------------------|
| 5 | CU levers (literal-bump precompute, const-rent, compile-away guardrails) | Out of scope here | Macro/runtime CU work is orthogonal to the descriptor model targeted this pass |
| 6 | Keep `CuBudget` + SIMD-0339/0268 constants live | Done (pre-existing) | Retained; `SizeClass` bounds track the growth cap direction |
| 7 | Formalize the layout-fingerprint ABI contract end-to-end | Done | Fingerprint flows descriptor → registry row (`layout_id`) → `DescriptorIdlNode` → `DescriptorMetadata` JSON → `decode_allowed` guard; CLI now enforces the `layout_id` half against a live account |
| 8 | Add a layout-fingerprint node to the Codama projection | Done | `DescriptorIdlNode.fingerprint` + `fingerprint` field in `DescriptorMetadataJson` |
| — | Typed address/account validation derived from a descriptor | **Done (this commit)** | `AccountDescriptor::expect_owned_by` → `AccountExpectation`; `check` (owner+size+disc) and `check_decodable` (adds fingerprint guard) return an `AccountCheck` verdict for off-hot-path / manager / client use |

### P2

| # | Recommendation | Status | Where / rationale |
|---|----------------|--------|-------------------|
| 9 | Miri harness over the cast boundary | Out of scope here | Verification-infra task, not a descriptor gap |
| 10 | Kani proofs | Out of scope here | As above |
| 11 | TS client test path over `hopper-svm` | Partial | Blocked on the per-language generator (P0-1) |
| 12 | Extend `hopper explain` to an SBF/CU step view | Out of scope here | CLI/tooling feature outside the descriptor model |

## Not copied from Anchor (deliberate)

- **8-byte SHA-256 discriminator** — Hopper keeps the 1-byte compact disc; the
  16-byte `LayoutFingerprint` provides collision-resistant identity off the hot
  path without paying 8 bytes per account.
- **Pinocchio as a hard dependency** — Hopper's runtime stays independent.
- **Borsh-renamed account types** — Hopper's wire types are alignment-1 Pod
  overlays, not serde/Borsh.

## Remaining roadmap

1. Per-language client generators (`rust_client`, `python_client`, …) consume
   the `DescriptorMetadata{,Set}Json` / `DescriptorGroupJson` output to embed
   `fingerprint` constants and call `recommend_loaded_data_limit` in their
   transaction builders (closes P0-1 / P2-11).
2. Emit `DescriptorGroup`s from the macro for multi-account instructions so
   composite fingerprints are generated, not hand-built.
3. Optional Anchor v2-compat feature flag once a concrete consumer exists (P0-4).
4. Extend `cost_lint` with per-field hot/cold classification once field-level
   role metadata is threaded through the descriptor.
