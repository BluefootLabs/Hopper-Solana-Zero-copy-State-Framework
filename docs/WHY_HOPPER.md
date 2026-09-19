# Why Hopper

## The short version

Hopper is a policy-driven zero-copy runtime for Solana. The short boundary is:
**Solana locks accounts; Hopper governs bytes.** Three things set it apart:

1. **Segment-level borrow tracking.** Within one Hopper invocation, a write lease over a vault's `balance` bytes can coexist with a disjoint read of `authority`. This is an in-program safety and observability model; Solana's scheduler still locks the whole Pubkey.
2. **One access model, five explicit tiers.** Generated field accessors / `segment_ref_typed` are the default hot path; `load::<T>()` is validated whole-layout access; const/dynamic segment APIs are advanced; `raw_ref` / `raw_mut` are typed escape hatches; `unsafe { as_mut_ptr() }` is full raw access. Same pipeline, different guarantees.
3. **Policy-driven enforcement.** `#[hopper::program(strict)]`, `(sealed)`, or `(raw)` records the module posture; `#[instruction(N, unsafe_memory, skip_token_checks)]` records per-handler exceptions. Typed contexts and supported governed APIs perform the enforcement. Policy constants do not semantically inspect arbitrary Rust, FFI, dependencies, or direct-substrate calls.

## Where Hopper sits

This dated summary uses the source pins in
[`ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md`](ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md)
and the
[`COMPETITIVE_REFRESH_2026-09-02.md`](COMPETITIVE_REFRESH_2026-09-02.md)
correction. Anchor stable and the published Anchor v2 release candidate are
separate targets;
Quasar means its active `0.1.0-release` source line, not the older default
branch.

| Capability | Anchor 1.2.0 | Anchor v2 RC/Alpha line | Quasar 0.1 release line | Pinocchio 0.11.2 | **Hopper 0.3 workspace** |
|---|---|---|---|---|---|
| Zero-copy account access | opt-in `AccountLoader` | default mapped accounts | yes | raw substrate primitives | **yes** |
| `no_std` / no-allocation program path | no | yes | yes | yes | **yes** |
| Typed dynamic zero-copy collections | no direct mapped `Vec`/`String` | `Slab`, `PodVec` | bounded fields and migration views | bring your own | **bounded fields, `Seq`, `Slab`, and other account-byte collections** |
| Typed same/grow/shrink migration | app-authored | app-level / evolving alpha APIs | yes | bring your own | **yes, plus schema epochs, fingerprints, chains, and deposit-preserving fit shrink** |
| IDL and generated clients | mature IDL/TS ecosystem | evolving alpha toolchain | wire IDL, ABI hash, stable Rust/Kit/Web3 plus preview Python/Go/C | no framework layer | **Six SDKs + Hopper public IDL + Codama JSON + conditional Solana IDL: 9 interop formats; full manifest/lowered Rust separate** |
| Kani/Miri/fuzz workflow | ecosystem-dependent | substantial Miri/fuzz/Kani sources; Kani CI disabled at the pin | yes | substrate-specific | **yes, plus manifest-generated cases and compiled-SBF flagship lanes** |
| Manifest-linked runtime byte-write policy | no | no equivalent found | no equivalent found | no | **yes (`strict_writes`)** |
| On-chain per-instruction touch evidence | no | no equivalent found | test-SVM byte diffs, not the same contract | no | **yes, opt-in** |
| Schema fingerprint/evolution graph tied to client decoding | no comparable graph | no comparable graph at the pin | narrower ABI hash plus typed migration | no | **yes** |

This is a source-snapshot comparison, not a permanent ranking. Anchor leads in
ecosystem maturity, while Anchor v2 and Quasar contain serious zero-copy,
collection, migration, client, and verification work. Quasar leads the pinned
binary-size row and its verification lane is deeper. QEDGen/qedsvm also proves
frame conditions for constrained selected compiled paths and performs IDL
upgrade analysis; it does not provide Hopper's runtime gate or cumulative touch
evidence. See the 2026-09-06 competitive refresh for pins and boundaries.

## The three modes you can ship

### `STRICT`

Typed contexts auto-bind and run the constraint gauntlet. The module also
publishes the `enforce_token_checks` promise: authored token CPIs are expected
to use Hopper's strict helpers. The policy does not rewrite arbitrary CPI code,
so review remains required for direct or dependency CPI calls. `unsafe` is
allowed but should remain isolated to explicit reviewed regions.

```rust
#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
    pub authority: Signer<'info>,
}

impl<'info> Deposit<'info> {
    pub fn deposit(&self, amount: u64) -> ProgramResult {
        let mut vault = self.vault.get_mut()?;
        vault.balance.checked_add_assign(amount)
    }
}

#[hopper::program(strict)]
pub mod vault {
    #[instruction(0)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }
}
```

Reach for this by default.

### `SEALED`

Strict + `enforce_token_checks` + no `unsafe` anywhere. The program macro emits `#[deny(unsafe_code)]` on every handler. One handler can still opt back in via `#[instruction(N, unsafe_memory)]` for a single fast path.

```rust
#[hopper::program(sealed)]
pub mod vault {
    // Every handler here: no unsafe compiles.
    #[instruction(0)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }

    // Opt-in: this one handler gets raw access back.
    #[instruction(1, unsafe_memory)]
    pub fn fast_sweep(ctx: Ctx<Sweep>) -> ProgramResult {
        ctx.accounts.fast_sweep()
    }
}
```

Reach for this when writing code that goes to external audit.

### `RAW`

Raw-authoring posture: the module may use `&mut Context<'_>` handlers, does not
promise strict token helpers, and permits unsafe code. Typed `Ctx<T>` handlers
still bind even in a RAW module, and calls to validated Hopper accessors still
perform their documented checks. The author owns every invariant omitted by a
hand-written path.

```rust
#[hopper::program(raw)]
pub mod vault {
    #[instruction(0)]
    pub fn deposit(ctx: &mut Context<'_>, amount: u64) -> ProgramResult {
        let mut vault = ctx.load_mut::<Vault>(0)?;
        vault.balance = WireU64::new(vault.balance.get().checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?);
        Ok(())
    }
}
```

Reach for this only when the author has explicitly implemented and tested the
invariants that the chosen raw path omits.

## Why this matters

Three classes of Solana exploits map directly onto the levers:

| Exploit class | Lever that closes it |
|---|---|
| Missing signer or wrong-authority token move | `enforce_token_checks = true` + `TransferChecked::invoke_strict` |
| Layout drift between on-chain program and client | `LAYOUT_ID` fingerprint enforced in `load::<T>()` + TS / Kotlin / Rust client `assertLayoutId` |
| Aliasing bug in a multi-segment write | `SegmentBorrowRegistry` rejects overlapping mutable borrows at runtime, compile-fail fixture `ref_only_rejects_raw_ref.rs` proves raw `&mut` cannot satisfy `HopperRefOnly` |

Anchor and Quasar also generate standard signer, owner, PDA, and account
constraints. Hopper's additional claim is narrower: its typed contexts make
those checks the default, while its manifest can connect declared byte writes,
runtime gates, generated client metas, touch evidence, contention analysis,
and separately recomputed offline containment verification. Grillo is
maintained in the same workspace and does not authenticate its current
caller-supplied evidence. Raw and unchecked paths remain explicit
review boundaries rather than disappearing from the threat model.

## Benchmark, not claims

Release-facing numbers must come from the current same-behavior five-way
matrix in the sibling
[hopper-bench](https://github.com/BluefootLabs/hopper-bench) repository. Its
required targets are Hopper, Pinocchio 0.11.2, Quasar's pinned 0.1 release-line
snapshot, Anchor v2's pinned alpha snapshot, and Star Frame's pinned snapshot.
All five use the same program id, instruction contract, account state, seeds,
release profile, SBF toolchain, and Mollusk runner.

The runner hard-fails on successful-state divergence, unsigned deposit or
withdraw, wrong-PDA deposit or withdraw, a pin/lock mismatch, nonempty strict
build output, or a dirty source tree. The clean 2026-08-16 run retained the
required artifact from Hopper `8696640` and benchmark source `af5bc95`:
successful-state parity checks and all 30 rejection gates passed, with Hopper
at 1,578 deposit CU, 424 withdraw CU, and 9,032 binary bytes. The complete
five-way rows, two-way rows, toolchain, and archive SHA are bound in
[`audit/framework-matrix-2026-08-16.json`](../audit/framework-matrix-2026-08-16.json).
This closes the clean peer-benchmark evidence gap for that fixture only; it is
not a universal speed, cost, or binary-size ranking.

Historical primitive, vault, router, and public-cluster measurements remain in
`BENCHMARKS.md` with their dates and methods. They can motivate engineering
work, but they do not establish that Hopper is universally faster, cheaper, or
smaller than another framework.

## In-process testing - `hopper-svm`

The in-tree `hopper-svm` crate is a deliberately small host harness. It
fabricates aligned Hopper Native account buffers and calls the host bridge
generated by `#[hopper::program]`, preserving Hopper account-view memory,
borrow tracking, resulting bytes, owners, and lamports. It is useful for fast
framework and business-logic tests without a validator process.

```rust
let result = HopperSvm::new().process_instruction(
    program_id,
    &instruction_data,
    &[vault_fixture],
    process_instruction,
);
assert!(result.program_result.is_ok());
```

This host path does not execute an SBF ELF, reproduce validator syscalls, or
measure CU; `compute_units_consumed` is currently reported as zero. Use the
workspace's Mollusk/compiled-SBF lanes for canonical SPL CPI behavior, rollback,
and CU evidence, and a public cluster or validator-replay lane for deployment
evidence. Calling `hopper-svm` “Mainnet-fidelity” would overstate its contract.

## Where to start

1. Read [MEMORY_ACCESS.md](MEMORY_ACCESS.md) for the access-tier doctrine.
2. Read [POLICY_GUARANTEES.md](POLICY_GUARANTEES.md) for what each lever guarantees and drops. For which capabilities require an account-access architectural retrofit versus ordinary feature work, read [COMPARISON.md](../COMPARISON.md).
3. Read `examples/hopper-policy-vault/src/lib.rs` for the three modes side by side.
4. Run `cargo run -p hopper-cli -- verify --package hopper-policy-vault` to see the LAYOUT_ID fingerprint scan on a shipping `.so`.
5. Run `cargo test -p hopper-svm --locked` for the host-dispatch harness, then
   use the compiled-SBF commands in `examples/hopper-cicada/README.md` when you
   need runtime-backed CPI evidence.

## What Hopper doesn't promise

- Not an Anchor replacement for every workflow. Teams already on Anchor with a working IDL pipeline should weigh the migration cost against what Hopper adds.
- Not a serialization library. Hopper maps structs directly onto account bytes. If your account format uses Borsh, Hopper's zero-copy layer is not useful; stick with the Borsh pipeline.
- Not a host-side framework. The `hopper` crate is `no_std` and targets SBF. The schema / CLI / client-gen crates are host-side; programs are not.
