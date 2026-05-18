# Hopper Docs

Start here for the current Hopper framework surface:

- [FIRST_FIVE_MINUTES.md](FIRST_FIVE_MINUTES.md) - the shortest path through `#[account]`, `#[derive(Accounts)]`, `#[program]`, `Ctx<T>`, and `ctx.accounts.*`.
- [GETTING_STARTED_SERIOUS.md](GETTING_STARTED_SERIOUS.md) - a source-first walkthrough for a real program shape.
- [WRITING_HOPPER_PROGRAMS.md](WRITING_HOPPER_PROGRAMS.md) - handler, account, initialization, and wrapper patterns.
- [HOPPER_LAYERS.md](HOPPER_LAYERS.md) - when to stay in framework mode and when to reach for systems mode.
- [DYNAMIC_TAILS_FROM_QUASAR.md](DYNAMIC_TAILS_FROM_QUASAR.md) - bounded dynamic fields, generated tail helpers, and explicit tail wiring.
- [PORT_QUASAR_IN_20_MINUTES.md](PORT_QUASAR_IN_20_MINUTES.md) - hands-on Quasar-style dynamic account port.
- [../examples/hopper-devnet-audit/README.md](../examples/hopper-devnet-audit/README.md) - devnet audit program for dynamic tails, segments, and substrate probes.
- [HOPPER_VS_ANCHOR_QUASAR_PINOCCHIO.md](HOPPER_VS_ANCHOR_QUASAR_PINOCCHIO.md) - positioning and benchmark language without overclaiming.
- [QUASAR_PINOCCHIO_REPLACEMENT.md](QUASAR_PINOCCHIO_REPLACEMENT.md) - mapped replacement surface and benchmark provenance rules.

Historical research, presplit notes, and older systems-first sketches live in
[archive/](archive/). They may mention older spellings such as
`#[hopper::context]`, `AccountView`, or manual entrypoints, and should not be
used as the first-touch authoring model.