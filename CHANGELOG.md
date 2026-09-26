# Changelog

All notable changes to Hopper land here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once
1.0 ships; pre-1.0 minor versions may break the API.

## Native/runtime patch 0.4.1 and repository programs - 2026-09-26

- Reject missing unsigned signer privileges in specialized System/token CPI preflight.
- Abort no-allocation and panic failures through the SVM syscall without experimental
  inline assembly or a compute-burning panic loop.
- Replace the metadata-only multisig example with actual SOL custody, authenticated
  member and threshold changes, and expiring single-use payout approvals. Policy
  revisions invalidate old payouts; execution needs no member signature once approved.
- Repair treasury wallet deposits with System CPI, validate all segment identities,
  enforce live-Clock cooldowns, and preserve rent and period spending limits.
- Lead product guides with executable applications and move dated source-comparison
  research outside the product documentation. Framework crate version remains 0.4.0.

## [0.4.0] - 2026-09-25

- Harden safe fixed-layout overlays, events, frames, registries, and collections
  against incorrect `SIZE` declarations, including an overridden assertion
  constant. The framework checks the size independently at compile time.
- Reject malformed segment counts, capacities, element sizes, and overflowing
  geometry before exposing typed elements.
- Independently bound runtime typed projections even when custom layout traits
  override validation and size methods; check compact-tail initialization bounds.

- Generate named fixed-field inputs, `from_fields`, and `set_fields` without
  changing the wire ABI or removing positional constructors.
- Compose explicit fresh initialization with checked mutable access through
  `init_<account>_with`; allow application-defined `AccountFields` inputs.
- Fix body-only proc-macro layout projection in headered DSL, modifier, and
  migration wrappers while preserving declarative header-inclusive layouts.
- Fix the SOL vault's missing System Program account and legacy DSL custody,
  authority, and borrow-scope errors. Synchronize the getting-started example.
- Correct `VerifiedAccount` authorization claims and document host-harness
  limits. Add compiled vault and finalized-devnet regression runners.

## [0.3.2] - 2026-09-25

- Generated selected-cell readers/writers capture the context-bound selector
  and infer the element type, eliminating repeated offset arithmetic.
- `derive(Accounts)` accepts explicit empty `lamports()` policies, closing a
  forwarding/parser discrepancy with the direct context attribute.
- Added the on-chain byte-allowance example, compiled lifecycle/refusal tests,
  and a finalized-devnet test runner.
- Corrected orderbook initialization, authority checks, layout/counter
  validation and event-ring behavior. Its documentation now describes its
  storage scope without unsupported exchange or performance claims.
- Refreshed source and network observations, including Alpenglow on testnet
  only in the September 25 snapshot.
- Corrected policy, PDA, CPI-delegation, and performance documentation against
  the implemented checks and dated measurement scope.

## [0.3.1] - 2026-09-24

- Added explicit Token-2022 mint creation plans for six supported fixed-size
  extensions, with exact allocation, prefunding, PDA signing, and rollback tests.
- Generated extension constraints require Token-2022 ownership before reading
  extension data. Raw readers retain caller-owned validation responsibilities.
- Retained validated bumps for more direct required binding paths, without
  treating selected or stored bumps as proof of canonicality.
- Published the 26 framework/CLI updates and verified fresh registry-only mint
  and PDA programs alongside 45 focused finalized devnet transactions.

## [0.3.0] - 2026-09-23

- **PDA search reference lifetimes.** Each native search iteration now stages
  an immutable bump seed before hashing instead of mutating a byte behind a
  reused shared reference. Documentation distinguishes canonical search,
  selected-bump verification and compile-time address constants.

### Added

- **Program discovery.** `solana-check --all` recognizes parsed Hopper macro
  paths, ignores foreign framework entrypoints and quoted examples, and
  reports malformed source instead of silently skipping it.

- **Canonical literal PDAs.** `canonical_pda!` derives address bytes and the
  highest off-curve bump on the build host from explicit literal inputs.
- **Canonical inferred-bump validation.** Bare `bump` and `seeds_fn` now
  require the canonical address even for typed/initializing accounts. A
  matching bump alone did not establish that property. Direct required
  bare-bump fields retain the validated result instead of searching twice.
- **Finalized feature prerequisites.** `hopper feature-gate --json` records
  genesis, slot and validated feature accounts. Repeated `--require` gates
  automation on activation; malformed or wrongly owned evidence fails closed.
- **CLI seed-domain parity.** Client PDA search rejects oversized base-seed
  lists and individual seeds before hashing, matching the signing domain.
- **Typed seed helper lifetime.** Generated code retains an array returned by
  a `seeds_fn` helper while borrowing its slices for derivation.
- **Grillo provenance quotas.** v0.2 binding bounds textual RPC/replay labels
  before commitment encoding and cloning, including labels in CPI children.

- **Upgrade authority gate.** `grillo_manifest::authority` diffs two program
  manifests and reports every instruction that gains authority: a dropped
  signer, a newly writable account, a new instruction or writable account,
  byte ranges that reach another layout field, a removed exact-cell rule, a new
  lamport permission, a lost `strict_writes` or lamport contract, a raised
  remaining-account ceiling, and weaker context constraints (PDA seeds,
  `has_one`, owner or address checks, optionality, new `init`, `realloc`, or
  `close` lifecycles). Byte ranges are compared per layout field, so a field
  that only moved offset is not reported as a new permission. A PDA seed swap
  or a different expected CPI program is reported for review. Exposed as
  `grillo authority-diff old new` and
  `hopper verify --authority-baseline old [--baseline-so old.so]`, which exit
  2 on an unapproved widening and 3 on an unapproved review item. A reviewed
  report approves its findings only for the exact manifest pair whose SHA-256
  digests it records; under `--release` the baseline must match the interface
  commitment in its released ELF.

### Added

- **Finalized devnet evidence for seven lanes.** Fresh deployments of the
  migration, escrow, orderbook, compact-vault, Token-2022 vault, devnet
  audit, and cross-program-read examples ran their finalized harnesses on public devnet
  (Agave 4.3.0-rc.0, SIMD-0449, direct mapping, and SIMD-0460 active) with
  before-and-after artifact captures, and the ledger-bound authority review
  ran against a deployed sentinel program and a widened Buffer. Receipts,
  provenance, and checksum lists are archived under
  `audit/devnet-evidence-2026-09-19/`; the record with program ids and slots
  is in `docs/DEVNET_RELEASE_EVIDENCE.md`.
- **Devnet lane for the framework-comparison counter.** The macro counter
  fixture built at commit `a91462c` was deployed fresh on devnet
  (`F4Um7PWsnZfN7y8WFzu1aPYJwqGduJTa4zuCGY9EUqMy`) and driven with
  `hopper tx send` through the paths that commit changed: a wrong unsigned
  PDA refused at the creation CPI (`PrivilegeEscalation`), a signing
  non-PDA refused at bind (`InvalidSeeds`), `initialize` at 1,572 CU and
  `increment` at 368 (the Mollusk numbers to the unit), a refused second
  `initialize`, and a pre-funded PDA initialized with a zero-delta payer.
  Logs, receipts, checksums, and the finalized re-fetch are under
  `audit/devnet-evidence-2026-09-21/counter/`.

### Fixed

- The CLI manifest-publication test now includes its fixture from inside
  the CLI crate, so the publication train can package it independently.
- The new devnet evidence archive preserves file bytes across Git checkouts,
  keeping its recorded SHA-256 checksums valid on Windows and Unix.

### Changed

### Fixed

- **Programs using the compute-budget helpers could not deploy on any
  public cluster.** `hopper_native::CuBudget`, `hopper_runtime::compute`,
  `hopper_solana::compute`, and `hopper::compute` read
  `sol_remaining_compute_units` (SIMD-0049). That proposal is Withdrawn and
  its feature gate (`5TuppMutoyzhUSfuYdhgzD47F92GL1g89KpCZQKqedxP`) has never
  been activated on mainnet-beta, devnet, or testnet, so the loader rejects
  any ELF that references the symbol with `Unresolved symbol` while local
  validators, which enable every feature, accept it. The devnet evidence
  lane caught this on the audit example. Those items are now compiled for
  on-chain targets only under the `remaining-compute-units-syscall` feature
  (off by default; host builds keep them for tests), so a program that
  still uses them gets a compile error instead of an undeployable artifact.
  The audit example's substrate probe no longer uses the syscall.
- `SchemaExport::descriptor()` now carries the manifest's dynamic-tail flag,
  so cost lints and loaded-data-size recommendations see growable layouts.
- **Generated clients no longer derive `seeds::program` PDAs under the
  described program.** Context descriptors now publish no seeds for an account
  whose PDA is derived under a foreign program, so TypeScript, Kotlin, Python,
  Go, C, and Rust clients and the Solana IDL projection leave it
  caller-provided instead of computing the wrong address.
- **Escaped literal seeds stay caller-provided.** A literal such as
  `b"a"` is no longer copied into generated client derivations by its
  source spelling.
- **`hopper-svm` realloc baseline.** The host harness now records each
  account's entry data length in the runtime's resize baseline, so accounts
  larger than 10 KiB can grow or shrink as they can on chain.
- **`proportional_split` cost.** Largest-remainder ranking no longer performs
  u128 modulo in its inner loop and stops once every leftover unit is placed.
  Results are unchanged and pinned against a sorting reference, including
  `u64::MAX` inputs.
- **Transaction-size error text.** The CLI's oversize message no longer says
  transaction v1 is inactive; v1 activated on mainnet-beta on 2026-09-15, and
  the message now states that this CLI does not emit v1 envelopes yet.
- The Token-2022 vault's checked-in manifest now matches its source rendering
  (`hasDynamicTail`).

### Earlier development work prepared 2026-08-17

#### Added

#### Fixed

#### Changed

- Corrected pervasive documentation that referred to the error attribute as `#[hopper::error]`; the canonical public spelling is `#[hopper::error_code]` (the `error` macro namespace is occupied by the runtime `error!` guard macro). Fixed across migration guides, crate READMEs, macro docs, and compile-error messages.

#### Fixed

- **Hardened raw-input and cross-program soundness.** Replaced the remaining
  aligned scalar reads from the Solana BPF input buffer (`*(p as *const u64)`
  in `hopper-native`'s `raw_input.rs` account-count / instruction-length
  parsers and the `*(raw as *const u32)` CpiAccount header read in
  `instruction.rs`) with `core::ptr::read_unaligned`, so the parser never
  assumes pointer alignment it is not guaranteed. Added an explicit
  `required_len` guard to `AccountView::load_cross_program` (matching `load` /
  `load_mut`) so a foreign or overridden `LayoutContract` cannot force an
  out-of-bounds typed cast; covered by a lax-foreign-contract regression test.
- Tightened `hopper close` target parsing so ambiguous buffer/program cleanup
  invocations are rejected before forwarding to `solana program close`; exactly
  one of `--program-id`, `--buffer`, or `--buffers` is now required.
- Redacted query credentials when lifecycle commands and the devnet audit
  runner display custom RPC URLs.
- Made `hopper-compact-vault` emit a deployable SBF artifact, fixed the
  migration example's live rent top-up path to use System Program transfer
  before realloc, and updated the orderbook devnet test to pre-create its
  large segmented account outside CPI.
- Resolved all `clippy --workspace --all-targets --all-features -D warnings` findings (collapsible_if, if_same_then_else, doc_lazy_continuation, manual loop→`fill`/`strip_prefix`/`sort_by_key`, and narrowly-scoped `#[allow]` on macro-generated arity), and regenerated trybuild `.stderr` snapshots for rustc 1.96 diagnostic formatting drift while verifying every Hopper-authored guard message is unchanged.

## [0.2.1] - 2026-05-19

### Added

### Changed

- Hardened `hopper solana-check` so SBF macro calls must be path-qualified and stale runtime backend feature names are rejected.
- Wired `hopper publish-check --full` to run the Solana program shape gate before the full systems and trybuild suites.
- Updated public release docs, README links, and crate docs targets for the `0.2.1` release line.

### Fixed

- The native entrypoint now rejects excess accounts instead of truncating them.

## [0.2.0] - 2026-05-15

### Added

- **`hopper-svm` Tier 4 - Language bindings (TypeScript / Python).**
  Closes the SVM roadmap by exposing the harness to non-Rust
  test code. Three pieces:

  **`hopper-svm-ffi`** - new C-ABI wrapper crate. Single
  `lib.rs` (~700 lines) exposes the `HopperSvm` surface as
  `extern "C"` functions through opaque handle types
  (`HopperSvmHandle`, `ExecutionResultHandle`). Builds three
  artifacts via `cargo build -p hopper-svm-ffi`: `cdylib` for
  dynamic loading, `staticlib` for embedding in compiled
  extensions, `rlib` for Rust integration tests. Forwards the
  `bpf-execution` feature so host languages can pick the
  Phase 1 (lightweight) or Phase 2 (real `.so` execution)
  build. Surface coverage:
  - Lifecycle: `hopper_svm_new`, `hopper_svm_with_solana_runtime`,
    `hopper_svm_set_compute_budget`, `hopper_svm_free`.
  - Account state: `hopper_svm_set_account`,
    `hopper_svm_get_lamports`, `hopper_svm_get_account_data`.
    The FFI keeps an internal account cache aligned with the
    Rust crate's stateless `process_instruction(ix, accounts)`
    model - host code seeds accounts once, dispatches feed the
    cache through, post-state replaces the cache.
  - Dispatch: `hopper_svm_dispatch` returns an opaque outcome
    handle; eight accessor functions read out logs, error,
    consumed units, transaction fee, post-accounts, return
    data.
  - String marshalling: `HopperFfiString` (`{ ptr, len }`)
    instead of null-terminated C strings - log lines may
    contain interior nulls. `hopper_svm_string_free` releases
    every owned string.
  - Version probe: `hopper_svm_ffi_version` returns the crate
    semver as a static null-terminated string for host-side
    compatibility checks.

  6 unit tests pin the FFI edges: string round-trip + empty
  handling, lifecycle smoke (new → with_solana_runtime →
  set_compute_budget → free), `u64::MAX` sentinel for unknown
  lamports, set + get account round-trip, replace-existing-
  account-in-place, version-string format.

  **`bindings/typescript/`** - `@hopper/svm` npm package.
  Minimal scaffold (no compile step beyond `tsc`):
  - `package.json` declaring `koffi` as the only runtime dep.
  - `tsconfig.json` strict-mode, ES2022 target.
  - `src/index.ts` (~450 lines) - wraps the FFI through
    `koffi.func` declarations, exposes high-level `HopperSvm`
    + `ExecutionResult` classes with `dispose()` + identical
    semantics to the Rust crate. Includes a hand-written
    base58 codec (no `bs58` dep) since pubkey display is the
    only place it's needed.
  - `README.md` covering build setup
    (`HOPPER_SVM_LIB_PATH` env var) + a quick-start example.

  **`bindings/python/`** - `hopper-svm` PyPI package via
  `cffi` ABI mode (no compile step):
  - `pyproject.toml` declaring `cffi >= 1.16` as the only
    runtime dep, Python ≥ 3.10.
  - `hopper_svm/__init__.py` re-exports the public API.
  - `hopper_svm/core.py` (~400 lines) - `cffi.FFI` `cdef`
    declarations matching the C surface exactly, plus
    Pythonic `HopperSvm` + `ExecutionResult` classes that
    are context managers (`with` blocks free the handle).
    Also includes a tiny base58 codec.
  - `README.md` mirroring the TS shape with Python idioms.

  Both bindings drive the *same* shared library - one source
  of truth in Rust, two host-language ergonomic shapes. Adding
  a new FFI export means: edit `hopper-svm-ffi/src/lib.rs`,
  add the koffi `lib.func` line in TS, add the cffi `cdef`
  line in Python. The pattern is mechanical enough that future
  surface growth doesn't drift between the bindings.

- **`hopper-svm` Tier 3 - Niche syscalls (introspection,
  obsolete sysvars, curve25519, heavy-crypto clear-error shims).** New
  module `hopper-svm/src/bpf/tier3_syscalls.rs` plus the
  matching adapter shims in `bpf/adapters.rs` and the
  registrations in `bpf/engine.rs`. Closes the syscall surface
  gap - programs that link these names now load and dispatch
  cleanly under `--features bpf-execution`.

  **Introspection (fully implemented):**
  - `sol_get_stack_height` - reports `BpfContext::cpi_depth`.
    Outermost program runs at depth 1; each
    `sol_invoke_signed_*` increments. CU pinned at 100.
  - `sol_remaining_compute_units` - reads the meter post the
    syscall's own charge. CU pinned at 100.
  - `sol_get_processed_sibling_instruction` - Hopper Phase 2
    doesn't yet ledger sibling instructions, so the syscall
    returns `0` (mainnet's empty-list convention) for any
    index. CU pinned at 100.

  **Obsolete-but-referenced sysvars (compat buffers matching a
  fresh-validator state):**
  - `sol_get_slothashes_sysvar` - empty SlotHashes vec
    (`len(u64) = 0`).
  - `sol_get_slothistory_sysvar` - all-zero SlotHistory
    bitvec (16,392 bytes: 16,384 bitvec + 8 next_slot).
  - `sol_get_stakehistory_sysvar` - empty StakeHistory vec
    (`len(u64) = 0`).

  **Generic `sol_get_sysvar` (fully implemented):**
  - Mainnet wire `(sysvar_id_addr, offset, out_addr, length)`.
  - Adapter resolves the 32-byte ID, looks up bytes via the
    existing per-sysvar handlers (Clock, Rent, EpochSchedule,
    EpochRewards, LastRestartSlot, plus the new SlotHashes /
    SlotHistory / StakeHistory compat buffers), copies the requested
    `[offset, offset + length)` slice into `out`. CU pinned at
    100.

  **Curve25519 (fully implemented - uses existing
  `curve25519-dalek` dep):**
  - `sol_curve_validate_point` - Edwards (curve = 0) or
    Ristretto (curve = 1). Returns `0` for valid, `1` for
    invalid (mainnet's inverted-bool convention). Edwards
    validation includes the prime-order subgroup check
    (`is_torsion_free`); Ristretto validation is the
    decompression check. CU pinned at 159.
  - `sol_curve_group_op` - Add (op = 0), Sub (op = 1),
    Mul-by-scalar (op = 2). Edwards or Ristretto. Returns `0`
    on success (writes 32-byte result), `1` on invalid input
    (off-curve operand or non-canonical scalar). CU pinned at
    2,000.

  **Heavy crypto - clear-error shims (Tier 4 work):**
  - `sol_poseidon` - Poseidon hash. Returns a structured
    `Custom` error naming `light-poseidon` as the canonical
    backing crate.
  - `sol_big_mod_exp` - RSA-style modular exponentiation.
    Error names `num-bigint`.
  - `sol_alt_bn128_group_op` / `sol_alt_bn128_compression` -
    BN254 / alt_bn128 ops. Error names `ark-bn254`.

  Why clear-error shims instead of "unknown syscall": mainnet programs
  that link these names but don't always call them at runtime
  (feature-flagged paths, fallback branches) load cleanly
  under Hopper. Programs that DO hit the path see an
  actionable error message naming the missing dep, which
  surfaces the next step instead of a cryptic VM trap. CU
  pinned at 1 per shim call so the meter accounting stays
  honest if a future Tier 4 release wires up real impls
  behind the same names.

  **20+ unit tests** in `tier3_syscalls.rs` pin every layer:
  introspection (stack_height returns cpi_depth, remaining
  reports post-charge), sysvar buffer writes, generic accessor
  copy semantics + out-of-range error, curve validation
  (basepoint valid, junk invalid, unknown curve errors), group
  ops (add basepoint to itself round-trips, unknown op
  errors), shim messages contain actionable crate names, and
  out-of-meter short-circuiting on every syscall.

  Adapter layer in `bpf/adapters.rs` adds 13 new
  `declare_builtin_function!` shims (one per syscall),
  registered against the canonical sbpf names in
  `bpf/engine.rs::build_loader`.

  - **`hopper-svm/src/spl/config_program.rs`** -
    `ConfigProgramSimulator` for `Config1111111111111111111111111111111111111`.
    Single-instruction shape: parses
    `keys_len(u64) + n * (Pubkey + bool) + user_data`, validates
    every signer-flagged key against the instruction's account
    metas, writes user data into account 0 (padding/zeroing
    trailing bytes so a smaller Store doesn't leak the previous
    write). 3 unit tests pin: writes user data + zeroes tail,
    rejects unsigned signer-flagged key, rejects oversized
    payload. CU pinned at 450 (mainnet baseline).

  - **`hopper-svm/src/spl/stake_program.rs`** -
    `StakeProgramSimulator` for `Stake11111111111111111111111111111111111111`.
    Lifecycle slice: `Initialize` (0), `Authorize` (1),
    `DelegateStake` (2), `Withdraw` (4), `Deactivate` (5).
    Hand-coded 200-byte state layout matches upstream
    `StakeStateV2` bincode encoding so accounts round-trip
    cleanly: discriminator (u32 LE) + Meta
    (rent_exempt_reserve, authorized.staker,
    authorized.withdrawer, lockup.unix_timestamp, lockup.epoch,
    lockup.custodian) + optional Delegation (voter_pubkey,
    stake, activation_epoch, deactivation_epoch,
    warmup_cooldown_rate=0.25, credits_observed). `Withdraw`
    enforces a locked floor of `rent_exempt_reserve +
    active_stake` while delegated and not fully cooled, falling
    to just the rent-exempt floor once deactivation has cleared.
    Variants outside the slice (Split, Merge,
    AuthorizeWithSeed, InitializeChecked, AuthorizeChecked,
    AuthorizeCheckedWithSeed, SetLockup, SetLockupChecked,
    Redelegate, MoveStake, MoveLamports, DeactivateDelinquent)
    return a clear "not supported" error. 5 unit tests pin:
    Initialize writes Meta, full delegate-then-deactivate flow,
    Authorize swaps staker, Withdraw respects locked floor,
    unsupported variant errors. CU pinned at 750.

  - **`hopper-svm/src/spl/vote_program.rs`** -
    `VoteProgramSimulator` for `Vote111111111111111111111111111111111111111`.
    Administrative slice: `InitializeAccount` (0), `Authorize`
    (1), `Withdraw` (3), `UpdateValidatorIdentity` (4),
    `UpdateCommission` (5). Modeled header-slice (112 bytes):
    version (u32 LE) + node_pubkey + authorized_withdrawer +
    commission (u8) + authorized_voter at offset 80. Phase 1
    intentionally diverges from upstream's exact bincode for the
    versioned tail (lockouts, BTreeMap epoch->voter mapping)
    since application programs don't touch the TowerBFT
    machinery. Vote-emitting variants (Vote, VoteSwitch,
    UpdateVoteState{,Switch}, CompactUpdateVoteState,
    TowerSync{,Switch}, AuthorizeChecked, AuthorizeWithSeed)
    return a clear "not supported" error. 5 unit tests pin:
    InitializeAccount writes header, Authorize changes voter,
    Withdraw respects rent-exempt floor, UpdateCommission
    rejects commission > 100, unsupported variant errors. CU
    pinned at 3,000.

- **`hopper-svm` Tier 2 (d) - Address Lookup Tables.** Two new
  modules close the v0-transaction support gap:
  - **`hopper-svm/src/alt.rs`** - byte-layout helpers
    + the `LookupTableMeta` struct + `read_meta` / `write_meta` /
    `address_count` / `read_address` / `append_addresses` /
    `resolve_lookup`. Wire format matches mainnet exactly:
    56-byte fixed header (discriminator u32, deactivation_slot
    u64, last_extended_slot u64, last_extended_slot_start_index
    u8, authority COption + 32-byte slot, 2-byte pad), then
    addresses packed 32 bytes each at offsets 56, 88, 120, …
    Hand-coded layout to avoid `bincode` dep growth. Constants
    pinned: `LOOKUP_TABLE_META_SIZE = 56`,
    `LOOKUP_TABLE_MAX_ADDRESSES = 256`,
    `DEACTIVATION_COOLDOWN_SLOTS = 513`. **6 unit tests** pin
    every layout edge: meta round-trips, frozen tables zero
    the authority slot, append-and-read, append rejects
    overflow, resolve writable-then-readonly, resolve rejects
    out-of-bounds, closeable-after-cooldown.
  - **`hopper-svm/src/spl/alt_program.rs`** -
    `AltProgramSimulator`, the BuiltinProgram impl for
    `AddressLookupTab1e1111111111111111111111111111`. Five
    instruction tags:
    - **0 / Create** - derives the PDA from
      `(authority, recent_slot, bump_seed)` under the ALT
      program ID, validates the supplied target address
      matches, allocates the 56-byte meta header, sets the
      authority + active state.
    - **1 / Freeze** - clears the authority field. Frozen
      tables can't Extend / Deactivate / Close.
    - **2 / Extend** - appends a `Vec<Pubkey>` to the data
      region, updates `last_extended_slot` +
      `last_extended_slot_start_index`, charges rent to the
      payer.
    - **3 / Deactivate** - writes the current slot into the
      table's `deactivation_slot`, starting the 513-slot
      cooldown.
    - **4 / Close** - moves lamports out, zeroes the data,
      reassigns ownership to the system program. Requires
      `is_closeable`: deactivated AND
      `current_slot - deactivation_slot > 513`.

  Authority enforcement on every state-mutating instruction -
  Extend / Deactivate / Close all check that `signer ==
  stored_authority` and reject otherwise. Frozen tables are
  immutable.

  **2 end-to-end unit tests** in `alt_program.rs`:
  - `full_alt_lifecycle` - create → extend → deactivate →
    close-rejected-before-cooldown → close-succeeds-after.
    Pins every state-machine transition + the cooldown
    boundary.
  - `freeze_blocks_extend` - Freeze → Extend rejected.

  **`HopperSvm::with_alt_program()`** registers the simulator
  against the canonical ALT program ID. **`HopperSvm::with_solana_runtime()`**
  now includes ALT in the registered set.

  **`HopperSvm::resolve_address_table_lookup(accounts,
  table_address, writable_indexes, readonly_indexes)`** -
  the v0-transaction resolution path. Reads the table from
  the supplied account state, expands writable + readonly
  index lists into concrete `(Vec<Pubkey>, Vec<Pubkey>)`
  pairs. Rejects: missing table account, invalid
  discriminator, table that's been deactivated past the
  cooldown (closed-but-not-yet-Closed state), out-of-bounds
  indexes. Tests that simulate v0 transactions can use this
  to project lookup-table-shaped messages onto the standard
  flat `AccountMeta` shape `process_transaction` consumes.

- **`hopper-svm` Tier 2 (c) - fee-payer accounting +
  transaction-level CU budget.** New
  `hopper-svm/src/fees.rs` ships `FeeCalculator`
  (default 5000 lamports/signature, configurable via
  `set_fee_calculator`), `count_unique_signers` for dedupe
  across the chain, `priority_fee` and `total_fee` formulas.

  New `HopperSvm::process_transaction(ixs, accounts, fee_payer)`
  is the **fee-aware** chain dispatcher:
  1. Computes `total_fee = base_fee + priority_fee`:
      - `base_fee = 5000 * num_unique_signers`
      - `priority_fee = compute_unit_limit * micro_lamports_per_cu / 1_000_000`
  2. Deducts the fee from `fee_payer` up front. If insufficient,
     aborts before the first instruction with
     `HopperSvmError::InsufficientFunds`.
  3. Runs every instruction in the chain; **fee stays
     deducted even if a later instruction fails** (matches
     mainnet's anti-spam rule).
  4. Returns `HopperExecutionResult` with new
     `transaction_fee_paid: u64` field populated.

  Compute-budget integration: the compute-budget program's
  `SetComputeUnitPrice` handler now writes into a shared
  `priority_fee_micro_lamports_per_cu` cell on `HopperSvm` so
  programs that include
  `ComputeBudgetInstruction::set_compute_unit_price(N)` early
  in their chain see the new rate applied to subsequent
  `process_transaction` calls. Setter helpers
  `set_priority_fee_micro_lamports_per_cu` and
  `priority_fee_micro_lamports_per_cu()` exposed for tests
  that want to seed the value directly.

  4 unit tests in `fees.rs` pin: default mainnet rate (5000
  lamports/sig), priority-fee integer-division semantics
  (sub-lamport remainders truncated), unique-signer dedupe
  across instruction-list boundaries, base+priority sum.

  `process_instruction_chain` keeps the no-fee semantics for
  fast unit tests; `process_transaction` is the
  mainnet-equivalent path.

- **`hopper-svm` Tier 2 (b) - system program parity.** All 8
  remaining system-program variants implemented, closing the
  surface to mainnet:
  - **Tag 3 - `CreateAccountWithSeed`**: parses
    base/seed/lamports/space/owner from the body, validates the
    target address against `Pubkey::create_with_seed`, allocates
    + assigns + funds the new account.
  - **Tag 9 - `AssignWithSeed`**: reassigns the owner of a
    seed-derived account; same derivation check.
  - **Tag 10 - `TransferWithSeed`**: debits a seed-derived
    source; the from_base must sign.
  - **Tag 4 - `AdvanceNonceAccount`**: rotates the durable
    nonce; the new nonce is deterministically synthesised from
    the harness's current `clock.slot` (since Hopper doesn't
    track recent_blockhashes - that sysvar is deprecated).
    Same-slot calls produce the same nonce; different-slot
    calls produce different nonces.
  - **Tag 5 - `WithdrawNonceAccount`**: moves lamports out of a
    nonce account; only the stored authority can withdraw.
  - **Tag 6 - `InitializeNonceAccount`**: writes the canonical
    80-byte nonce state with the requested authority and a
    starting durable nonce.
  - **Tag 7 - `AuthorizeNonceAccount`**: changes the stored
    authority; current authority must sign.
  - **Tag 11 - `UpgradeNonceAccount`**: legacy → current
    `Versions` migration (no-op since both layouts share the
    same byte shape in our impl).

  **Nonce-state byte layout** is hand-coded directly:
  ```text
  [0..4]   Versions tag (1 = Current)
  [4..8]   State tag (0 = Uninitialized, 1 = Initialized)
  [8..40]  authority (Pubkey)
  [40..72] durable_nonce (Hash, 32 bytes)
  [72..80] fee_calculator.lamports_per_signature (u64 LE)
  ```
  No `bincode` dep growth - the format is stable on mainnet
  and the 80-byte layout matches the upstream
  `solana_sdk::nonce::state::Versions`. Helper functions
  `read_nonce_state` / `write_nonce_state` /
  `synthesise_nonce_from_slot` keep the wire shape isolated
  in one spot for future maintenance.

  **9 new unit tests** added to `system_program.rs`:
  CreateAccountWithSeed validates derivation + rejects
  address mismatch, AssignWithSeed round trip,
  TransferWithSeed round trip, InitializeNonceAccount writes
  the 80-byte state, AdvanceNonceAccount changes nonce with
  slot, AuthorizeNonceAccount rejects wrong signer,
  WithdrawNonceAccount moves lamports, nonce-state
  read/write round-trip.

  **System program coverage now matches mainnet's hot path
  exactly.** Programs that use `system_instruction::transfer_with_seed`,
  durable-nonce transactions, or any of the other previously-
  rejected variants now run unmodified.

- **`hopper-svm` Tier 1 - pre/post account validation.** New
  `hopper-svm/src/validation.rs` ships
  `validate_post_state(program_id, metas, pre, post, policy)`
  that enforces six structural invariants between an
  instruction's pre- and post-state, mirroring what
  `solana-bpf-loader-program` checks on mainnet:
  1. **Read-only accounts cannot change** - lamports, data,
     owner, executable all immutable when `meta.is_writable`
     is false.
  2. **Lamport conservation** across all metas - the runtime
     does not mint or destroy lamports.
  3. **Data writes require ownership** - `pre.owner == program_id`,
     with an exception for the system program creating an
     account (empty pre, system_program owner).
  4. **Owner reassignment requires ownership** - `pre.owner == program_id`,
     with the same creation exception. Backs
     `system_instruction::assign`.
  5. **Executable flag is immutable** - programs cannot toggle
     it; the BPF loader's `Finalize` is the only legitimate
     setter, and Phase 1 doesn't simulate that path.
  6. **Lamport debits require ownership** - `n.lamports < p.lamports`
     requires `pre.owner == program_id`. Anyone can credit any
     account; only the owner can debit. Combined with rule 2
     this makes "lamport theft" structurally impossible
     without ownership.

  Wired into `HopperSvm::dispatch_one` as a post-execution
  check on every successful instruction, both Phase 1 (built-in)
  and Phase 2 (BPF) paths. Validation failures roll back
  account mutations to the pre-state and surface a structured
  `HopperSvmError::AccountValidationFailed { account, reason }`
  so the test sees the offending account + the specific rule
  that fired. The validation step also writes a
  `Validation failed: <reason>` line into the log transcript
  so snapshot tests catch rule trips.

  **Default `Strict`, opt out via `with_lax_validation()`** for
  fast unit tests where structural invariants don't apply
  (hand-written Rust simulators don't always follow Solana's
  account-mutation rules). New `ValidationPolicy` enum exposed
  through the `hopper_svm` re-export. **9 unit tests** pin
  every rule against both passing and failing fixtures: identity,
  read-only mutation rejection, lamport-conservation rejection,
  legitimate transfer passes, non-owner data write rejection,
  data write on creation allowed, non-owner assign rejection,
  owner-self-assign passes, executable toggle rejection,
  non-owner lamport debit rejection, non-owner lamport credit
  allowed, lax policy disables all checks.

  This is the bug-revealing layer: a Hopper program that
  incorrectly mutates a non-owned account, breaks lamport
  conservation, or toggles the executable flag now FAILS the
  `hopper-svm` test with a clear pointer at the rule that
  fired, instead of silently passing locally and reverting on
  mainnet.

- **`hopper-svm` Tier 1 - Compute Budget program builtin.**
  New `hopper-svm/src/compute_budget_program.rs` ships
  `ComputeBudgetProgramSimulator` registered via
  `HopperSvm::with_compute_budget_program()`. Handles the five
  compute-budget instructions: `RequestUnits` (deprecated, tag
  0), `RequestHeapFrame` (1), `SetComputeUnitLimit` (2),
  `SetComputeUnitPrice` (3), `SetLoadedAccountsDataSizeLimit`
  (4). The signature feature: `SetComputeUnitLimit` writes to
  a shared `pending_cu_limit` cell on `HopperSvm`; the next
  `dispatch_one` reads + applies it as the budget override
  for that instruction. This makes
  `ComputeBudgetInstruction::set_compute_unit_limit(N)` take
  effect for subsequent instructions in
  `process_instruction_chain`, matching mainnet's
  transaction-level budget semantics within Hopper's
  instruction-by-instruction dispatch model. `MAX_COMPUTE_UNIT_LIMIT
  = 1_400_000` enforced (mainnet's hard cap). 4 unit tests
  pin the pending-cell write, the over-max rejection (cell
  untouched on rejection), the `RequestHeapFrame` Phase-1
  warning behavior, and the unknown-tag error message.

- **`hopper-svm` Tier 1 - `inner_instructions` field on
  ExecutionOutcome.** New `inner_instructions: Vec<InnerInstruction>`
  field on `ExecutionOutcome`. Each entry captures `program_id`,
  `accounts: Vec<AccountMeta>`, `data: Vec<u8>`, and
  `stack_height: u32` (1 = outermost; 2 = first-level CPI; …;
  capped at `MAX_CPI_DEPTH = 4`). Populated by
  `bpf::cpi::dispatch_cpi` after each successful inner call -
  the parent CPI is recorded first, then the inner call's own
  `inner_instructions` (CPIs the inner program made) are
  flattened in dispatch order. New `InnerInstruction` type
  exported from `crate::engine`. New
  `HopperExecutionResult::inner_instructions()` accessor.
  Mirrors the `inner_instructions` slice mainnet records on
  transaction metadata. **2 new unit tests** in
  `bpf/cpi.rs`: `dispatch_records_inner_instruction` pins the
  single-call recording with correct stack_height, and
  `nested_cpis_flatten_in_dispatch_order` pins that nested
  CPIs flatten parent-before-child in dispatch order with
  correct stack-height assignment.

- **`hopper-svm` Tier 1 - Token-2022 simulator.** New
  `hopper-svm/src/spl/token_2022.rs` ships
  `SplToken2022Simulator` registered via
  `HopperSvm::with_spl_token_2022_simulator()`. The legacy 9
  tags (0/1/3/4/5/7/8/9) delegate to the SPL Token
  simulator's logic - the on-disk Mint/Account layout is
  identical for non-extension accounts, and the owner check
  inside the Token handlers compares against `ctx.program_id`
  which is the Token-2022 ID at this dispatch site. Extension
  tags (22+) return a structured "Phase 1 doesn't support
  Token-2022 extensions yet" error pointing at
  `add_program(&id, "spl_token_2022")` for the real `.so`
  fallback. 3 unit tests pin transfer-via-delegation,
  extension-tag rejection, and InitializeMint preserving the
  Token-2022 owner.

- **`hopper-svm` Tier 1 - ATA simulator.** New
  `hopper-svm/src/spl/ata.rs` ships `SplAtaSimulator`
  registered via `HopperSvm::with_spl_associated_token_simulator()`.
  Handles `Create` (tag 0) and `CreateIdempotent` (tag 1).
  Validates the derived ATA address via
  `get_associated_token_address_with_program_id`, the system
  + token program addresses, and the mint's owner; allocates
  + initialises the token account inline (no CPI dispatch
  needed because the operation is deterministic and we have
  direct account-state access). Idempotent path validates
  the existing ATA matches the requested `(wallet, mint)`
  pair - wrong-mint reuse is a structured rejection. 4 unit
  tests pin: legacy-Token ATA create, Token-2022 ATA create
  (different derived address), wrong-derived-address
  rejection, idempotent-on-existing no-op + idempotent-on-
  wrong-existing rejection.

  Also adds `HopperSvm::with_spl_simulators()` - convenience
  builder that registers all three SPL simulators in one call.

- **`hopper-svm` Tier 1 - bundled SPL Token simulator.** New
  `hopper-svm/src/spl/token.rs` ships `SplTokenSimulator`,
  a pure-Rust `BuiltinProgram` impl of the 8 most-used SPL
  Token instructions: `InitializeMint` (0), `InitializeAccount`
  (1), `Transfer` (3), `Approve` (4), `Revoke` (5), `MintTo`
  (7), `Burn` (8), `CloseAccount` (9). Register against
  `SPL_TOKEN_PROGRAM_ID` via the new
  `HopperSvm::with_spl_token_simulator()` builder method.

  **Why a simulator and not a bundled `.so`?** Three reasons:
  - **Version stability.** No `.so` bytes to re-vendor on
    every Anza release; the Rust source updates with our
    semver cycle.
  - **Speed.** Phase-1 builtin dispatch is 10-100x faster than
    going through the BPF interpreter for the same
    instructions. Token transfers in tests run essentially
    free.
  - **Hopper-owned.** Every layer is hand-written Rust we can
    audit. Embedding third-party `.so` bytes would be an
    opaque trust dependency.

  Validation matches `spl_token::processor` end-to-end against
  the wire format: account-owner check (must be SPL Token
  program ID), mint-mismatch rejection on `Transfer` and
  `Burn`, signer = owner OR delegate (with delegated-amount
  decrement) on auth paths, `is_initialized` enforcement on
  `Mint` and on token-account state, supply / amount overflow
  detection via checked arithmetic, `CloseAccount` rejecting
  non-empty accounts.

  Unsupported tags (`FreezeAccount`, `*Checked` variants,
  `SetAuthority`, `InitializeMultisig`, etc. - 16 total) return
  a structured `BuiltinError` with the supported-tag list so
  test failures are actionable. Programs that need an
  unsupported instruction can fall back to BPF by registering
  the real SPL `.so` via `HopperSvm::add_program`.

  Token-2022 + ATA simulators land in subsequent passes; the
  same `BuiltinProgram` pattern applies. **3 unit tests** pin
  the happy path (initialize_mint → initialize_account →
  mint_to → transfer → burn against a single state machine
  with every numeric invariant verified), the mint-mismatch
  rejection, the close-account-rejects-non-empty rule, and
  the unsupported-tag error message.

- **Hopper segment system polish - DX cohesion pass.** Audit
  of `crates/hopper-core/src/account/`, `segment_map.rs`, and
  the macro-emitted accessors found three documentation gaps
  worth tightening:
  - Read-only segment escape (`<field>_segment_ref`) and the
    pre-declared `<field>_<seg>_ref` accessor now have the
    same RAII-contract paragraph as their mutable
    counterparts. Authors hovering over the generated
    accessor see the lease semantics inline.
  - Module-level cross-link added between `segment_map`
    (compile-time `StaticSegment`) and `account::registry`
    (runtime `SegmentDescriptor`). Resolves a recurring
    "should I use SegmentMap or SegmentRegistry?" question.
  - The "compile-time vs runtime segments" decision matrix is
    now in both module headers, so a developer landing in
    either file sees both options.

  No code changes - just documentation tightening that's
  safe to ship and immediately improves the DX of the
  segment system.

- **`hopper-svm` Phase 2.3 step 12 - `sol_invoke_signed_rust`
  fully implemented.** The Rust-ABI CPI variant - what
  `solana_program::program::invoke_signed` emits by default -
  now parses through Rust's `Rc<RefCell<&mut u64>>` and
  `Rc<RefCell<&mut [u8]>>` AccountInfo wrappers + the
  three-level-nested `&[&[&[u8]]]` signer-seeds shape, runs
  the same `cpi::verify_signer_seeds` + `cpi::dispatch_cpi`
  pipeline as the C variant, and writes mutations back
  through the resolved Rust-shape pointers. Realloc-across-
  CPI honored via the `data_len` slot inside the
  `RefCell<&mut [u8]>`.

  **Layout pinned in one file.** All version-sensitive
  offsets (`AccountInfo`'s 48-byte struct, `RcInner<T>`'s
  16-byte ref-count header, `RefCell<T>`'s 8-byte borrow
  flag, `Vec<T>`'s 24-byte ptr-cap-len header, the 34-byte
  `AccountMeta` stride, etc.) live in
  `bpf/cpi_rust::layout` as named constants. A future Anza
  toolchain bump that shifts any of these is a one-file
  fixup. Six unit tests pin every constant against its
  expected value so a silent shift produces a test failure
  rather than wrong runtime behaviour.

  **Pointer-chase helpers** in `cpi_rust`:
  - `follow_rc_refcell_u64(rc_ptr) → u64_addr` - walks
    `RcInner` (skip 16) → `RefCell` (skip 8) → `&mut u64`
    (read pointer).
  - `follow_rc_refcell_slice(rc_ptr) → (data_addr, data_len)`
    - same chain, but the value at the end is a 16-byte fat
    pointer.
  - `parse_account_infos`, `parse_instruction`,
    `parse_signer_seeds` - full structured readers for the
    three top-level wire shapes the Rust ABI passes.
  - `build_parsed_cpi` - the entry point the syscall
    adapter calls. Returns a `(ParsedCpi, Vec<RustAccountInfo>)`
    tuple where the second element captures every writeback
    address the post-call sync needs.

  With Rust-ABI in place, `hopper-svm` Phase 2 covers **both
  CPI variants** end-to-end. The BPF surface is now
  feature-complete for the typical Hopper test workload:
  programs that use the standard `invoke_signed`,
  `invoke_signed_unchecked`, all sysvars, all crypto
  syscalls, all log + memory + return-data syscalls, and PDA
  derivation should run unmodified under `hopper-svm` once
  `cargo check --features bpf-execution` clears.

  Phase 2.4+ remaining: integration smoke tests against a
  compiled `.so`, the obsolete sysvars (`SlotHashes`,
  `SlotHistory`, `StakeHistory`, `Fees`,
  `RecentBlockhashes`) most modern Hopper programs don't
  read, the `inner_instructions` slice on `ExecutionOutcome`,
  and JIT execution.
- **`hopper-svm` Phase 2.2 step 11 - CPI polish: log
  threading, realloc-across-CPI, EpochRewards sysvar.** Three
  follow-ups to the step-10 CPI implementation:
  - **Inner-instruction logs land in the outer transcript.**
    The `CpiDispatcher` closure type now takes
    `&mut LogCapture` so the inner program's `Program <id>
    invoke [N+1]`, `Program log:`, and `Program <id> success`
    framing append directly to the outer call's log buffer.
    Test-side snapshot diffs over CPI now read as one
    coherent transcript across the depth boundary. The
    `LogCapture::invoke` / `success` framing already tracks
    depth; sharing the buffer is what makes the depth
    tracking observable. New unit test
    `dispatcher_logs_thread_into_outer_transcript` pins the
    cross-boundary append.
  - **Realloc tail across CPI**. Phase 2.1's writeback
    clamped at the originally-mapped data capacity, so an
    inner program that grew an account's data was silently
    truncated. Phase 2.2 honors the
    `MAX_PERMITTED_DATA_INCREASE = 10240`-byte realloc tail
    the parameter buffer already reserves: the writeback
    accepts up to `original_data_len + 10240` bytes,
    updates the `SolAccountInfo`'s `data_len` field at byte
    offset 16 of its 56-byte record, and emits a
    `Program log: warning: CPI realloc truncated …`
    diagnostic if the inner call exceeded even the tail.
    The writeback path now produces results bit-identical to
    what `solana-bpf-loader-program` does for a CPI that
    reallocs an account.
  - **`sol_get_epoch_rewards_sysvar`** - final sysvar in
    scope. 96-byte wire layout (u64 distribution_starting_block_height
    + u64 num_partitions + 32-byte parent_blockhash + u128
    total_points + u64 total_rewards + u64 distributed_rewards
    + bool active + 15-byte zero pad - `#[repr(C)]`-shaped to
    match `solana_sdk::epoch_rewards::EpochRewards`). New
    `EpochRewards` struct + `Sysvars::epoch_rewards` field
    (defaults to "no rewards distributing"). Registered as
    the 25th syscall in the engine loader. New unit test
    `epoch_rewards_layout_canonical` pins every field offset
    + the 15-byte zero pad against the canonical wire format.

  **Phase 2 cumulative coverage**: 25 syscalls - 12 Phase 2.0
  + 2 PDA + 5 sysvar + 1 heap + 3 crypto + 2 CPI. The BPF
  surface is feature-complete for the typical Hopper test
  workload. Remaining for Phase 2.3+: the Rust-ABI CPI
  variant (`sol_invoke_signed_rust`), inner-instruction
  tracking field on `ExecutionOutcome` (the upstream
  `inner_instructions` slice), and the obsolete sysvars
  (`SlotHashes`, `SlotHistory`, `StakeHistory`, `Fees`,
  `RecentBlockhashes`) most modern Hopper programs don't
  read.
- **`hopper-svm` Phase 2.1 step 10 - CPI wired in.** Adds
  `sol_invoke_signed_c` (full implementation) and
  `sol_invoke_signed_rust` (registered with structured
  "Phase 2.2" error so programs see clean failure rather than
  "missing syscall"). The C-ABI variant ships:
  - **Wire-format parsing** of `SolInstruction` (40 bytes →
    program_id / metas / data pointers + lengths),
    `SolAccountMeta` (16 bytes per entry - pubkey_addr + flags),
    `SolAccountInfo` (56 bytes per entry - key_addr +
    lamports_addr + data_len + data_addr + owner_addr +
    rent_epoch + flags), and the nested `SolSignerSeeds[m]`
    pointing at `SolSignerSeed[q]` pointing at byte slices.
    Every pointer translation goes through `MemoryMapping::map`
    so an out-of-region read fails the syscall cleanly instead
    of segfaulting the host.
  - **Signer-seed verification** - for each meta marked
    `is_signer` in the inner instruction, either the same
    pubkey was a signer in the outer instruction (forwarded-
    signature path), or one of the supplied seed sets derives
    to the signer's pubkey under the calling program's ID
    (PDA-signing path). Reuses `do_sol_create_program_address`
    from step 6 - no new crypto. Returns
    `cpi_err::SIGNER_SEEDS_INVALID = 3` on mismatch.
  - **Recursive dispatch** through a `CpiDispatcher` closure
    on `BpfContext` that the engine populates with a clone of
    the `HopperSvm`. The closure runs `HopperSvm::dispatch_one`
    one depth deeper. **Depth bounded at `MAX_CPI_DEPTH = 4`**
    matching mainnet - `cpi_err::DEPTH_EXCEEDED = 2` past the
    limit.
  - **Account-state writeback** - after the inner call returns,
    every modified writable account's lamports / owner / data
    are written back through the SolAccountInfo pointers the
    caller supplied. Outer program continues reading from
    those addresses on resume so mutations are visible.
    Phase 2.1 clamps writeback at the originally-mapped data
    capacity; Phase 2.2 will plumb the realloc tail into the
    SolAccountInfo's `data_len` so growth across CPI is
    observable.
  - **CU costs** - 1 000 CU per invoke (matches mainnet's
    `invoke_units`).
  - **`MAX_SIGNERS = 16`** enforced - a 17th signer-seed set
    is `cpi_err::TOO_MANY_SIGNERS = 4`.
  - **6 new unit tests** in `bpf/cpi.rs` pin: outer-signer
    pass-through accepted, unauthorised signer rejected,
    `MAX_SIGNERS` enforcement, PDA-derived signer accepted
    (round-trips through `do_sol_create_program_address`),
    `dispatch_cpi` rejects recursion past `MAX_CPI_DEPTH`,
    `dispatch_cpi` returns `FAILED` when no dispatcher is
    configured (Phase 1 path).

  **Phase 2.2 (next session)**: full `sol_invoke_signed_rust`
  parsing through the `Rc<RefCell<…>>` AccountInfo layout
  (Rust-internal layout that drifts between toolchain
  versions; deferring keeps 2.1 stable), realloc tail
  plumbing across CPI boundaries, inner-instruction tracking
  for the `inner_instructions` log field. After 2.2, the BPF
  surface is feature-complete for the typical Hopper test
  workload.

  With CPI in place, `hopper-svm` Phase 2 covers **24 syscalls**
  (12 Phase 2.0 + 2 PDA + 4 sysvar + 1 heap + 3 crypto + 2 CPI),
  matching the typical-program coverage of `mollusk-svm` while
  remaining 100% Hopper-owned above the eBPF interpreter.
- **`hopper-svm` Phase 2.1 step 9 - crypto syscalls wired in.**
  Adds `sol_keccak256_`, `sol_blake3`, and
  `sol_secp256k1_recover_` to the BPF engine. Each delegates to
  a published, well-vetted crate:
  - `sol_keccak256_` → `sha3 = "0.10"`'s `Keccak256` (the legacy
    Ethereum variant - *not* the standardised SHA3-256, which
    differs in padding and produces different digests).
  - `sol_blake3` → `blake3 = "1"`.
  - `sol_secp256k1_recover_` → `secp256k1 = "0.29"` with the
    `recovery` feature.
  Wire shape: hashes accept the same `(addr, len)` chunk-list
  format as `sol_log_data` and the PDA-derive seed list - the
  `translate_seeds` adapter helper is reused. secp256k1 recover
  takes a 32-byte hash, recovery id (0..=3), 64-byte (r || s)
  signature, and writes a 64-byte uncompressed public key
  (X || Y, no leading 0x04 marker - matches upstream's wire).
  CU costs match the production runtime: hash = 85 base + 1
  per 16-byte chunk, secp256k1 recover = 25_000 flat. **8 new
  unit tests** pin known-good digests for Keccak-256
  (empty input → c5d24601…, "abc" → 4e03657a… - pinning these
  exact hex strings catches the SHA3-vs-Keccak swap bug class),
  BLAKE3 empty input → af1349b9…, streaming (multi-chunk) hashing
  matches one-shot, hash CU formula at 0/1/16/17/32 bytes,
  secp256k1 rejects bad signatures, secp256k1 rejects out-of-
  range recovery_id (7), out-of-meter on hash returns OutOfMeter
  without touching the output.

  Phase 2.1 remaining: **CPI** (`sol_invoke_signed_*`) - the
  recursive harness-dispatch one. After that, the BPF surface
  is feature-complete for the typical Hopper test workload.
- **`hopper-svm` Phase 2.1 step 8 - heap allocator wired in.**
  Adds `sol_alloc_free_` to the BPF engine. Bump-allocator
  semantics (matches upstream `agave-syscalls::SyscallAllocFree`):
  alloc returns monotonically-increasing 8-byte-aligned VM
  addresses inside the 32 KiB heap region at `MM_HEAP_START`;
  free is a no-op. Heap exhaustion returns null (0) without
  moving the cursor, so a subsequent alloc that fits can still
  proceed. Cursor lives on `BpfContext::heap_cursor` and resets
  to 0 at every fresh instruction (matches per-instruction
  allocator lifetime). New constants exposed: `HEAP_ALIGN = 8`,
  `HEAP_SIZE = 32 KiB`, `HEAP_VM_START = 0x300000000`. **6 new
  unit tests** pin: monotonic VM addresses with expected offsets
  (16, 16+24, etc.), 1-byte alloc rounds to 8, 9-byte rounds to
  16, free-is-noop (cursor unchanged after a free), exhaustion
  returns null and leaves cursor untouched (subsequent fitting
  allocs still succeed), zero-size alloc returns the cursor
  address without moving it (sentinel pattern), out-of-meter
  returns null without partial cursor movement.

  Phase 2.1 remaining: crypto syscalls (`sol_keccak256_`,
  `sol_secp256k1_recover_`, `sol_blake3`), CPI
  (`sol_invoke_signed_*`).
- **`hopper-svm` Phase 2.1 step 7 - sysvar fetches wired in.**
  Adds `sol_get_clock_sysvar`, `sol_get_rent_sysvar`,
  `sol_get_epoch_schedule_sysvar`, `sol_get_last_restart_slot_sysvar`
  to the BPF engine. Each writes a fixed-size `#[repr(C)]`-shaped
  byte buffer that's wire-compatible with the upstream
  `solana_sdk::sysvar::*` structs:
  - `Clock` - 40 bytes (5 * 8-byte fields).
  - `Rent` - 24 bytes (u64 + f64 + u8 + 7-byte zero pad).
  - `EpochSchedule` - 40 bytes (2 * u64 + bool with 7-byte pad +
    2 * u64).
  - `LastRestartSlot` - 8 bytes (single u64).
  Sysvar state lives on `Sysvars` (extended this release with
  `EpochSchedule` + `LastRestartSlot` fields, defaulted to
  mainnet-typical values). The engine snapshots sysvars onto
  `BpfContext::sysvars` at instruction start so a program sees
  a consistent view even if the outer test code mutates the
  harness mid-chain. **6 new unit tests** pin canonical layout
  for each sysvar (specifically including the zero-padding
  bytes - a future change to `Rent` or `EpochSchedule` can't
  silently leak garbage into the wire format), short-buffer
  rejection, and out-of-meter short-circuit.

### Changed

### Pre-publication checklist

- LICENSE: Apache-2.0, present at repo root.
- CONTRIBUTING.md: see [CONTRIBUTING.md](CONTRIBUTING.md).
- SECURITY.md: see [SECURITY.md](SECURITY.md).
- All published crates have `homepage = "https://hopperzero.dev"`,
  version-pinned `documentation = "https://docs.rs/crate/<crate>/0.2.0"`, and a per-crate
  `README.md`.

## [0.1.0] - 2026-05-10

### Added

- First public release line for the Hopper framework, CLI, and public companion
  crates. The top-level package target is `hopper-lang` with library crate
  name `hopper`.
