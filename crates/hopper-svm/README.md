# hopper-svm

Hopper-native in-process Solana execution harness with three layered execution modes. Phase 1 ships inline Rust simulators; Phase 2 is `solana-sbpf` direct interpretation; Phase 3 is the real Agave validator stack.

The harness is shaped around how Hopper programs actually want to be tested, with first-class hooks for Hopper headers, layout fingerprints, segment maps, and receipts. Hopper-aware decoders sit on every result type so layout-bearing accounts surface their identity directly.

## Phase 1 (default features) — inline simulators

A complete built-in program execution path with zero external runtime dep:

- System program flows (transfers, account creation, allocation, owner reassignment) end-to-end via Hopper's own `system_program::SystemProgram` simulator.
- User-registered simulators for any Hopper program whose business core is small enough to hand-mirror in Rust.
- Hopper-aware decoding of post-state account bytes.

Plus the full Quasar-parity verb surface:

- `simulate_instruction` / `simulate_instruction_chain` for non-mutating dry runs.
- `process_instruction_chain` for atomic multi-instruction transactions.
- `process_transaction(ixs, accounts, fee_payer)` with mainnet-style fee deduction.
- `warp_to_slot(n)` / `warp_to_timestamp(t)` clock control.
- Stateful overlay: `set_account` / `get_account` / `airdrop` / `create_account` / `set_token_balance` / `set_mint_supply` / `snapshot_accounts` / `restore_accounts` plus the `*_with_store` dispatch pair.
- Result enrichment: `assert_success` / `assert_error` / `assert_inner_instruction_count` / `compute_units_consumed` / `execution_time_us` / `inner_instructions` / `decode_header` / `hopper_accounts` / `decoded_logs`.

## Phase 2 (`bpf-execution` feature) — direct `solana-sbpf`

Real `.so` execution via [`solana-sbpf`](https://crates.io/crates/solana-sbpf), Anza's canonical eBPF interpreter. Lower-fidelity than Phase 3 because syscall semantics and CPI dispatch are reimplemented here rather than delegated to the validator stack.

API surface:

- `add_program(id, name)` / `add_program_from_bytes(id, elf)` for default V3 loader registration.
- `add_program_with_loader(id, LoaderKind::{V2, V3}, elf)` for explicit loader pinning.
- `with_program(id, elf)` / `with_program_loader(id, loader, elf)` builder verbs.
- `with_bundled_spl_token(elf)` / `with_bundled_spl_token_2022(elf)` / `with_bundled_spl_associated_token(elf)` for caller-supplied SPL ELFs (registered under V2, the mainnet-deployment loader).

## Phase 3 (`agave-runtime` feature) — real Agave validator stack

Mainnet-fidelity path. Replaces inline simulators with the same crates the validator runs:

- `solana-program-runtime` for the invoke stack
- `solana-bpf-loader-program` for ELF loading (Loader v2 + Loader-v3-Upgradeable)
- `solana-compute-budget` for `SVMTransactionExecutionBudget` / `SVMTransactionExecutionCost`
- `solana-sysvar-cache` for sysvar surfacing
- `solana-system-program` for Agave's real `system_processor::Entrypoint`

Enable with `cargo test --features agave-runtime`. Wire it into the harness:

```rust
let svm = HopperSvm::new().with_agave_runtime();
let result = svm.process_instruction(&ix, &accounts);
```

After `with_agave_runtime`, every `process_instruction` whose program ID is registered in the engine's program cache routes through `InvokeContext::process_instruction` instead of the inline registry. Behaviour matches mainnet because it IS the validator's code. The system program is installed automatically; custom builtins register through `agave::AgaveEngine::add_builtin_function(id, account_size, BuiltinFunctionWithContext)`.

End-to-end coverage in `crates/hopper-svm/src/agave/engine.rs::tests::system_transfer_through_agave_runtime` and `crates/hopper-svm/src/lib.rs::tests::process_instruction_routes_through_agave_runtime`: alice 1_000_000 → bob 250_000 transfer dispatches through Agave, balances flow back via `AccountSharedData`, the harness reports `>= 150 CU` consumed (Agave's system program declares that as its baseline).

## Usage

Add as a dev-dependency in your program's `Cargo.toml`:

```toml
[dev-dependencies]
hopper-svm = { workspace = true }
```

Then in a test:

```rust
use hopper_svm::{HopperSvm, KeyedAccount, Pubkey};
use hopper_svm::token::create_keyed_system_account;
use solana_sdk::system_instruction;

#[test]
fn alice_transfers_to_bob() {
    let alice = Pubkey::new_unique();
    let bob = Pubkey::new_unique();
    let svm = HopperSvm::new();  // system program registered by default

    let accounts = vec![
        create_keyed_system_account(&alice, 5_000_000),
        create_keyed_system_account(&bob, 0),
    ];
    let ix = system_instruction::transfer(&alice, &bob, 1_000_000);

    let result = svm.process_instruction(&ix, &accounts);
    result.assert_success();
    assert_eq!(result.account(&bob).unwrap().lamports, 1_000_000);
}
```

## API

| Method | Behavior |
|--------|----------|
| `HopperSvm::new()` | Empty harness with system program registered |
| `.with_builtin(id, program)` | Register a custom built-in for a program ID |
| `.with_sysvars(sysvars)` | Override clock/rent state |
| `.set_compute_budget(units)` | Override the per-instruction CU budget |
| `.process_instruction(&ix, &accts)` | Execute one instruction atomically |
| `.process_instruction_chain(&[ix], &accts)` | Execute many as one chain — state carries forward |

`HopperExecutionResult`:
- `assert_success()` / `assert_error_contains(needle)` / `is_success()` / `is_error()`
- `account(&pubkey)` returns the post-execution `KeyedAccount`
- `compute_units_consumed()`, `return_data()`, `resulting_accounts()`, `all_logs()`, `decoded_logs()`
- **Hopper-specific**: `decode_header(&pubkey)` returns the 16-byte Hopper account header by address; `hopper_accounts()` returns only the resulting accounts whose first 16 bytes look like a valid Hopper header.

## Built-in programs

Implement `BuiltinProgram`:

```rust
use hopper_svm::{BuiltinProgram, builtin::InvokeContext, KeyedAccount, error::HopperSvmError};

struct CounterSimulator;
impl BuiltinProgram for CounterSimulator {
    fn name(&self) -> &'static str { "counter-simulator" }
    fn invoke(
        &self,
        _data: &[u8],
        accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError> {
        ctx.log("incrementing counter");
        if accounts[0].data.len() < 8 { return Err(HopperSvmError::Custom(1)); }
        let mut value = u64::from_le_bytes(accounts[0].data[0..8].try_into().unwrap());
        value += 1;
        accounts[0].data[0..8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
}

let svm = HopperSvm::new().with_builtin(my_program_id, CounterSimulator);
```

## License

Apache-2.0
