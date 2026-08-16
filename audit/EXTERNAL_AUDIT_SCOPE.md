# Hopper independent security review scope

Status: in progress, scope and evidence preparation  
Started: 2026-08-16  
Independent reviewer: not yet engaged  
Target revision: pending the clean release-candidate commit

This file starts the external-review workflow without claiming that an
independent audit is under way. The release gate remains blocked until a named
independent reviewer examines a clean, immutable revision and a final report is
registered in `audit/readiness.json`.

## Review objectives

The review must determine whether untrusted Solana instruction input can:

1. bypass owner, signer, writable, discriminator, version, layout identity, PDA,
   or account-size checks;
2. mutate data bytes or lamports outside the instruction's declared effect
   contract;
3. create aliased mutable access, including duplicate account keys and
   overlapping segment leases;
4. exploit grow, shrink, lazy, epoch, or route migrations to corrupt state,
   steal rent, skip initialization, or leave a partially valid layout;
5. make generated clients emit account privileges or instruction bytes that do
   not match the on-chain declaration;
6. make manifest, fuzz-plan, Grillo verification, ABI, or release evidence refer
   to a different program contract than the deployed binary; or
7. reach unsafe compatibility surfaces without satisfying their documented
   proof obligations.

## In-scope components

- `crates/hopper-native`: loader input parsing, raw account projection, lazy
  access, program entry, and error containment.
- `crates/hopper-runtime`: account validation, borrows, write policies,
  lamports, migrations, CPI boundaries, segment leases, foreign accounts, and
  compatibility adapters.
- `crates/hopper-core`: layouts, headers, bounded collections, dynamic tails,
  receipts, and migration primitives.
- `crates/hopper-macros-proc` and `crates/hopper-macros`: generated contexts,
  entrypoints, clients, layout descriptors, and declaration consistency.
- `crates/hopper-schema`: canonical manifests, fingerprints, compatibility,
  client generation, and instruction/account metadata.
- `crates/grillo-manifest` and `crates/grillo-verifier`: effect-contract parsing,
  invocation binding, state-transition verification, and fail-closed behavior.
- Security-critical `hopper-cli` gates: manifest emission, fuzz-plan generation,
  binary verification, publication checks, transaction limits, and audit checks.
- Cicada, Sentinel, migration, strict-write, alias, and loader-conformance
  fixtures used to demonstrate the guarantees above.

## Explicitly out of scope

- Solana validator consensus, runtime, loader, and SDK correctness except where
  Hopper makes an integration assumption about them.
- Wallet, RPC-provider, operating-system, and hardware compromise.
- Application economics or business logic that is not generated or enforced by
  Hopper.
- Legacy compatibility modules that are not enabled by the release feature set.
- Performance leadership claims. Those require the separate clean peer
  benchmark artifact and are not security conclusions.

## Required invariants

The reviewer should treat the following as claims to falsify, not assumptions:

- Typed access is granted only after identity, ownership, role, size, and layout
  validation appropriate to the selected trust profile.
- A strict-write instruction cannot obtain or commit a mutation outside its
  resolved byte ranges or lamport-account set.
- Parametric write ranges are resolved from the authenticated instruction wire
  arguments, with bounds and arithmetic checked before authority is granted.
- Duplicate pubkeys cannot yield incompatible mutable capabilities.
- A successful migration leaves exactly one valid target layout. A failed
  migration does not expose partially migrated typed state.
- Growth is payer-authorized, rent-aware, and zero-initialized. Shrink refunds
  only the rent made excess by the final valid size.
- Generated clients preserve signer, writable, PDA, argument, and discriminator
  declarations from the same manifest used by the program and verifier.
- Layout, semantic, contract, invocation-frame, and deployment commitments are
  domain-separated and cannot be silently substituted across artifacts.
- Unsafe blocks and raw escape hatches satisfy the obligations recorded in
  `docs/UNSAFE_INVARIANTS.md`.

## Reviewer evidence package

Before handoff, the release candidate must include:

- a clean signed or otherwise immutable commit and its complete source archive;
- locked Rust and SBF toolchains plus reproducible build commands;
- the dependency-advisory report with reachability and disposition;
- host, compiled-SBF, fuzz, Kani, Miri where applicable, and trybuild results;
- emitted program manifest, deterministic fuzz plan, generated clients, release
  `.so`, binary ABI verification output, and SHA-256 hashes;
- current five-framework same-behavior benchmark sources and raw results;
- architecture, unsafe-invariant, network-baseline, and prior-finding documents;
  and
- a remediation log mapping every finding to code, regression tests, and the
  reviewed fix revision.

## Reproduction entry points

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
cargo audit --no-fetch
cargo run -p hopper-cli -- publish-check --source-only --full
cargo run -p hopper-cli -- audit-check --strict --json
```

The binary-mode publication command and compiled-SBF commands must be recorded
against the final release candidate rather than copied from an older artifact.

## Completion rule

`externalAudit.status` may become `independent-complete` only after the named
reviewer delivers a report for the immutable target revision, required fixes
are merged and retested, and the report path exists in the evidence bundle.
