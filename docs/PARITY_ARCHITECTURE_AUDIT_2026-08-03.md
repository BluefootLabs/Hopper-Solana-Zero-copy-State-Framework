# Hopper parity and architecture audit — 2026-08-03

This is a source-level snapshot, not a permanent ranking. It compares the
unreleased Hopper 0.3.0 workspace at `c539bb6` plus the follow-up corrections
in the working tree with the upstream default branches and official docs
available on 2026-08-03.

> **2026-08-15 update:** Anchor v2 is active on `anchor-next` but remains an
> unpublished, unaudited alpha, and
> Quasar's active `0.1.0-release` branch is substantially ahead of its default
> `0.0.0` branch. See the pinned
> [zero-copy framework audit](ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md). The
> original dated findings below are retained where they describe the 08-03
> snapshot.

## Executive verdict

Hopper has a real technical differentiator: its mutation declaration is shared
by generated clients, runtime enforcement, manifests, touch maps, and the
Grillo verifier. C4 proved the value of that architecture by exposing the
`epoch_migrate` writable-demotion bug. No reviewed peer currently offers the
same enforced field/byte write contract.

The adoption bottleneck is credibility and surface-area control, not another
feature. The 0.3 workspace is substantially broader than Pinocchio, Quasar, or
Star Frame, owns a custom unsafe substrate, and has not had an independent
security audit. The raw-surface and `epoch_migrate` bugs both crossed macro,
schema, client, and runtime boundaries. That makes an audit-readiness dossier,
invariant automation, and a public 0.3 release more valuable than a byte-level
"parallelization" feature the current protocol cannot use.

Recommended order:

1. Land this audit follow-up and keep the release gate green.
2. Build the audit-readiness dossier and commission an external review.
3. Keep C3's 698-case no-skip Cicada semantic adapter green in CI and retain
   the compiled-SBF lifecycle suite as the separate transaction evidence lane.
4. Fold C6 into C4 as account-set topology/contention analysis.
5. Keep C1 as a research/API draft; do not promise scheduler parallelism.

## Local commit verification

The repository was clean before this audit and `main` was exactly four commits
ahead of `origin/main`:

| Commit | Verified scope | Assessment |
| --- | --- | --- |
| `a0408a5` | rustfmt-only normalization of ten files | Sound, but the pinned Rust 1.96 Clippy gate had additional diagnostics after the later commits. The working-tree follow-up fixes them. |
| `51411e6` | guarded raw surfaces, lifecycle gaps, compile-fail and integration coverage | Direction is correct. One duplicated `#[test]` left `bare_strict_writes_installs_a_data_only_ambient_gate` unregistered; fixed in the working tree. |
| `f8560d9` | public-claim corrections | Improved the record, but missed several 0.3 publication and competitor-version claims. Corrected in the working tree. |
| `c539bb6` | contention profile and `epoch_migrate` declaration fix | The migration correctness fix and its regression test are valid. C4's network constants and "exact CU" semantics needed correction; see below. |

`cargo test --workspace --locked --no-fail-fast` passed with no failures. The
reported "190 suites" is not a reproducible Cargo metric: Cargo reports 130
distinct test executables under `--no-run`, while the Hopper CLI test binary
alone contains 192 tests. The defensible statement is "the locked workspace
test command passes," not "190 suites," unless the counting method is defined.

## C4 correctness audit

### What C4 got right

- `epoch_migrate` performs bind-time writes and therefore must contribute a
  whole-account declared range. Without it, mutation-complete demotion made a
  required writable role read-only in generated clients.
- The effective-writable rule is useful because it turns an enforced,
  mutation-complete declaration into client metadata and a static contention
  signal.
- The tool correctly keeps remaining accounts separate and warns that requested
  execution CU, loaded-data CU, instruction bytes, and the fee payer are outside
  an instruction declaration.

### Corrections required

1. `MAX_BLOCK_UNITS` was 60M, but SIMD-0286 activated on Mainnet on
   2026-07-29 and raised it to 100M. `MAX_WRITABLE_ACCOUNT_UNITS` was recorded
   as 24M; the live limit is 12M and did not change. The constants, docs, CLI
   reference line, and a regression test now use 100M/12M. See the official
   [100M CU Blocks upgrade](https://solana.com/upgrades/100m-cu-blocks).
2. Agave bills unique present Pubkeys. Hopper's profile counts manifest role
   slots. Optional roles can be absent and `dup` roles can alias one key, but
   the contention account surface does not preserve enough runtime identity to
   deduplicate them. Role counts are exact; their CU product is a deterministic
   fixed-role upper bound. The CLI now labels it `Fixed max`.
3. `--max-block-cost` is retained for compatibility, but it gates only that
   fixed-role upper bound, not a transaction's block cost. A future breaking
   CLI revision should rename it to `--max-fixed-cost`.

## Framework architecture snapshot

Primary sources:

- [Hopper workspace](../Cargo.toml)
- [Quasar source](https://github.com/blueshift-gg/quasar) and
  [official docs](https://quasar-lang.com/docs)
- [Pinocchio source](https://github.com/anza-xyz/pinocchio)
- [Anchor source](https://github.com/otter-sec/anchor)
- [Star Frame source](https://github.com/staratlasmeta/star_frame)
- [Steel source](https://github.com/regolith-labs/steel)

Versions are source snapshots, not claims about crates.io publication:

| Framework | Snapshot | Architectural center |
| --- | --- | --- |
| Hopper | workspace 0.3.0; public release 0.2.1 | Owned zero-dependency substrate plus macro/schema/client/verifier contract |
| Quasar | default/crates 0.0.0; active `0.1.0-release` branch is beta/unaudited | Direct account views, capability wrappers, resize migrations, ABI/client/test/profiler tooling |
| Pinocchio | source 0.11.x | Minimal `no_std` Solana SDK replacement and zero-copy entrypoint primitives |
| Anchor | stable 1.1.2; unpublished `anchor-next` v2 alpha | Full-stack ecosystem; v2 adds default zero-copy state, Slab/PodVec, typed CPI borrows, and formal/adversarial lanes |
| Star Frame | source 0.30.0 | Pinocchio-backed modular traits, zero-copy unsized types, IDL/Codama tooling |
| Steel | source 4.0.9 | Lightweight macros/helpers and CLI over a smaller, explicitly unaudited surface |

### Feature parity matrix

`Strong` means a first-class, source-visible workflow; `Partial` means a narrower
or opt-in equivalent. It does not mean audited or production-proven.

| Capability | Hopper | Quasar | Pinocchio | Anchor 1.1/v2 work | Star Frame |
| --- | --- | --- | --- | --- | --- |
| Zero-copy/no-allocation program path | Strong | Strong | Strongest/minimal | Partial/stronger in v2 work | Strong |
| Declarative account validation | Strong | Strong | Helpers only | Strong | Strong |
| Enforced field/byte write policy | **Unique strong** | No | No | No | No |
| Runtime touch/effect map | Strong, opt-in | Byte diffs in test SVM | No | Coverage/fuzz tooling, not an effect contract | No comparable contract found |
| Schema fingerprint/evolution graph | **Strongest** | Typed migration, narrower schema identity | No | IDL/account discriminators; migration is app-level | IDL/Codama, no comparable graph found |
| IDL and generated clients | 8 outputs total: TypeScript, Kotlin, Python, Go, C, off-chain Rust, Codama JSON, Anchor IDL JSON | Stable Rust/Kit/Web3 plus preview Python/Go/C and wire IDL/CPI generation | No | **Strongest ecosystem** | IDL/Codama |
| SVM/test harness | `hopper-svm`, Mollusk/devnet lanes | QuasarSVM Rust/Node/Python | Bring your own | Surfpool/test validator plus tooling | Examples/tooling |
| Fuzz/formal workflow | Kani/Miri scripts, static targets | Kani/Miri/fuzz integration on 0.1 release line | Bring your own | v2 Kani/Miri/fuzz and runtime lanes | Miri CI; no equivalent formal suite found |
| Profiling/debugging | Hopper profile/bench, tx explain | Static CU profiler/flamegraph | Bring your own | CLI profile/debugger/coverage | Basic tooling |
| Verifiable build/deploy/registry maturity | Partial | Partial | Substrate only | **Strongest** | Partial |
| Independent framework audit | No | Docs say unaudited beta | Production/audit lineage, but scope-specific | Mature ecosystem; audit scope varies | No broad audit claim found |

### Architecture assessment

Hopper's architecture is coherent but tightly coupled:

```text
context/account macros
        ↓
schema + mutation manifest
   ↙       ↓        ↘
runtime   clients   Grillo verifier
   ↓        ↓            ↓
write gate writable metas effect verdict
```

That shared declaration is the moat and the largest correlated-failure domain.
The release criterion should therefore be a generated invariant, for every
lifecycle attribute and mutation surface:

```text
declared writable
== published writable
== client writable
== runtime-required writable
and every bind/handler mutation ⊆ enforced declared ranges
```

The `epoch_migrate` bug violated that equality. R2/R3 showed that raw and
unchecked paths can violate the subset relation. C3 now generates deterministic
positive and adversarial cases and can run them through a strict adapter
protocol. Application adapters still supply valid program-specific fixtures
and business invariants; a plan that was only generated has not tested these
equalities.

## Competitive findings

### Where Hopper leads

- Enforced byte-range mutation declarations and proof-backed writable demotion.
- A single effect surface spanning runtime, generated clients, manifest, touch
  evidence, and an independent verifier.
- Layout fingerprints, schema epochs, migration graphs, and bind-time healing.
- Broad client generation without requiring the Anchor TypeScript stack.

### Where peers lead

- Anchor leads on adoption, documentation volume, integrations, verifiable
  builds/deploys, debugger/coverage workflows, and formal/adversarial breadth;
  v2 alpha also supplies zero-copy Slab/PodVec and typed CPI-borrow ergonomics.
- Quasar has a smaller conceptual surface, polished direct-state ergonomics,
  grow/shrink typed migrations, ABI hashing, Kani/Miri/fuzz work, a static CU
  profiler, and multi-language QuasarSVM/client work on its 0.1 release line.
- Pinocchio is the clearer choice when a team wants the smallest raw substrate
  and will own validation, IDL, clients, and state evolution itself.
- Star Frame is a credible high-performance peer with modular trait composition
  and Codama-oriented IDL tooling; it should be kept in every future audit.

### Material Hopper risks

- No independent security audit despite a large unsafe/custom-runtime surface.
- Public release lag: the repository advertises and documents 0.3 features while
  crates.io remains 0.2.1.
- Surface area: many crates, compatibility layers, generators, examples, and
  lifecycle modes multiply combinations that need generative testing.
- Some public competitive claims were snapshots presented in the present tense.
  Benchmarks against Anchor 0.31.1 are not evidence about Anchor 1.1.2 or v2.
- `hopper contention` is useful for account-set analysis, but byte disjointness
  has no scheduler payoff on today's protocol because locks remain per Pubkey.

## Critical network-upgrade verification

The transaction-size upgrade is **not Mainnet-live as of 2026-08-03**.

- Today's legacy/v0 limit remains 1,232 bytes. Solana's current core docs still
  list that limit: [Core Concepts](https://solana.com/docs/core).
- The planned increase is 1,232 to 4,096 bytes and only applies to the new v1
  transaction format. The official upgrade page targets Mainnet in Q3 2026:
  [Larger Transaction Sizes](https://solana.com/upgrades/larger-transaction-sizes).
- V1 uses leading version byte 129, does not support address lookup tables, and
  moves compute-budget configuration into a header mask. Raw transaction
  decoders/indexers must change.
- The governing documents remain in `Review` with no feature key recorded:
  [SIMD-0296](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0296-larger-transactions.md)
  and [SIMD-0385](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0385-transaction-v1.md).

The newly live Mainnet upgrade is different: SIMD-0286 raised the **block CU
limit** from 60M to 100M on 2026-07-29. It did not change transaction bytes,
the 12M writable-account CU cap, or the 100MB block account-data delta. Any
statement that "Mainnet raised transaction size" currently conflates these two
upgrades.

## Decision on the backlog

- **Audit dossier first.** Include unsafe inventory, trust boundaries, generated
  invariant map, threat model, dependency/SBOM, reproducible SBF builds, fuzz and
  formal coverage, devnet evidence, known limitations, and explicit excluded
  scope. Then seek an independent auditor.
- **C3 execution infrastructure and Cicada semantic rollout completed
  locally.** `hopper fuzz generate/check` covers every declared role,
  duplicate alias pair, remaining-account ceiling, migration edge, lamport
  effect, argument boundary, and static/parametric write range. `hopper fuzz
  run` fails closed on missing cases and invariant hooks. Cicada's no-skip
  adapter recomputes all 698 cases against the live manifest and exercises
  actual private business guards under seeded host states. It does not label
  structural host probes as SBF transaction executions.
- **C6 reframe.** Analyze conflicts between instruction account sets, hot Pubkeys,
  needless writable flags, and transaction-packing opportunities. Byte ranges can
  explain *why application-level sharding is safe*, but must not be scored as
  protocol parallelism.
- **C1 research only.** A field-scoped declaration could inform a future scheduler
  SIMD or off-chain executor, but current Sealevel privileges and locks are
  account-scoped. Keep protocol claims out of the product copy.
