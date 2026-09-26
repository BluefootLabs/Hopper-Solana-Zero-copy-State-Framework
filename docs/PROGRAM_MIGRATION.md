# Moving a program to Hopper

Migrate the instruction and asset contract deliberately. Preserve client-visible
account addresses, discriminators, amounts, and permissions where required;
introduce an explicit version or migration when the wire layout changes.

## Program structure

Use `#[account]` for state, `#[derive(Accounts)]` for instruction roles,
`#[program]` for handlers, and `Ctx<T>` for validated context. Start with
`ctx.accounts.*` and typed wrappers. The systems layer is available for programs
that need explicit segment layouts or low-level control.

## Account and asset contract

- Declare signer, writable, owner, seed, and relationship constraints.
- Distinguish supplied bumps from canonical derivation and persist the validated
  value when an application uses stored bumps.
- Transfer wallet SOL through the System Program. Transfer tokens through the
  correct token program, with explicit mint, authority, and extension policy.
- Preserve live rent when debiting a program-owned data account.
- Release state borrows before CPI and test failure after earlier CPIs succeed.

## State and clients

Choose a headered or compact layout consciously. Headered layouts carry a
16-byte identity/version contract. Use external account adapters when a foreign
wire format must remain unchanged. A framework change does not make old account
bytes compatible automatically.

Use bounded strings/vectors for capped variable fields, and explicit final tails
for deliberate remaining-byte formats. Export the manifest, regenerate clients,
and compare instruction payloads and account flags with the intended ABI.

Run compiled success, refusal, and rollback fixtures. Deploy the tested artifact
on devnet and verify real balances and account closures. See
[funded escrow](../examples/hopper-escrow) and
[bounded multisig](../examples/hopper-bounded-multisig) for complete lifecycles.
