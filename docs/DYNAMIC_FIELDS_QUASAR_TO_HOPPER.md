# Dynamic Fields: Quasar To Hopper

This page is a code-facing migration note for teams moving bounded dynamic account data into Hopper. It compares the authoring shape only. Benchmark and market claims stay in the benchmark docs.

## Side-by-side shape

Quasar-style dynamic fields keep the account source compact:

```rust
#[account]
pub struct Multisig<'a> {
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Pubkey, 10>,
}
```

Hopper keeps the same first-touch shape while lowering it into a fixed body plus a compact bounded tail:

```rust
#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
}
```

The generated Hopper layout preserves a zero-copy fixed body and stores variable bytes in the account tail. That lets hot fixed fields stay predictable while string and vector payloads remain bounded, validated, and schema-visible.

## Deliberate bare-tail difference

Hopper's tail is deliberately bare: it stores the bounded payload bytes described by the account schema instead of wrapping every dynamic value in an extra runtime object header. The account header, layout fingerprint, field metadata, and dynamic-tail schema hash are the contract. The tail itself stays compact.

That gives Hopper three properties that matter for stateful programs:

- fixed fields remain zero-copy and stable across reads,
- dynamic fields are bounded by the source type and checked by generated helpers,
- schema and compatibility tools can verify the tail contract without inventing a second serialization model.

## Migration pattern

1. Keep fixed, hot fields first.
2. Use `String<'a, N>` and `Vec<'a, T, N>` for bounded dynamic fields.
3. Pick caps that are protocol rules, not UI guesses.
4. Use generated setters and push helpers so bounds and tail offsets stay checked.
5. Export schema during review so clients and migrations see the same tail contract.

```rust
impl<'info> RenameMultisig<'info> {
    pub fn rename(&self, label: &str) -> ProgramResult {
        self.multisig.set_label(label)
    }
}

impl<'info> AddSigner<'info> {
    pub fn add_signer(&self, signer: Address) -> ProgramResult {
        self.multisig.push_unique_signer(signer).map(|_| ())
    }
}
```

The complete working example is [../examples/quasar-port-20-min](../examples/quasar-port-20-min).

## Token-2022 wedge

Dynamic fields often sit beside Token-2022 vault metadata. Hopper keeps those checks in the same zero-copy authoring model: use account constraints or `hopper_token_2022` helpers to require or forbid TLV extensions before mutating state. See [TOKEN_2022_GUIDE.md](TOKEN_2022_GUIDE.md) for extension policy examples.
