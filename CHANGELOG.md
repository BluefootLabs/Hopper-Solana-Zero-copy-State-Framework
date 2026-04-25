# Changelog

All notable changes to Hopper land here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once
1.0 ships; pre-1.0 minor versions may break the API.

## [Unreleased]

### Added

- **`#[derive(Accounts)]` — Anchor-spelled drop-in for `#[hopper::context]`.**
  Anchor users porting code now get the spelling they expect without
  losing any of Hopper's account-attr surface. Identical generated code
  to the attribute form: same binder type, same accessors, same
  constraint validation pipeline, same Hopper-specific sugar
  (segment-tagged `mut(field, …)`, `read(...)`, the inline
  `#[hopper::pipeline]` / `#[hopper::receipt]` / `#[hopper::invariant]`
  stack). The full Anchor-grade constraint set is supported in either
  spelling: `init`, `init_if_needed`, `mut`, `signer`, `seeds`, `bump`,
  `payer`, `space`, `has_one`, `owner`, `address`, `constraint`,
  `token::*`, `mint::*`, `associated_token::*`, the Token-2022
  extension gates (`non_transferable`, `immutable_owner`,
  `transfer_hook`, `metadata_pointer`, …), `dup`, `sweep`,
  `executable`, `rent_exempt`, `realloc`, `zero`, `close`. The derive
  registers `account`, `signer`, `instruction`, and `validate` as
  helper attributes so `#[account(...)]`, `#[signer]`,
  `#[instruction(...)]`, and `#[validate]` field/struct annotations
  compile without an extra `use`. Lives in
  `hopper-macros-proc::derive_accounts`, exported through the
  `Accounts` symbol from `hopper::prelude::*`. Implementation reuses
  `context::expand_inner` behind a single `emit_struct: bool` flag —
  zero duplication of the constraint surface.
- **CLI polish pass two — final Quasar parity sweep.** Three commands
  Quasar shipped that Hopper still lacked, plus shared visual polish.
  - **`hopper add [-i|-s|-e <name>]`** — incremental scaffolding for
    an existing project. `-i/--instruction` creates
    `src/instructions/<name>.rs` with a Hopper-shaped context stub,
    wires it through `src/instructions/mod.rs` and
    `mod instructions;` in `lib.rs`, and (for projects using the
    `#[hopper::program]` style dispatch) injects a stub
    `#[instruction(N)]` handler at the next-available
    discriminator. For projects using a manual `match *disc` block,
    prints a "wire it in by hand" hint instead of guessing. `-s/--state`
    creates or appends to `src/state.rs` with a
    `#[hopper::state(disc = N, version = 1)]` struct picking the
    next-unused discriminator. `-e/--error` creates or appends to
    `src/errors.rs` with a discriminated `pub enum` plus a
    `From<...> for ProgramError` impl. All edits idempotent —
    re-running on the same name errors rather than overwriting.
  - **`hopper clean [-a|--all]`** — clear `target/{deploy,idl,client,
    profile,hopper}` while preserving `*-keypair.json` files (losing
    a program keypair means losing the on-chain program address —
    Quasar makes the same exception). With `-a`, also runs
    `cargo clean` for a full target wipe.
  - **Animated `hopper init` opening — leap reveal.** First-time
    interactive runs play a one-second FIGlet `HOPPER` animation:
    each row arrives from below with an ease-out-back bounce,
    leaving a trail of green grass-dots. Quasar has the blue nebula
    sweep; Hopper gets the leap. Auto-disables on subsequent runs
    (the wizard sets `ui.animation = false` after the first save),
    when stdout isn't a TTY, when `NO_COLOR` is set, or when the
    user toggles it off in `~/.hopper/wizard.toml`.
  - **Shared `style` module.** Centralised `bold`, `dim`, `color`,
    `success`, `fail`, `warn`, `step`, `human_size` helpers under
    `tools/hopper-cli/src/style.rs`. Auto-respects `NO_COLOR` and
    TTY detection, with explicit `--no-color` flag and
    `ui.color = false` config override. Replaces the ad-hoc ANSI
    escapes that were sprinkled through `cmd::lifecycle`. Build-size
    delta lines now colour the delta itself: green when shrinking,
    yellow when growing.
- **CLI polish — Quasar parity-plus pass.** `hopper init` now drops into
  an interactive `dialoguer` wizard when invoked without a `<path>`,
  prompting for project name, template, testing framework, and git
  policy. Choices are persisted to `~/.hopper/wizard.toml` so the
  second run skips the prompts. Four templates ship: `minimal`,
  `nft-mint` (uses `hopper-metaplex`), `token-2022-vault` (extension
  screening), `defi-vault` (segment-safe authority + balance with
  PDA verification). Pass `--template <name>` to pick one
  non-interactively, `--yes`/`-y` to use saved defaults without
  prompts, `--no-git` to skip git, `--interactive` to force the
  wizard even with a path.
- **`Hopper.toml` project config.** `hopper init` writes a declarative
  `Hopper.toml` at the project root with `[project]`,
  `[toolchain]`, `[testing]`, and `[backend]` sections. The rest of
  the CLI reads it to know how to build / test / deploy.
- **Binary-size delta on `hopper build`.** SBF builds now print a per-
  artefact summary: `✔ my_program.so   56.6 KiB  (-1.2 KiB)`. New
  binaries print `(new)`; unchanged binaries are silent.
- **Git automation in `hopper init`.** `commit` policy runs `git init`
  + initial commit; `init` policy runs `git init` only; `skip`
  leaves git alone.
- **`hopperzero.dev`** is the canonical project domain. Crate metadata,
  README headers, and footer links now point there.
- **`hopper-metaplex`** crate (optional, behind `--features metaplex`):
  `CreateMetadataAccountV3`, `CreateMasterEditionV3`,
  `UpdateMetadataAccountV2` builders, plus `metadata_pda` /
  `master_edition_pda` derivation helpers and a `BorshTape`
  stack-buffer encoder. Closes the Quasar-parity Metaplex gap.
- **`examples/hopper-nft-mint`** — reference NFT-mint program using the
  new Metaplex builders end-to-end (1-of-1 NFT with locked master
  edition).
- **`bench/anchor-vault`** — in-tree Anchor parity vault using
  `AccountLoader<CounterState>` for zero-copy counter access. Bench
  harness now prefers the in-tree binary over `--anchor-root` when
  present.
- **`bench/pinocchio-vault`** — in-tree Anza Pinocchio parity vault.
  Replaces the previous "Pinocchio-style" column that loaded a
  Quasar-authored reference vault. The bench `--quasar-root` flag is
  now optional.
- **`bench/lazy-dispatch-vault`** — eight-instruction dispatch vault
  built twice (eager + lazy) so the lazy-entrypoint CU win is
  directly measurable.
- **`examples/hopper-token-2022-transfer-hook`** — Token-2022 transfer
  hook validation reference program.
- **DSL parity additions**: `#[derive(HopperInitSpace)]` standalone
  derive; `#[hopper::access_control(expr)]` handler attribute;
  `executable` and `rent_exempt = enforce|skip` field keywords;
  `init_if_needed` field keyword; `hopper_load!(slice => [a, b])`
  destructuring sugar; `err!` and `error!` short-form aliases in the
  prelude.
- **`hopper schema export --anchor-idl`** — emit Anchor 0.30-shaped
  IDL JSON from a `ProgramManifest`. Codama remains the preferred
  interop path; this exists for the long tail of wallets/explorers
  that still consume Anchor IDL.
- **`hopper-runtime::rent`** — `check_rent_exempt(account)` and
  `minimum_balance(data_len)` helpers backing the
  `rent_exempt = enforce` field keyword.
- **`docs/UNSAFE_INVARIANTS.md`** — supplemented with the full
  hopper-native unsafe surface (every `AccountView` unsafe method,
  `raw_input.rs`, `lazy.rs`, `pda.rs`, `mem.rs`, plus the expanded
  `cpi.rs` invariants).
- **Per-crate READMEs** for every crate under `crates/`, with
  hopperzero.dev / docs.rs / crates.io badges.
- **`AUDIT.md`** — comprehensive deep-review of Hopper vs Pinocchio,
  Quasar, Anchor zero-copy with verified parity findings, gap list,
  and implementation-pass record.

### Changed

- **README.md** Getting Started leads with the proc-macro path
  (`#[hopper::state]` / `#[hopper::program]`) for Anchor refugees;
  declarative `hopper_layout!` is the "Day Two" subsection.
- **Bench column** previously labelled "Pinocchio-style" is now
  "Pinocchio" (Anza). Pre-rename CU numbers retained with a
  `(deprecated)` marker so the historical record stays intact.
- **README** "What You Get" table extended with the new capabilities
  (Metaplex builders, Anchor IDL emit, full guard family,
  constraint vocabulary, Token-2022 extensions, handler attributes).
- **`cpi::invoke_unchecked` / `cpi::invoke_signed_unchecked`** now
  carry explicit seven-item `# Safety` invariant lists.
- **README "Segment-level borrows" row** correctly attributes
  `SegmentBorrowRegistry` to `hopper-runtime` (not `hopper-core`).

### Pre-publication checklist

- LICENSE: Apache-2.0, present at repo root.
- CONTRIBUTING.md: see [CONTRIBUTING.md](CONTRIBUTING.md).
- SECURITY.md: see [SECURITY.md](SECURITY.md).
- All published crates have `homepage = "https://hopperzero.dev"`,
  `documentation = "https://docs.rs/<crate>"`, and a per-crate
  `README.md`.

## [0.1.0] — initial publication target

First public release of the Hopper framework. See
[`AUDIT.md`](AUDIT.md) for a full feature audit.
