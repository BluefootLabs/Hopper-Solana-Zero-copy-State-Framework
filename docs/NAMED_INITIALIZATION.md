# Named initialization and ordinary Rust handlers

Hopper generates named input values from the same fixed-field declarations used
for positional constructors and wire conversion. For a `Vault` account,
`VaultFields` accepts native scalar inputs such as `u64` for `WireU64`:

```rust,ignore
ctx.init_vault_with(VaultFields {
    authority: *ctx.accounts.payer.key(),
    balance: 0,
    bump: 0,
})?;
```

This is the initialization path in the [SOL vault example](../examples/hopper-vault/src/lib.rs).
It creates a fresh account with the existing System Program CPI, writes the
header, acquires the normal checked mutable layout, and applies the values.
Signer, ownership, layout, borrow, and installed write-policy checks still apply
at their respective creation, validation, and borrowing boundaries.

The example creates a keypair account: its zero bump is an unused field, not a
canonical PDA claim. For a PDA, store the bump validated by the declared context.
A helper whose seeds use instruction arguments accepts those arguments after
the input values, in declaration order: `ctx.init_vault_with(fields, nonce)?`.

## Explicit control stays available

`Vault::new(...)` and headered `vault.set_inner(...)` retain their signatures.
Use `Vault::from_fields(VaultFields { ... })` to construct a fixed body, or
`vault.set_fields(VaultFields { ... })?` through an existing mutable borrow.
These methods do not create accounts or write headers.

Headered and compact state macros generate the named input type. Each input
field keeps the authored field's visibility. Wire scalars accept the same native
types as `new`; arrays and other field types retain their existing input types.
The companion is an ordinary Rust input value, **not a wire format**. The
account's discriminator, fingerprint, offsets, and byte size are unchanged.

For dynamic accounts, these values contain only the fixed head. Initialize tails
with their existing tail APIs. The input aggregate can consume stack space,
especially for large arrays; use individual field or cell access for small hot
path updates. No performance claim follows from the syntax alone.

The composed helper is generated for explicit `init` contexts. It is absent for
`init_if_needed` and `auto_lifecycle`; use their existing lifecycle and access
methods so that initial values do not silently reset existing state. Compact
named values are supported, but the composed context helper follows the existing
headered initialization path; it is not a new compact-account lifecycle API.

## Application-defined inputs

The public `AccountFields` trait in `hopper::prelude` lets a separate crate define
inputs without changing Hopper's macros:

```rust,ignore
impl AccountFields for CheckedVaultFields {
    type Layout = Vault;

    fn write(self, vault: &mut Vault) -> ProgramResult {
        self.validate()?;
        vault.set_fields(self.values)
    }
}
```

The trait supplies values, not authorization or a new account-wrapper protocol.
The generated helper obtains the mutable layout before invoking it. An
implementation can fail, and its writes are not locally undone. Propagate every
error to the instruction boundary to obtain Solana transaction rollback,
including reversal of a successful account-creation CPI. Catching an error and
returning success does not promise rollback.

Ordinary Rust methods, traits, helper functions, explicit CPI builders, borrowed
guards, and `with_mut` closures remain available. Named initialization does not
introduce an operation DSL, infer global invariants, or shard accounts.

## Verification

- [Consumer tests](../tests/named_fields.rs) check native scalar conversion,
  exact bytes, compact sizing, custom input validation, and propagated failure.
- [Compiled vault tests](../bench/framework-comparison/verifier/tests/named_vault_sbf.rs)
  execute initialization, prefunding, deposit, withdrawal, rejection paths, and
  rollback against the example ELF. They require `HOPPER_NAMED_VAULT_SBF` and
  `--ignored`; a normal host test pass does not execute them.

See the release evidence for actual build, devnet, and publication results.
