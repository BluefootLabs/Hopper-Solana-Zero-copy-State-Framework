# Inspect prior calls on chain

A settlement or treasury program may need to inspect the call that preceded it:
which program ran, which accounts it received, and what instruction data it used.
Hopper exposes processed siblings through caller-owned scratch buffers. No indexer,
service, or heap allocation is required for the read.

## Read the instruction and its account privileges

Native programs use `hopper_native::introspect`; runtime programs use
`hopper_runtime::crypto`. The caller-buffer API is available in native 0.4.3 and
runtime 0.4.4.

```rust
use hopper_native::{Address, ProgramError};
use hopper_native::introspect::{
    get_processed_instruction_into, ProcessedInstructionAccount,
};

fn require_previous_program(expected: &Address) -> Result<(), ProgramError> {
    let mut data = [0u8; 128];
    let mut accounts: [ProcessedInstructionAccount; 4] =
        core::array::from_fn(|_| ProcessedInstructionAccount::default());
    let previous = get_processed_instruction_into(0, &mut data, &mut accounts)?
        .ok_or(ProgramError::InvalidArgument)?;
    if previous.program_id != *expected {
        return Err(ProgramError::IncorrectProgramId);
    }
    // Also validate the instruction discriminator, data, account identities,
    // signer/writable flags, and the operation your application expects.
    Ok(())
}
```

The helper queries exact data and account counts before copying. It returns only
the used prefixes and leaves unused buffer capacity untouched. `Ok(None)` means
no such sibling; `AccountDataTooSmall` means either buffer is too small. It never
returns truncated instruction data. Choose capacities that fit your program's
SBF stack budget; the helper does not allocate a larger buffer for you.

Each account record holds a native `Address`, `is_signer`, and `is_writable`.
These are the privileges of that sibling instruction, which can be narrower
than the transaction's privileges. Runtime callers can convert a record's native
address with `hopper_runtime::Address::from(record.address.clone())`.

## Understand the scope of the read

Index zero is the most recent processed sibling at the same depth and caller;
index one is the preceding sibling. Parents and children are excluded. This
is not a transaction-wide index or a complete nested execution trace.

The older `get_processed_instruction` convenience API reads up to 1,232 data
bytes and 64 account records. Its `None` result combines absence and capacity
failure. Runtime's `get_processed_instruction_data::<N>` chooses the data budget
but still needs internal space for up to 64 account records. The caller-buffer
API is preferable when you need account metadata or distinct capacity errors.

Host stubs return absence. Use a compiled SVM test or a cluster transaction to
test execution history.

## Keep authorization and outcomes explicit

Inspecting an instruction does not prove a token receipt or grant authority to
spend funds. Validate the expected action and account relationships. Use
[token receipts](TOKEN_RECEIPTS.md) when your contract requires a specific debit
or minimum amount received after CPI, and propagate errors to the instruction
boundary for rollback.

The `require_ed25519_instruction`, `require_secp256k1_instruction`, and
`require_secp256r1_instruction` helpers check only the sibling's program ID.
Applications must validate signature counts, offsets, referenced instruction
bytes, and the expected public key and message. WebAuthn additionally requires
the application's challenge and relying-party policy. Use the Instructions
sysvar for the transaction-level list and absolute cross-instruction references.

For framework applications, start with the stricter Instructions-sysvar helpers
in the [crypto guide](CRYPTO_CAPABILITIES.md), such as
`hopper::crypto::check_ed25519_signature_at`. Those validate the selected payload
and expected signer/message within their documented offset policy. They are
different APIs from the low-level `require_*_instruction` ID checks above.

## Regression coverage

The compiled fixture checks native and runtime reads, account identities and
privileges, empty and missing instructions, reverse sibling order, exclusion of
a nested child, 1,300-byte CPI data, and both kinds of capacity refusal.
[Fixture and runner](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/bench/sibling-introspection).
Release and devnet status are recorded in [validation](RELEASE_0_4_VALIDATION.md).

The September 27, 2026 run finalized all 11 devnet transactions, including the
two expected capacity refusals. Complete payer/program snapshots matched, and
the deployed ELF matched before and after. Native 0.4.4/runtime 0.4.5 clarify
the packaged documentation; their implementation and rebuilt v0/v3 binaries
match the tested release. [Inspect the evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-introspection-2026-09-27).
