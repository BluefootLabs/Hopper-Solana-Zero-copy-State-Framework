# Hopper Docs

Start here for the current Hopper framework surface:

- [FIRST_FIVE_MINUTES.md](FIRST_FIVE_MINUTES.md) - the shortest path through `#[account]`, `#[derive(Accounts)]`, `#[program]`, `Ctx<T>`, and `ctx.accounts.*`.
- [GETTING_STARTED_SERIOUS.md](GETTING_STARTED_SERIOUS.md) - a source-first walkthrough for a real program shape.
- [WRITING_HOPPER_PROGRAMS.md](WRITING_HOPPER_PROGRAMS.md) - handler, account, initialization, and wrapper patterns.
- [HOPPER_LAYERS.md](HOPPER_LAYERS.md) - when to stay in framework mode and when to reach for systems mode.
- [DYNAMIC_TAILS.md](DYNAMIC_TAILS.md) - bounded dynamic fields, generated tail helpers, and explicit tail wiring.
- [BOUNDED_FIELDS.md](BOUNDED_FIELDS.md) - bounded dynamic account fields and Hopper's compact-tail contract.
- [EXTERNAL_ACCOUNTS.md](EXTERNAL_ACCOUNTS.md) - adapter-checked zero-copy for non-Hopper accounts, typed views, lenses, snapshots, lazy remaining accounts, and grouped tails.
- [LARGE_ZERO_COPY_ACCOUNTS.md](LARGE_ZERO_COPY_ACCOUNTS.md) - pre-created large accounts, `load_init()`, segment-safe queues, and external large-account adapters.
- [COLLECTIONS_AND_RESIZING.md](COLLECTIONS_AND_RESIZING.md) - choose bounded fields, growable `Seq`, stable-ID `Slab`, and safe grow/fit migrations without weakening write contracts.
- [CRYPTO_CAPABILITIES.md](CRYPTO_CAPABILITIES.md) - shipped Solana crypto helpers, precompile checkers, and feature-gated heavy crypto wrappers.
- [PROGRAM_CAPABILITIES.md](PROGRAM_CAPABILITIES.md) - implemented execution capabilities and application responsibilities.
- [BOUNDED_MULTISIG.md](BOUNDED_MULTISIG.md) - bounded member storage, authenticated approvals, and SOL custody.
- [../examples/hopper-tail-lab/README.md](../examples/hopper-tail-lab/README.md) - devnet tail lab for bounded fields, `TailStr`, `TailBytes`, init helpers, and account wrappers.
- [../examples/hopper-devnet-audit/README.md](../examples/hopper-devnet-audit/README.md) - devnet audit program for dynamic tails, segments, and substrate probes.
- [PROFILING.md](PROFILING.md) - `hopper profile elf`, binary profile artifacts, and reproducible benchmark commands.
- [PROTOCOL_GRADE_EXAMPLES.md](PROTOCOL_GRADE_EXAMPLES.md) - receipt indexing, compatibility reports, migration plans, typed cross-program reads, and segment leases.
- [EFFECT_ABI_V0_1.md](EFFECT_ABI_V0_1.md) - framework-neutral static and invocation-parametric write effects, Grillo verification, and the exact v0.1 nonclaims.
- [EFFECT_ABI_V0_2.md](EFFECT_ABI_V0_2.md) - full account-state transition contracts, deployment binding, CPI envelopes, the fail-closed invocation-frame binding, and the shared manifest-commitment that threads runtime containment, verification, and placement.
