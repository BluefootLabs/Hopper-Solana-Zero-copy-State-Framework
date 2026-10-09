# Write lower-level Solana programs

Hopper exposes account views, entrypoints, PDA helpers, checked CPI, and
syscalls below its typed framework. You can write a handler directly against
those APIs, then adopt generated account validation instruction by instruction.
Porting an existing program requires reviewing account layouts, ownership,
signers, aliasing, and CPI behavior; changing imports alone is not a guarantee
of compatibility or equal compute cost.

## A compact account handler

This excerpt shows an increment helper; supply instruction dispatch and
initialization in the complete program. The account owner, signer, layout,
and PDA are checked before mutation.

```rust
use hopper::prelude::{AccountView, Address, ProgramError, ProgramResult, WireU64};

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction, 3);

/// `[disc = 1][bump][count]`, 10 bytes.
#[derive(Clone, Copy, Debug, Default)]
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Counter {
    #[bump]
    pub bump: u8,
    pub count: WireU64,
}

fn increment(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, counter] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !counter.owned_by(program_id) {
        return Err(ProgramError::IncorrectProgramId);
    }
    let mut state = counter.load_compact_mut::<Counter>()?;
    let derived = hopper::pda::create_program_address(
        &[b"counter", authority.address().as_array(), &[state.bump]],
        program_id,
    )?;
    if counter.address() != &derived {
        return Err(ProgramError::InvalidSeeds);
    }
    state.count.checked_add_assign(1)?;
    Ok(())
}
```

The [complete counter](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/bench/framework-comparison/programs/counter/hopper-substrate/src/lib.rs)
also creates the account and dispatches instructions.

## Boundaries to preserve

For instruction bytes, the unreleased `#[hopper::args]` path lets you reuse one
fixed wire type across a manual entrypoint and a generated handler. Start with
`MyArgs::parse_exact_checked(payload)`; a `#[hopper::program]` handler can instead
accept `args: &MyArgs` and receive the same checked borrowed representation.
This does not select an account layout or grant access to account state. See
[borrowed arguments](BORROWED_ARGUMENTS.md).

- Checked borrows track live references. Release incompatible data borrows
  before CPI; unsafe access requires satisfying its safety contract.
- Safe resize checks the runtime growth limit relative to entry length.
- Token builders refuse one account in two writable roles. `TokenBatch`
  permits reuse across different instructions and rejects it within one.
- A stored bump and a matching hash do not prove canonicality. Use the
  canonical derivation APIs when account uniqueness requires it.
- Optional write policies constrain tracked writes and CPI delegation within
  their documented coverage. Raw and unchecked operations need separate review.

For the published 0.5 API, see [migration](MIGRATION_0_5.md).
The [readiness guide](RELEASE_READINESS.md) identifies unreleased additions.
Run your program's successful and rejected transactions in an SVM and on
devnet, then measure the complete workload.
