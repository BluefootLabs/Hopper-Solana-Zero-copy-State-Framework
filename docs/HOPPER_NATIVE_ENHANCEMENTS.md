# Hopper Native - Enhancement Plan

Hopper Native is the sovereign substrate. It already owns loader parsing,
syscalls, eager + lazy entrypoints, duplicate-account resolution, and
`AccountView`. This is a dated roadmap snapshot for what Hopper considered
absorbing from Pinocchio and Quasar's substrates. The current source, crate
API documentation, and release evidence are authoritative when a status below
ages.

> **Pinocchio is the Pareto frontier for raw substrate efficiency.
> Quasar is the Pareto frontier for substrate-plus-DX integration.**
> Hopper Native should aim to be substrate-competitive with Pinocchio
> and DX-competitive with Quasar - without becoming a copy of either.

## Substrate-boundary commitment (Option A)

Hopper's direct runtime is the canonical substrate. Old backend feature names
are compatibility aliases only; they do not select peer runtime targets.
Enhancement effort goes into Hopper's own account-memory runtime.

## Product repositories

Framework-internal crates live in this repo so Hopper's runtime, core, macros,
schema, SPL wrappers, and CLI evolve together. The earlier one-repo-per-crate
split was folded back with subtree history preserved; those temporary repos are
archived/private.

Only coherent standalone products remain public siblings:

| Repo | Purpose |
|------|---------|
| [Hopper-Solana-Zero-copy-State-Framework](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework) | Main framework workspace. |
| [hopper-bench](https://github.com/BluefootLabs/hopper-bench) | Cross-framework benchmark harness and CU regression lab. |

`hopper-svm` is no longer a sibling product. Its current source and release
contract live in-tree at `crates/hopper-svm` so the host harness stays locked to
the account-view/runtime versions it exercises.

## Status

| # | Item | Status |
|---|------|--------|
| 1.1 | Public `process_entrypoint` | ✅ shipped (`hopper_native::entrypoint::process_entrypoint`) |
| 1.2 | `MAX_TX_ACCOUNTS` configurability | ✅ shipped (`hopper_program_entrypoint!(fn, max)`) |
| 1.3 | `no_allocator!` macro | ✅ shipped |
| 1.4 | `hopper-log` crate | ⏳ planned |
| 1.5 | Static syscalls feature | ✅ feature flag added (`static-syscalls`) |
| 1.6 | Anza modular SDK 2.x audit | ⏳ planned |
| 2.1 | Wire integer arithmetic convenience | ✅ shipped on all `Wire*` integer types (`+`, `-`, `*`, `+=`, `-=`, `*=`) |
| 2.2 | Wrapping in release / panic in debug | ✅ matches Rust default for direct operators; checked helpers stay explicit |
| 2.3 | Compile-time discriminator dispatch | ✅ shipped at the framework macro layer: `crates/hopper-macros-proc/src/program.rs` emits deterministic match dispatch and a dense tiny-profile function table, with expansion tests. Further substrate tuning remains benchmark work. |
| 2.4 | Self-CPI event emission | ✅ shipped: `#[hopper::context(event_cpi)]`, `Context::emit_event_cpi`, and the authenticated event-sink dispatcher are implemented and tested. |
| 2.5 | `init_if_needed`, `realloc`, `close` parity | ✅ shipped in `crates/hopper-macros-proc/src/context.rs`, including generated lifecycle helpers and constraint validation. Combination restrictions remain compile-time errors documented by the context macro. |

## Tier 3 - explicitly not porting

- Anchor's runtime types verbatim. Hopper's public surface is Hopper-owned:
  `Account<'info, T>`, `InitAccount<'info, T>`, `Signer<'info>`, and the
  lower-level modifier composition remain separate layers.
- Quasar's IDL-by-default. We separate that into `hopper-schema`.
- Bump allocator on by default. `no_alloc` stays default; heap is opt-in.
- Pinocchio-style "zero deps" minimalism for the whole framework. We retain
  selected dependencies where they buy a reviewed capability; Hopper now owns
  its feature-independent const SHA-256 layout-ID implementation.

## Remaining roadmap items in this snapshot

1. `hopper-log` crate (1.4).
2. Anza modular SDK 2.x migration audit (1.6).

Compile-time dispatch, self-CPI events, and the listed lifecycle keywords are
shipped surfaces, so follow-up work there is measurement, hardening, and
same-behavior DX review rather than initial implementation.
