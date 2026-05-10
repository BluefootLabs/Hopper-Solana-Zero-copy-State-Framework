# Solana Zero-Copy Parity and Differentiation

This is the launch-facing comparison for Hopper against the zero-copy paths a
Solana team is likely to evaluate:

- **Native Solana / raw zero-copy**: direct account-data borrows and explicit
  byte casts or `bytemuck`-style overlays in hand-written programs.
- **Anchor zero-copy**: `#[account(zero_copy)]` plus
  `AccountLoader<'info, T>`.
- **Quasar zero-copy**: Pinocchio-shaped raw account access with framework
  macros, constraints, CLI, profiling, and generated client artifacts.
- **Hopper**: Hopper Native plus Hopper's typed layout, schema, segment,
  policy, receipt, migration, and CLI layers.

The comparison below is intentionally conservative. A row is a Hopper win only
when the repository contains the runtime, macro, test, or documentation surface
that backs it. Performance claims stay Hopper-vs-Quasar unless the sibling
`hopper-bench` repository publishes a same-provenance Anza Pinocchio run.

## External evidence snapshot

Reviewed May 2026:

- Anchor's zero-copy docs describe `AccountLoader`, `#[account(zero_copy)]`,
  direct account-byte casting, larger-account support up to 10 MB, and the
  10,240-byte CPI initialization limit for `init`.
- Quasar's public benchmark docs report a standard vault comparison where
  Quasar is in the same CU tier as Pinocchio and materially below Anchor's
  default Borsh path.
- Solana's native docs demonstrate the baseline account model: programs own
  data accounts and read/write account bytes directly through borrowed account
  data.
- Pinocchio's public docs position it as the minimal no-dependency Solana
  program substrate optimized for CU and binary size. Hopper treats Pinocchio as
  the substrate baseline, not as a same-level state framework.

## Executive verdict

Hopper is not merely another zero-copy cast helper. It is strongest when a
protocol wants **state contracts**: versioned layouts, typed fingerprints,
segment-level access, schema export, compatibility checks, receipts, policy
semantics, and cross-program typed reads.

Where Hopper currently stands:

- **Ahead of raw Solana/native zero-copy** on DX, safety rails, schema, and
  lifecycle tooling; raw native still wins on absolute minimal surface area.
- **Ahead of Anchor zero-copy** on `no_std`/`no_alloc`, layout evolution,
  segment-aware access, receipts, policy/lint semantics, and compute-tier
  positioning; Anchor still wins on ecosystem maturity and wallet/explorer/IDL
  adoption.
- **Ahead of Quasar** on state-contract semantics, segment roles, receipts,
  policy, migration, backend portability, and cross-program layout reads;
  Quasar still has a polished public profiler story and simpler public
  positioning.
- **Comparable to Pinocchio-class raw access** as an access model, but not a
  replacement for teams that want the thinnest possible substrate and are happy
  to hand-write every validation rule.

## Scorecard

Scores are launch-readiness grades, not theoretical ceilings.

| Dimension | Native Solana / raw zero-copy | Anchor zero-copy | Quasar zero-copy | Hopper |
|---|---:|---:|---:|---:|
| Raw speed ceiling | **A+** | B | **A** | **A** |
| Default safety rails | D | **A-** | B+ | **A** |
| Zero-copy DX | D | B+ | **A-** | **A-** |
| Large-account usability | C | **A** | B | B+ |
| Layout/version evolution | D | C | C+ | **A** |
| Schema/IDL/client story | D | **A+** | A- | A- |
| Segment/field-level borrow model | D | D | C | **A** |
| Cross-program typed reads | D | C | C | **A** |
| Mutation receipts / audit trail | D | D | D | **A** |
| Policy/lint semantics | D | B | B | **A** |
| CLI/profiler workflow | D | **A** | **A** | B+ |
| Ecosystem adoption | **A** | **A+** | B | C+ |
| Backend portability | N/A | C | C | **A** |
| Publication risk | Low | Low | Medium | Medium |

## Feature parity matrix

| Capability | Native Solana / raw | Anchor zero-copy | Quasar | Hopper |
|---|---|---|---|---|
| Direct account-byte access | Yes, manual | Yes through `AccountLoader` | Yes | Yes |
| No serialization round trip | Manual | Yes for `AccountLoader` | Yes | Yes |
| `no_std`/`no_alloc` on-chain posture | Manual | No, Anchor runtime oriented | Yes | **Yes** |
| Account constraints | Manual | **Mature** | Strong | Strong |
| Typed zero-copy account wrapper | Manual | `AccountLoader<T>` | Generated account wrappers | `Account<T>`, `AccountView`, layout contracts |
| Explicit `Clone + Copy` / Pod contract | Manual | Generated / bytemuck-based | Framework-owned | **Documented + compile-tested** |
| Field offset constants | Manual | Limited / IDL-side | Yes | **Yes, body + absolute offsets** |
| Layout fingerprint | Manual | Discriminator-focused | Partial | **8-byte layout ID + binary anchor** |
| Layout version | Manual | Manual | Partial | **First-class** |
| Schema manifest | Manual | IDL | IDL / client artifacts | **Layout manifest + Codama/Anchor IDL projection** |
| Schema compatibility checks | Manual | Mostly off-chain/manual | Partial | **First-class compatibility verdicts** |
| Cross-program typed layout reads | Manual | IDL/client oriented | Limited | **`hopper_interface!` / layout fingerprint checks** |
| Segment-aware field borrows | No | No | No/limited | **Yes** |
| Named segment roles | No | No | No | **Yes** |
| Receipts / mutation proof | No | No | No | **Yes** |
| Policy capability model | No | Constraints only | Constraints/profiles | **Capability/policy/lint surface** |
| Migration helpers | Manual | Manual / ecosystem patterns | Partial | **Versioned migration module** |
| CLI verify/layout explanation | Manual | Anchor CLI ecosystem | Quasar CLI/profiler | **Hopper CLI verify/explain/manager** |
| Public profiler maturity | Manual | Mature ecosystem | **Strong** | Good, still a polish lane |
| Raw escape hatch | Always | Possible, less idiomatic | Yes | **Yes, explicitly unsafe** |
| Backend portability | N/A | Solana program model | Pinocchio-shaped | **Native, Pinocchio compat, Solana-program backend** |

## DX and usability notes

### Native Solana / raw zero-copy

Native zero-copy is the control baseline. It is unbeatable when a team wants
only a byte slice, a pointer cast, and zero framework policy. The cost is that
all owner checks, signer checks, account order checks, layout invariants,
versioning, schema export, and client decoding become project-specific code.

**Hopper position:** Hopper should not hide that raw native remains valid for
small, expert-owned programs. Hopper wins when the state format is long-lived,
shared across programs, audited, indexed, or migrated.

### Anchor zero-copy

Anchor zero-copy is the ecosystem baseline. `AccountLoader` is practical,
documented, and familiar to teams already using Anchor. Anchor's ecosystem is
still the strongest for IDL expectations, wallet/explorer assumptions, examples,
and hiring.

**Hopper position:** Hopper should avoid claiming Anchor's entire framework is
slow; the honest comparison is Anchor's zero-copy path versus Hopper's safe
zero-copy path. Hopper's strongest advantages are no-alloc/no-std posture,
segment-aware access, layout fingerprints, compatibility checks, receipts, and
policy semantics.

### Quasar zero-copy

Quasar proves there is demand for Pinocchio-tier performance with Anchor-like
macros. Its public benchmark and profiler story are strong. It is the closest
competitive peer for Hopper's authoring model.

**Hopper position:** Hopper should compete with Quasar on state semantics, not
just CU. The launch claim should be: Hopper reaches the same performance class
while adding layout IDs, segment roles, receipts, policy, migration, backend
portability, and cross-program typed reads.

## Performance posture

The release-facing parity table remains Hopper-vs-Quasar only:

| Scenario | Hopper | Quasar |
|---|---:|---:|
| Authorize | **432 CU** | 585 CU |
| Auth-fail (missing sig) | 70 CU | **66 CU** |
| Counter (segment-safe) | **539 CU** | 607 CU |
| Deposit | **1651 CU** | 1768 CU |
| Withdraw | **455 CU** | 605 CU |
| Binary size | **7.62 KiB** | 8.36 KiB |

Interpretation:

- Hopper is faster than Quasar on four of five published instruction paths and
  has the smaller binary in this table.
- The auth-fail gap is negligible and not a protocol-level concern.
- The counter result is important because Hopper keeps segment-level safety in
  the measured path.
- No Pinocchio win claim is published until the benchmark repo records the same
  lockfile, SBF toolchain, Mollusk version, seed set, feature flags, release
  profile, and command line for an Anza Pinocchio target.

See [`BENCHMARKS.md`](../BENCHMARKS.md) for the full benchmark policy and
provenance requirements.

## Hopper innovation ledger

| Innovation | Why it matters | Competitor baseline |
|---|---|---|
| Layout IDs anchored in binaries | Lets tooling prove binary/manifest agreement | Native/Anchor/Quasar generally stop at discriminator or IDL identity |
| Segment-aware access | Mutate a field or segment without claiming the entire layout | Anchor/Quasar mostly operate at account or raw-slice granularity |
| Segment roles | Lets compatibility and policy reason about semantic regions | Manual elsewhere |
| Receipts | Compact mutation proof for logs/indexers/auditors | Not present in the compared zero-copy paths |
| Policy packs and semantic linting | Moves protocol intent into code-reviewable declarations | Anchor/Quasar constraints are account-centric |
| Cross-program typed reads | Lets another program pin a layout fingerprint without importing the source crate | Anchor IDL is mostly client-side; raw/native is manual |
| Multi-backend runtime | Lets teams pick native, compatibility, or Solana-program paths | Competitors are typically tied to one substrate |
| Hybrid fixed-body + dynamic tail | Keeps hot fields zero-copy while supporting bounded variable data | Anchor uses separate serialized parameter/account patterns; raw is manual |

## Final gap review

### Closed in this pass

- The `#[hopper::state]` Copy contract is now pinned with an explicit trybuild
  fail fixture. Missing `Clone + Copy` no longer relies on incidental downstream
  Pod errors; the generated proof is part of the tested public macro contract.

### Still worth polishing before a major launch

1. **Public profiler polish:** Quasar still has a cleaner profiler narrative.
   Hopper has benchmark and CLI pieces, but the one-command public profiler path
   should be made as obvious as `hopper profile` in the README and CLI docs.
2. **Warning-free release lane:** `cargo check --workspace --all-targets` passes,
   but examples and CLI still emit warnings. They do not block correctness, but
   a public launch reads better with either fixes or deliberate `allow` markers.
3. **Crate-name publication verification:** Before telling users to install
   `hopper = "0.1.0"`, verify the crates.io name and docs.rs page point to this
   project, not an unrelated historical crate.
4. **Independent benchmark artifacts:** The current Hopper-vs-Quasar table is
   useful, but launch claims should link immutable `hopper-bench` artifacts and
   exact commits.

## Bottom line

Hopper stands best as a **protocol-state zero-copy framework**. Native Solana
and Pinocchio remain the minimal-control baseline; Anchor remains the adoption
baseline; Quasar remains the closest performance/DX peer. Hopper's strongest
claim is that it packages the same zero-copy performance class with state
contracts that competitors mostly leave to each project: layout fingerprints,
segments, receipts, policy, migration, manager metadata, and cross-program typed
compatibility.