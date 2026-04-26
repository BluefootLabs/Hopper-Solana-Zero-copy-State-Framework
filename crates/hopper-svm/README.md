# hopper-svm

Hopper-native in-process Solana execution harness. **No `mollusk-svm` dependency, no `quasar-svm` dependency, no copy of any other framework's design.**

Every layer above the eBPF interpreter — built-in program registry, syscall surface (Phase 2), CPI dispatch (Phase 2), compute metering, log buffer, sysvar state, account input/output serialization, and Hopper-aware result decoding — is implemented here from scratch. The harness is shaped around how Hopper programs actually want to be tested, with first-class hooks for Hopper headers, layout fingerprints, segment maps, and receipts.

## Phase 1 (this release)

Ships a complete, working **built-in program** execution path. Drop-in for tests that exercise:

- System program flows (transfers, account creation, allocation, reassignment of ownership) end-to-end.
- Custom built-in programs registered for unit testing — useful when a Hopper program's business core is small enough that a hand-written Rust simulator gives the same coverage as the compiled `.so`.
- Anything that needs Hopper-aware decoding of post-state account bytes.

## Phase 2 (planned)

Wires [`solana-sbpf`](https://crates.io/crates/solana-sbpf) — Anza's canonical eBPF interpreter, the foundation Mollusk and Agave both build on — as the execution engine for real `.so` files. Adds the full Solana syscall surface (`sol_log_*`, `sol_panic_`, `sol_mem*`, `sol_get_*_sysvar`, `sol_create_program_address`, `sol_invoke_signed`, …), CPI dispatch back into the harness, account input-buffer serialization, and the realloc + return-data conventions. The seam is already in place — `engine::Engine` — so Phase 2 lands as one new file plus one extra fall-through line in `HopperSvm::dispatch_one`.

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
