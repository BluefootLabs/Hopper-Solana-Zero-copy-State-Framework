# Hopper Native — Enhancement Plan

Hopper Native is the sovereign substrate. It already owns loader parsing,
syscalls, eager + lazy entrypoints, duplicate-account resolution, and
`AccountView`. This document is the source of record for what we absorb
from Pinocchio and Quasar's substrates, what we deliberately do not,
and the priority order.

> **Pinocchio is the Pareto frontier for raw substrate efficiency.
> Quasar is the Pareto frontier for substrate-plus-DX integration.**
> Hopper Native should aim to be substrate-competitive with Pinocchio
> and DX-competitive with Quasar — without becoming a copy of either.

## Substrate-boundary commitment (Option A)

Hopper Native is the canonical substrate. `legacy-pinocchio-compat` and
`solana-program-backend` are **compat shims** for users with existing
dep trees, not peer targets. Enhancement effort goes into Hopper Native.
The `legacy-pinocchio-compat` feature surface is frozen.

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
| [hopper-svm](https://github.com/BluefootLabs/hopper-svm) | In-process Solana execution harness for tests. |

## Status

| # | Item | Status |
|---|------|--------|
| 1.1 | Public `process_entrypoint` | ✅ shipped (`hopper_native::entrypoint::process_entrypoint`) |
| 1.2 | `MAX_TX_ACCOUNTS` configurability | ✅ shipped (`hopper_program_entrypoint!(fn, max)`) |
| 1.3 | `no_allocator!` macro | ✅ shipped |
| 1.4 | `hopper-log` crate | ⏳ planned |
| 1.5 | Static syscalls feature | ✅ feature flag added (`static-syscalls`) |
| 1.6 | Anza modular SDK 2.x audit | ⏳ planned |
| 2.1 | Pod arithmetic operator overloads | ✅ shipped on all `Le*` wire types |
| 2.2 | Wrapping in release / panic in debug | ✅ matches Rust default via direct `+`/`-` |
| 2.3 | Compile-time discriminator dispatch | ⏳ audit pending |
| 2.4 | Self-CPI event emission | ⏳ planned |
| 2.5 | `init_if_needed`, `realloc`, `close` parity | ⏳ audit pending |

## Tier 3 — explicitly not porting

- Anchor's `Account<'info, T>` / `Signer<'info>` runtime types verbatim.
  Hopper's modifier composition (`Signer<Mut<Account<'a, T>>>`) is the
  canonical surface.
- Quasar's IDL-by-default. We separate that into `hopper-schema`.
- Bump allocator on by default. `no_alloc` stays default; heap is opt-in.
- Pinocchio-style "zero deps" minimalism for the whole framework. We
  keep `bytemuck`, `sha2-const-stable`, `five8_const`.

## Priority order for the next quarter

1. `hopper-log` crate (1.4)
2. Self-CPI event pattern (2.4)
3. Anza modular SDK 2.x migration audit (1.6)
4. Compile-time dispatch table audit (2.3)
5. Anchor-keyword parity audit (2.5)
