# Solana zero-copy framework audit: 2026-08-15

This is a dated source audit of Anchor 1.x and the Anchor v2 alpha branch,
Blueshift Quasar, Anza Pinocchio, Star Frame, Steel, and Hopper's Cicada and
Sentinel flagship programs. It is not an external security audit and does not
turn an upstream benchmark into a Hopper measurement.

> **Reverified 2026-09-06:** Anchor published `anchor-lang` 2.0.0-rc.1 and
> tag `v2.0.0-rc.1` on 2026-08-12. The pinned source findings remain useful,
> but the original “not published” packaging claim was already stale; stable
> Anchor v1.2.0 shipped 2026-09-04. QEDGen/qedsvm also invalidates broad
> “nobody proves/diffs” claims. Current competitor, network, rent, and
> deployment-cost findings are in
> [COMPETITIVE_REFRESH_2026-09-02.md](COMPETITIVE_REFRESH_2026-09-02.md).

## Executive result

- **Anchor v2 is published as a release candidate.** `anchor-lang`
  2.0.0-rc.1 and tag `v2.0.0-rc.1` exist; its official README still labels
  the line Alpha/unaudited and presents Anchor 1.1.2 as stable.
- **Quasar's meaningful target is its `0.1.0-release` branch.** The default
  branch and crates.io package remain `0.0.0`, but evaluating only that branch
  misses its current CLI, client, migration, profiler, ABI, testing, and formal
  verification work. Quasar still labels 0.1 beta and unaudited.
- **Hopper is not justified in claiming universal superiority yet.** It has a
  differentiated, machine-enforced write/effect contract and broader state
  evolution model. Anchor leads in ecosystem maturity; Anchor v2 and Quasar
  have important zero-copy/DX ideas Hopper must measure and, where useful,
  match. A 2026-08-16 clean five-way archive now supplies the current
  same-fixture benchmark for the pinned vault contract. Hopper still needs an
  independent audit, and the benchmark does not establish universal
  superiority.
- **The internal Cicada review found and fixed concrete correctness defects.**
  The working tree now closes custody-adoption, Token-2022 restore, token-shape,
  mint-mutation, loader-authority, finalized-vault dusting, duplicate-meta, and
  writable-mint-alias paths with host and compiled-SBF regressions. This is
  strong local evidence, not an independent audit or public-cluster proof.

## Pinned upstream snapshots

| Project | Snapshot inspected | Release signal |
|---|---|---|
| Anchor stable | [`06d776a`](https://github.com/otter-sec/anchor/commit/06d776a2380885259558e3d2ff95f69842705d6a) | GitHub release `v1.1.2` |
| Anchor v2 | [`e8d0e47`](https://github.com/otter-sec/anchor/commit/e8d0e47d3ad3825982a399ef9d2769bc1f124259) on `anchor-next` | Pinned source snapshot; `anchor-lang` 2.0.0-rc.1 and tag `v2.0.0-rc.1` were published 2026-08-12; still Alpha/unaudited |
| Quasar default | [`b0de7db`](https://github.com/blueshift-gg/quasar/commit/b0de7db4cd271654a2dcf78807dd865e98e0b339) | `0.0.0`, beta/unaudited |
| Quasar release line | [`0361701`](https://github.com/blueshift-gg/quasar/commit/03617018c2665340abc63dc9f7becda55a25ce48) on [`0.1.0-release`](https://github.com/blueshift-gg/quasar/tree/0.1.0-release) | Source version `0.1.0`, beta/unaudited; no public tag/release at inspection time |
| Pinocchio | [`adbd48d`](https://github.com/anza-xyz/pinocchio/commit/adbd48d12229ffa30d6fb3d3a8ff777fdb053b80) | Current crates.io line `0.11.2` |
| Star Frame | [`6936d58`](https://github.com/staratlasmeta/star_frame/commit/6936d582a942760b67728059be651473645ee099) | Current source/published line `0.30.0` |
| Steel | [`59f8e9a`](https://github.com/regolith-labs/steel/commit/59f8e9a5633dc6a3b0f5acfb44693e935e257024) | Source line newer than its latest GitHub release; README says unaudited |
| LiteSVM | [`8559c7e`](https://github.com/LiteSVM/litesvm/commit/8559c7e5b8822818894bfeb43bbdd6911df5c872) | Reviewed source snapshot; latest tagged release `0.15.1`; testing infrastructure, not an on-chain framework |
| Blueshift Parallax | [`f8ffdca`](https://github.com/blueshift-gg/parallax/commit/f8ffdcac66ba512893de211fc10fb1d8ac120033) | Source `0.1.0`; no tag, release, or CI at inspection time |
| Light Protocol | [`ad5964f`](https://github.com/Lightprotocol/light-protocol/commit/ad5964f175d0b1c9fc6c61c6f82fe0831842941d) | Compression protocol/tooling; not the project named LiteSVM |

No official stable Anchor v2 release date was found. A tag, branch, README, or
target quarter is not evidence of a future stable release date.

## Anchor zero-copy: shipped 1.x behavior

Anchor 1.x zero-copy is an opt-in account representation built around
`#[account(zero_copy)]` and `AccountLoader<T>`. The generated type is
`repr(C)` plus `bytemuck::Pod`/`Zeroable`; `load`, `load_mut`, and `load_init`
borrow the account data through a `RefCell` and validate ownership and the
account discriminator as appropriate. The current official guide is
[Anchor zero-copy](https://www.anchor-lang.com/docs/features/zero-copy).

Practical constraints:

- the mapped body must have a fixed, Pod-compatible layout;
- variable-length fields such as `Vec` and `String` are not part of the direct
  mapped body;
- callers must release loader borrows before CPI;
- `zero_copy(unsafe)` preserves a deliberately unsafe compatibility path; and
- current stable source includes a size check protecting loader exit from a
  truncated backing account.

This is genuine zero-copy state access, but it does not provide Hopper's
declared byte-range mutation policy, schema-epoch graph, or cross-checked
client/runtime write declaration.

## Anchor v2: what changed

The v2 alpha makes zero-copy the default account model rather than a special
mode. Its Pinocchio-based, `no_std` program layer maps
`[8-byte discriminator][repr(C) T]` as `Account<T>` and does not serialize the
account back on exit. It also adds:

- `BorshAccount<T>` for dynamic or enum-heavy state;
- `Slab<H, Item>` for a fixed header plus dynamic zero-copy tail;
- `PodVec<T, MAX>` and alignment-one Pod wire wrappers;
- typed CPI handles and borrow tracking around Pinocchio's unchecked CPI path;
- literal-seed PDA validation and a program-owned PDA optimization;
- Wincode events by default, with a bytemuck option;
- optional constant-rent calculation, with an explicit formula-drift warning;
- required Miri Tree Borrows and fuzz/runtime-test lanes, plus Kani harnesses
  whose CI execution is currently disabled pending kani-verifier 0.68+; and
- default `guardrails` that may compile away checks to reduce CU.

The design is materially closer to Hopper than Anchor 1.x, especially around
mapped dynamic tails and verification. Hopper remains different where its
manifest connects layout, migrations, client metas, declared mutation ranges,
runtime gates, touch evidence, contention analysis, and Grillo verification.

### Anchor v2 security-readiness signal

The alpha branch has substantial verification work, including merged
[Kani/Miri coverage](https://github.com/otter-sec/anchor/pull/4424), external
Crucible fuzz integration, and required Miri CI. Kani harness source exists,
but the pinned workflow disables its Kani lane because 0.67 panics and says to
restore it at 0.68+. Active audit remediation also means Hopper must not
describe the alpha as finished or audited.

Stable Anchor 1.1.2 also has the low-severity
[`GHSA-6px8-6mw3-4hx4`](https://github.com/otter-sec/anchor/security/advisories/GHSA-6px8-6mw3-4hx4)
advisory. It affects `anchor-lang >=0.31.0` with no patched release listed at
this review: `LazyAccount::unload()` clears its cache after CPI but does not
revalidate the current owner or discriminator before later typed loads. Hopper
uses this as a cross-framework threat-model case rather than as a marketing
claim; its own safe typed handles must prove their post-CPI validation behavior
with mutation regressions.

Hopper's follow-up review now makes the boundary concrete. Safe
`ExternalAccount` and `InterfaceAccount` typed access repeats live owner and
layout validation, including before a checked proof is issued; regressions
mutate owner, discriminator, and concrete interface layout after binding.
Foreign lenses retain a borrow guard, so safe writable CPI fails until the lens
is dropped. `ExternalChecked` business proofs and the raw views exposed by
`Account`/`InitAccount`/`SystemAccount` remain explicitly point-in-time: a
caller must re-check them after a mutating CPI or deliberate account lifecycle.
Solana prevents a foreign callee from reassigning caller-owned program state,
but Hopper does not turn that protocol rule into a broader claim that every raw
method reachable through `Deref` is role-revalidated.

Open work at inspection time included:

- ordering account updates after access control
  ([#4863](https://github.com/otter-sec/anchor/pull/4863));
- rejecting floating-point/NaN account fields
  ([#4865](https://github.com/otter-sec/anchor/pull/4865));
- gating Borsh and Slab writes by program ownership
  ([#4866](https://github.com/otter-sec/anchor/pull/4866),
  [#4867](https://github.com/otter-sec/anchor/pull/4867));
- preserving instruction argument names
  ([#4868](https://github.com/otter-sec/anchor/pull/4868));
- retaining remaining accounts in generated CPI and validating PDA signers
  ([#4885](https://github.com/otter-sec/anchor/pull/4885),
  [#4925](https://github.com/otter-sec/anchor/pull/4925));
- close-destination writability and Slab/PodVec boundary corrections
  ([#4886](https://github.com/otter-sec/anchor/pull/4886),
  [#4888](https://github.com/otter-sec/anchor/pull/4888),
  [#4906](https://github.com/otter-sec/anchor/pull/4906),
  [#4907](https://github.com/otter-sec/anchor/pull/4907)); and
- flattened-header, import, optional-PDA, and zero-byte CPI return handling
  ([#4909](https://github.com/otter-sec/anchor/pull/4909),
  [#4924](https://github.com/otter-sec/anchor/pull/4924),
  [#4893](https://github.com/otter-sec/anchor/pull/4893),
  [#4895](https://github.com/otter-sec/anchor/pull/4895)).

These links are a dated triage list, not a claim that every PR is exploitable
or will merge unchanged.

### Anchor's own performance fixture

Anchor v2's committed vault fixture reports the following. These are upstream
measurements, not independently reproduced Hopper results:

| Framework | Binary bytes | Deposit CU | Withdraw CU |
|---|---:|---:|---:|
| Pinocchio | 5,072 | 1,229 | 57 |
| Quasar | 6,024 | 1,889 | 396 |
| Anchor v2 alpha snapshot | 6,000 | 1,910 | 403 |
| Steel | 65,160 | 2,452 | 530 |
| Anchor v1 | 107,368 | 5,707 | 2,478 |

Source: [`bench/results.json`](https://github.com/otter-sec/anchor/blob/anchor-next/bench/results.json).
On this one fixture Quasar is slightly cheaper in CU than Anchor v2, while
their binaries are nearly tied. It does not establish a universal ranking.

## Quasar 0.1 release-line audit

Quasar's `0.1.0-release` branch is a substantially different comparison target
from its `0.0.0` default branch. It contains:

- direct SVM account views, `no_std`, and zero-allocation account access;
- account behavior/capability macros and bounded remaining-account handling;
- typed migrations that cover same-size, grow, and shrink cases and test
  cleared padding;
- wire IDL plus an ABI hash and `declare_program` support;
- stable Rust, Kit 7, and Web3.js 3 clients, with Python, Go, and C marked
  preview;
- a CLI covering init/build/test/deploy/verify/lint/profile/IDL/client work;
- QuasarSVM test packages and Kani, Miri, and fuzz workflows; and
- a profiler that counts decoded SBF instructions statically, with budget/diff
  work in progress. It is not runtime CU and does not model loops or syscall
  cost.

Quasar fixed accounts use direct typed views. Its dynamic mutation path is
different: it snapshots dynamic fields into owned `PodString`/`PodVec` values
and repacks the compact tail on save/drop. Its single-edge migration helper is
useful, but shrink realloc transfers all excess above the new rent floor to the
payer. Hopper's fit migration deliberately refunds only freed rent so unrelated
deposited lamports are preserved.

The release line still says **beta and unaudited**. Its benchmark work is an
open draft ([#497](https://github.com/blueshift-gg/quasar/pull/497)), and active
fix branches/PRs include CPI return-data handling and optional mutable-account
alias coverage. Treat the functionality as serious engineering, but do not
convert an unreleased branch or draft benchmark into a production claim.

## Other relevant frameworks

| Framework | What it establishes | What it does not provide by itself |
|---|---|---|
| Pinocchio | The minimal `no_std`, zero-copy/zero-allocation efficiency floor; lazy/custom entrypoints, explicit CPI, allocator-free modes, account resizing | A full declarative framework, IDL/client ecosystem, layout evolution graph, or enforced write manifest |
| Star Frame | Pinocchio-backed trait composition, fixed and unsized zero-copy types, Borsh alternatives, CLI and Codama-oriented IDL verification | Hopper's byte-write policy/effect verifier or migration graph; rent helpers may refund all excess above the current minimum, unlike Hopper's deposit-preserving fit policy |
| Steel | Small Pod account helpers, assertions, instruction/account macros, and CLI ergonomics | IDL generation or an audited production claim; its README explicitly warns it is unaudited |

Pinocchio is a substrate, not a like-for-like full framework. A fair performance
comparison must charge Hopper, Anchor, Quasar, or Star Frame for equivalent
validation and behavior rather than compare checked framework code with an
unchecked cast.

## Testing infrastructure and the Light/Lite naming distinction

LiteSVM's latest tagged release is 0.15.1; the reviewed `8559c7e` source
snapshot is newer than that tag. LiteSVM is in-process Solana execution
infrastructure. It adds
snapshots, time travel, CU/heap controls, CPI trees, register traces, custom
syscalls, debugger support, and Node bindings. It still uses Agave 4.1.1
crates, so it complements rather than replaces Hopper's Agave 4.2 compiled-SBF
lane.

Blueshift Parallax is a framework-neutral fixture layer over LiteSVM with a
shared Rust core and Kit/Web3 bindings. Its fixture worlds, invariants,
commit/simulate split, dump/load artifacts, and outcome assertions are valuable
DX targets. Its default backend disables signature and blockhash checks and
zeros fees, so it is intentionally not Mainnet transaction-authentication or
economic fidelity. Its published microsecond figures are host wall time, not
on-chain CU.

Light Protocol is a separate compression project. Its `light-zero-copy`
borrowed parsing and program-test utilities are relevant design references,
but `light-program-test` is compression-specific and pins an older LiteSVM.
Light Protocol audit claims must not be transferred to its active-development
SDKs, macros, or to LiteSVM.

## Hopper parity-plus assessment

| Area | Current result | Required next gate |
|---|---|---|
| Zero-copy fixed state | Parity-plus: checked mapped state, owned substrate, compact/wire layouts | Keep malformed-size, owner, discriminator, alias, and raw-surface tests release-blocking |
| Dynamic zero-copy data | Strong: bounded fields, growable `Seq`, and bitmap-backed `Slab`/`TailSlab` with collection-style capacity helpers | Keep same-fixture ergonomics and CU/binary comparisons current; avoid “only framework” claims |
| Mutation/effect declarations | Differentiated: manifest, runtime gate, generated metas, touch evidence, Grillo, and generated fuzz plans/corpora | C3 generation and strict execution protocol are implemented; Cicada has a no-skip host semantic adapter, and other programs should add adapters where valid fixtures or business invariants cannot be derived from the manifest |
| Migration | Strong graph/epoch/fingerprint model plus payer-funded `resize = grow|fit`; shrink refunds only freed rent | Keep zero-fill, live-rent, hostile payer, rollback, and client-writability tests release-blocking |
| CPI safety | Strong declared policies plus canonical SPL Token and Token-2022 Cicada lifecycle/hostile-route checks | Keep the canonical rollback matrix release-blocking; add validator/RPC attribution for dynamic downstream effects |
| IDL/clients | Broad target count and parity checks | Compile generated outputs continuously and add ABI-hash compatibility fixtures |
| Testing/formal | Broad tests plus Kani/Miri/fuzz lanes and manifest-derived seeded plans | Cicada executes its corrected 698-case plan through a no-skip host semantic adapter; preserve the separate compiled-SBF transaction lane and expand the same truthful adapter/provenance model to other programs |
| Profiling | Bench/profile/contention tools plus stable baseline budgets/diffs and a behavior-gated five-way fixture | The clean Hopper `8696640` / benchmark `af5bc95` archive is retained and content-addressed in [`audit/framework-matrix-2026-08-16.json`](../audit/framework-matrix-2026-08-16.json); rerun when a pin changes and keep byte ranges out of protocol-parallelism claims |
| Release/security | Internal unsafe inventory and executable dossier | Independent audit, public remediation record, SBOM/advisory closure, reproducible 0.3 release |

## Cicada and Sentinel validation

### Static Cicada result

The review found and then hardened the important boundaries represented in
code:

- creation atomically moves source authority from the signing owner to an
  owner/source-bound vault PDA, re-reads the post-CPI authority, and uses a
  source lease to prevent concurrent custody of the same source;
- the typed shard borrow is dropped before an arbitrary route CPI;
- route programs must be executable and committed program/account metadata are
  checked;
- legacy SPL account/mint shapes are exact, while Token-2022 TLV parsing is
  fail-closed with a narrow role-specific extension allowlist;
- source delegate, delegated amount, close authority, unrestorable
  `ImmutableOwner`, and enabled CPI Guard states are rejected at custody entry;
- full mint bytes, including supply, are stable across route CPI;
- signer/writable privilege escalation is rejected, with only the vault PDA
  eligible as an added signer, conflicting duplicate metas and every repeated
  writable route address rejected, and both committed mints protected from
  writable remaining-account aliases;
- source and destination lamports obey native-aware floors across route CPI,
  preventing a signer-capable route from restoring token bytes after diverting
  rent or excess SOL through close-and-recreate;
- settlement uses observed balance deltas and Solana rollback protects failed
  routes;
- live loader-v3 upgrade authority, rather than the first arbitrary caller,
  controls singleton initialization. The source's loader-v4-format branch is
  compatibility/test modeling only; loader v4 was abandoned and is not a live
  deployment path; and
- reclaim binds the lease slot/sequence, tolerates post-final vault dust,
  restores and re-reads source authority, clears cells, and closes the lease.

### Commands and results

| Validation | Result |
|---|---|
| `HOPPER_REQUIRE_CICADA_SBF=1 cargo test -p hopper-cicada` | 21 host tests and 22 compiled-SBF tests passed against freshly built Cicada, hostile-route, and canonical-route ELFs; the matrix includes real SPL Token and Token-2022 execute/refund/reclaim, both processors' canonical native-mint lamport coupling, writable-mint refusal, and rollback of a modeled under-backed native account |
| Canonical Cicada route fixture | 3 direct Mollusk tests passed; its supply-neutral `MintToChecked` plus `BurnChecked` attack restores final mint bytes, while compiled Cicada rejects the writable mint alias before invoking the route |
| Grillo Cicada parametric suite | 7 passed |
| Grillo real-manifest parser suite | 7 passed |
| Runtime competitor bug classes | 11 passed |
| Systems competitor bug classes | 7 passed |
| Native loader-input conformance | 8 passed |
| `cargo clippy -p hopper-cicada --all-targets --locked -- -D warnings` | passed |
| Cicada lint with `--deny-escapes` | exited successfully; emitted receipt-chain attribution warnings for writable roles lacking a named validation/context invariant |
| `cargo test -p hopper-sentinel` | 8 host flagship tests and 2 compiled-SBF refusal tests passed |
| Grillo Sentinel suite | 4 passed |
| `cargo test --workspace --locked --no-fail-fast` | passed in the encompassing 2026-08-15 audit run; this exercises workspace host tests but is not evidence that every example has a compiled-SBF adversarial suite |
| Historical binary-backed Cicada publish check | passed all 3 legacy layout-anchor scans and every program-shape, documentation, feature, token, client, fuzz, artifact, Solana-shape, 160 systems, and trybuild gate against the 167,680-byte `cargo-build-sbf 4.1.0` ELF (`sha256:ac8ec1d76b4f85a5515dc446536bafccabe1c62b8ff46a13a662971b785da0e9`); the artifact predates the versioned ELF interface binding and cannot satisfy the current release gate |

The repository contains many demonstration programs beyond Cicada and
Sentinel. Their workspace tests are green, but only the named flagship paths
received this review's deeper static and compiled-SBF adversarial pass. That
scope distinction prevents a workspace-green result from becoming a blanket
security claim.

The receipt-chain warnings are an auditability/DX gap, not evidence that the
runtime checks failed. They should be resolved or explicitly justified before
external review.

### Residual flagship work

Cicada now exercises real route CPIs into Mollusk's canonical SPL Token and
Token-2022 programs, while a separate hostile fixture injects mutations that
canonical token programs cannot perform and proves rollback. Remaining limits
are evidence and deployment-policy boundaries: no independent audit report, no
archived public-cluster/validator-replay run, no clean committed CI attestation
for the final ELF, and no complete attribution of dynamic downstream account
deltas. Retained mint/freeze authority can still change supply or freeze assets
between lifecycle instructions, and program-trust mode necessarily delegates
broad behavior to an upgradeable route program. Token-2022 support is a narrow
fail-closed subset, not universal extension support. Passing local fixtures do
not prove arbitrary downstream programs or issuers safe.

## Recommended execution order

1. Finish the audit-readiness package and commission the independent review.
2. Commit and gate C3's corrected 698-case Cicada plan, strict runner, and
   no-skip semantic adapter. Preserve the evidence boundary: live-manifest and
   actual host business-guard probes are not compiled-SBF transactions.
3. Keep the closed Cicada canonical SPL/Token-2022 and hostile rollback matrix
   green, then archive public-cluster and validator-replay evidence.
4. Keep the completed clean Hopper, Anchor v2 alpha snapshot, Quasar 0.1,
   Star Frame, and Pinocchio matrix current. The 2026-08-16 archive closes the
   evidence gap for its exact pins; rerun after source, dependency, toolchain,
   fixture, or runner changes.
5. Keep safe grow/fit migration and stable CU baseline/diff gates green; both
   parity items are implemented in this working tree.
6. Keep C6 account-set based. Solana schedules by Pubkey today; byte-disjoint
   writes to one account do not create protocol parallelism.

The defensible positioning is: **Hopper is designed to make zero-copy state
intent independently checkable.** “Best,” “fastest,” and “safest” remain
fixture- or audit-qualified claims until the corresponding evidence exists.
