# Why Hopper

## The short version

Hopper is a policy-driven zero-copy runtime for Solana. Three things set it apart:

1. **Segment-level borrow tracking.** When one instruction mutates a vault's `balance` field, Hopper locks exactly those 8 bytes. A parallel read of `authority` on the same account? No conflict. Every other framework locks the whole account.
2. **One access model, three safety tiers.** `load::<T>()` for validated whole-layout access, `segment_ref`/`segment_mut` for field-level access, `unsafe { as_mut_ptr() }` for Pinocchio-parity raw access. Same pipeline, different guarantees.
3. **Policy-driven enforcement.** `#[hopper::program(strict)]`, `(sealed)`, or `(raw)` at the module level; `#[instruction(N, unsafe_memory, skip_token_checks)]` per handler. Every safety lever is a compile-time const the user toggles in one line.

## Where Hopper sits

| | Anchor | Quasar | Pinocchio | **Hopper** |
|---|---|---|---|---|
| Raw entrypoint ownership | no | yes | yes | **yes** |
| Zero-copy account access | `AccountLoader` | yes | yes | **yes** |
| `no_std` / `no_alloc` | no | yes | yes | **yes** |
| Segment-level borrow enforcement | no | no | no | **yes** |
| Compile-time layout fingerprints | no | no | no | **yes** |
| Versioned + foreign typed loads | no | no | no | **yes** |
| State receipts | no | no | no | **yes** |
| Policy-driven safety levers | no | no | no | **yes** |
| Selective per-instruction unsafe | no | no | no | **yes** |
| Proc macros optional (not required) | required | yes | no | **yes** |
| Compile-fail safety proofs | no | no | no | **yes** (13 fixtures) |

## The three modes you can ship

### `STRICT`

Every lever on. Typed contexts, auto-bind, constraint gauntlet, `enforce_token_checks`, `unsafe` allowed but isolated to explicit `hopper_unsafe_region!` blocks.

```rust
#[hopper::program(strict)]
pub mod vault {
    #[instruction(0)]
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
        let mut balance = ctx.vault_balance_mut()?;
        *balance = WireU64::new(balance.get().checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?);
        Ok(())
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
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult { ... }

    // Opt-in: this one handler gets raw access back.
    #[instruction(1, unsafe_memory)]
    pub fn fast_sweep(ctx: Context<Sweep>) -> ProgramResult { ... }
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

Numbers from `bench/results/framework-vaults/vault-framework-comparison.csv`, 8-seed average, Mollusk harness, identical vault contract across frameworks:

| Instruction | Hopper | Quasar | Pinocchio-style |
|---|---|---|---|
| authorize | **432 CU** | 585 | 2543 |
| counter_access | **539 CU** | 607 | 2575 |
| deposit | **1651 CU** | 1768 | 3763 |
| withdraw | **455 CU** | 605 | 2567 |
| binary size | **7.62 KiB** | 8.36 | 10.13 |

Methodology pinned in [bench/METHODOLOGY.md](../bench/METHODOLOGY.md). Re-run:

```sh
bash bench/measure.sh all
```

## Where to start

1. Read [MEMORY_ACCESS.md](MEMORY_ACCESS.md) for the access-tier doctrine.
2. Read [POLICY_GUARANTEES.md](POLICY_GUARANTEES.md) for what each lever guarantees and drops.
3. Read `examples/hopper-policy-vault/src/lib.rs` for the three modes side by side.
4. Run `cargo run -p hopper-cli -- verify --package hopper-policy-vault` to see the LAYOUT_ID fingerprint scan on a shipping `.so`.

## What Hopper doesn't promise

- Not an Anchor replacement for every workflow. Teams already on Anchor with a working IDL pipeline should weigh the migration cost against what Hopper adds.
- Not a serialization library. Hopper maps structs directly onto account bytes. If your account format uses Borsh, Hopper's zero-copy layer is not useful; stick with the Borsh pipeline.
- Not a host-side framework. The `hopper` crate is `no_std` and targets SBF. The schema / CLI / client-gen crates are host-side; programs are not.
