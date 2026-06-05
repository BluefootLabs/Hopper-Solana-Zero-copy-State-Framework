# Hopper Docs

Start here for the current Hopper framework surface:

- [FIRST_FIVE_MINUTES.md](FIRST_FIVE_MINUTES.md) - the shortest path through `#[account]`, `#[derive(Accounts)]`, `#[program]`, `Ctx<T>`, and `ctx.accounts.*`.
- [GETTING_STARTED_SERIOUS.md](GETTING_STARTED_SERIOUS.md) - a source-first walkthrough for a real program shape.
- [WRITING_HOPPER_PROGRAMS.md](WRITING_HOPPER_PROGRAMS.md) - handler, account, initialization, and wrapper patterns.
- [HOPPER_LAYERS.md](HOPPER_LAYERS.md) - when to stay in framework mode and when to reach for systems mode.
- [DYNAMIC_TAILS_FROM_QUASAR.md](DYNAMIC_TAILS_FROM_QUASAR.md) - bounded dynamic fields, generated tail helpers, and explicit tail wiring.
- [DYNAMIC_FIELDS_QUASAR_TO_HOPPER.md](DYNAMIC_FIELDS_QUASAR_TO_HOPPER.md) - side-by-side bounded dynamic field migration and Hopper's compact-tail contract.
- [EXTERNAL_ACCOUNTS.md](EXTERNAL_ACCOUNTS.md) - adapter-checked zero-copy for non-Hopper accounts, typed views, lenses, snapshots, lazy remaining accounts, and grouped tails.
- [LARGE_ZERO_COPY_ACCOUNTS.md](LARGE_ZERO_COPY_ACCOUNTS.md) - pre-created large accounts, `load_init()`, segment-safe queues, and external large-account adapters.
- [CRYPTO_CAPABILITIES.md](CRYPTO_CAPABILITIES.md) - shipped Solana crypto helpers, precompile checkers, and feature-gated heavy crypto wrappers.
- [HOPPER_VS_QUASAR.md](HOPPER_VS_QUASAR.md) - one-page product/technical comparison: write like Quasar, Hopper checks the cast.
- [PORT_QUASAR_IN_20_MINUTES.md](PORT_QUASAR_IN_20_MINUTES.md) - hands-on Quasar-style dynamic account port.
- [../examples/hopper-tail-lab/README.md](../examples/hopper-tail-lab/README.md) - devnet tail lab for bounded fields, `TailStr`, `TailBytes`, init helpers, and account wrappers.
- [../examples/hopper-devnet-audit/README.md](../examples/hopper-devnet-audit/README.md) - devnet audit program for dynamic tails, segments, and substrate probes.
- [PROFILING.md](PROFILING.md) - `hopper profile elf`, binary profile artifacts, and reproducible benchmark commands.
- [PROTOCOL_GRADE_EXAMPLES.md](PROTOCOL_GRADE_EXAMPLES.md) - receipt indexing, compatibility reports, migration plans, typed cross-program reads, and segment leases.
