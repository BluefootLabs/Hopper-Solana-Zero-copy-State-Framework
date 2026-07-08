# Why Hopper

## The short version

Hopper is a policy-driven zero-copy runtime for Solana. Three things set it apart:

1. **Segment-level borrow tracking.** When one instruction mutates a vault's `balance` field, Hopper locks exactly those 8 bytes. A parallel read of `authority` on the same account? No conflict. Every other framework locks the whole account.
2. **One access model, five explicit tiers.** Generated field accessors / `segment_ref_typed` are the default hot path; `load::<T>()` is validated whole-layout access; const/dynamic segment APIs are advanced; `raw_ref` / `raw_mut` are typed escape hatches; `unsafe { as_mut_ptr() }` is full raw access. Same pipeline, different guarantees.
3. **Policy-driven enforcement.** `#[hopper::program(strict)]`, `(sealed)`, or `(raw)` at the module level; `#[instruction(N, unsafe_memory, skip_token_checks)]` per handler. Every safety lever is a compile-time const the user toggles in one line.

## Where Hopper sits

| | Anchor | Quasar | Pinocchio | **Hopper** |
|---|---|---|---|---|
| Raw entrypoint ownership | no | yes | yes | **yes** |
| Zero-copy account access | `AccountLoader` | yes | yes | **yes** |
| `no_std` / `no_alloc` | no | yes | yes | **yes** |
| Segment-level borrow enforcement | no | no | no | **yes** |
| Instruction touch maps (per-ix byte footprint) | no | no | no | **yes** (`touch-map` feature, measured 0 CU) |
| Field-level write policies (`strict_writes`) | no | no | no | **yes** (declared mut ranges enforced at borrow acquire) |
| Compile-time layout fingerprints | no | no | no | **yes** |
| Versioned + foreign typed loads | no | no | no | **yes** |
| State receipts | no | no | no | **yes** |
| Policy-driven safety levers | no | no | no | **yes** |
| Selective per-instruction unsafe | no | no | no | **yes** |
| Proc macros optional (not required) | required | yes | no | **yes** |
| Compile-fail safety proofs | no | no | no | **yes** (21 compile-fail fixtures + 2 pass guards) |

## The three modes you can ship

### `STRICT`

Every lever on. Typed contexts, auto-bind, constraint gauntlet, `enforce_token_checks`, `unsafe` allowed but isolated to explicit `hopper_unsafe_region!` blocks.

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

Pinocchio parity. Strict off, token checks off, unsafe on. Handlers take `&mut Context<'_>` directly. Author is responsible for every invariant.

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

Reach for this when every CU counts and the author has already validated the invariants by hand.

## Why this matters

Three classes of Solana exploits map directly onto the levers:

| Exploit class | Lever that closes it |
|---|---|
| Missing signer or wrong-authority token move | `enforce_token_checks = true` + `TransferChecked::invoke_strict` |
| Layout drift between on-chain program and client | `LAYOUT_ID` fingerprint enforced in `load::<T>()` + TS / Kotlin / Rust client `assertLayoutId` |
| Aliasing bug in a multi-segment write | `SegmentBorrowRegistry` rejects overlapping mutable borrows at runtime, compile-fail fixture `ref_only_rejects_raw_ref.rs` proves raw `&mut` cannot satisfy `HopperRefOnly` |

Other frameworks rely on the author to remember every check. Hopper makes the check the default and lets you opt out explicitly.

## Benchmark, not claims

Current release-facing numbers come from the sibling
[hopper-bench](https://github.com/BluefootLabs/hopper-bench) parity harness:
8-seed average, Mollusk execution, and one command line for every included
framework. The current snapshot (2026-07-07, four-way,
`hopper-bench/results/framework-vaults-2026-07-07-post-ep/`) includes Hopper,
the benchmark repo's in-tree Anza Pinocchio target, Quasar's upstream
`examples/vault` target, and a measured Anchor 0.31.1 comparator. Quasar's
upstream vault implements only `deposit` and `withdraw`, so validation-only
rows are `n/a` for Quasar.

| Instruction | Hopper | Anza Pinocchio | Quasar | Anchor 0.31.1 |
|---|---:|---:|---:|---:|
| authorize | **420 CU** | 2512 CU | n/a | 5017 CU |
| counter_access | **564 CU** | 2539 CU | n/a | 5156 CU |
| deposit | **1650 CU** | 3856 CU | 1756 CU | 7150 CU |
| withdraw | **442 CU** | 2548 CU | 592 CU | 5108 CU |
| binary size | 7.41 KiB | 7.73 KiB | **5.47 KiB** | 190.11 KiB |

An earlier version of this page published the 2026-05-25 table
(431/551/1669/453 CU). Those numbers are retired: they were measured with a
pre-gate fast-entrypoint path that only worked in local SVMs (SIMD-0321 is
not active on public clusters) and were therefore better than any mainnet
deployment could achieve. The table above is the honest, deployable number —
the full bisect story is in `BENCHMARKS.md`.

Two more measured 2026-07-07 results (provenance in `BENCHMARKS.md`):

- **The safe overlay costs what a raw pointer cast costs.** In the Mollusk
  primitive lab, Hopper's validated overlay and a raw unsafe cast both
  measure 1 CU net.
- **Router parity, first published three-way:** Hopper beats Quasar on every
  1–3-hop swap row (−18/−20/−21 CU) and runs within 2.1–2.7% of hand-written
  Pinocchio,
  with the smallest binary of the three
  (`hopper-bench/results/router-parity-2026-07-07-post-review/`).

That supports a precise claim: **Anchor/Quasar-class DX, Hopper-grade
safety/state contracts, Pinocchio-class raw control.** It does not turn one
vault benchmark into a universal "faster than Pinocchio" statement.

Methodology lives in the sibling
[hopper-bench](https://github.com/BluefootLabs/hopper-bench) product repo. Re-run
from that checkout:

```powershell
.\compare-framework-vaults.ps1 -HopperRoot ..\Hopper-Solana-Zero-copy-State-Framework -QuasarRoot <path-to-quasar> -OutDir results\framework-vaults
```

## In-process testing - `hopper-svm`

Hopper ships its own validator-class harness so tests don't need a live `solana-test-validator`. Three layered execution modes:

- **Default features** - inline Rust simulators for the system program plus user-registered builtins. Fast unit tests, no validator dep, full Quasar-parity verb surface (`simulate_instruction`, `process_instruction_chain`, `warp_to_slot` / `warp_to_timestamp`, stateful overlay with `airdrop` / `set_token_balance` / `snapshot_accounts` / `restore_accounts`).
- **`bpf-execution`** - direct `solana-sbpf` interpretation of `.so` bytes when you need real BPF execution but want the lean dep tree.
- **`agave-runtime`** - the mainnet-fidelity path. Replaces inline simulators with the actual Agave validator stack (`solana-program-runtime` + `solana-bpf-loader-program` + `solana-system-program`). After `HopperSvm::new().with_agave_runtime()`, every `process_instruction` routes through `InvokeContext::process_instruction` against Agave's program cache. Behaviour matches mainnet because it IS the validator's code.

```rust
let svm = HopperSvm::new().with_agave_runtime();
let result = svm.process_instruction(&transfer_ix, &[alice, bob]);
result.assert_success();
// Agave's system program reports its real CU baseline.
assert!(result.compute_units_consumed() >= 150);
```

The `hopper-svm` crate is the harness layer; it ships standalone so any Solana program (Hopper or otherwise) can pull it in as a dev-dependency. See the sibling [hopper-svm](https://github.com/BluefootLabs/hopper-svm) repo for the full surface and its `programs/README.md` for sourcing SPL Token / Token-2022 / ATA `.so` bytes when you need real CPI tests.

## Where to start

1. Read [MEMORY_ACCESS.md](MEMORY_ACCESS.md) for the access-tier doctrine.
2. Read [POLICY_GUARANTEES.md](POLICY_GUARANTEES.md) for what each lever guarantees and drops. For which capabilities are structural (uncopyable without forking the account type) versus table stakes, read [THE_MOAT.md](THE_MOAT.md).
3. Read `examples/hopper-policy-vault/src/lib.rs` for the three modes side by side.
4. Run `cargo run -p hopper-cli -- verify --package hopper-policy-vault` to see the LAYOUT_ID fingerprint scan on a shipping `.so`.
5. In the `hopper-svm` repo, run `cargo test --features agave-runtime process_instruction_routes_through_agave_runtime` to see the harness execute a system transfer through Agave's real runtime.

## What Hopper doesn't promise

- Not an Anchor replacement for every workflow. Teams already on Anchor with a working IDL pipeline should weigh the migration cost against what Hopper adds.
- Not a serialization library. Hopper maps structs directly onto account bytes. If your account format uses Borsh, Hopper's zero-copy layer is not useful; stick with the Borsh pipeline.
- Not a host-side framework. The `hopper` crate is `no_std` and targets SBF. The schema / CLI / client-gen crates are host-side; programs are not.
