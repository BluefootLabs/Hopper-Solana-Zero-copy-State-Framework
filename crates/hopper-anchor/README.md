# hopper-anchor

Anchor interoperability helpers for Hopper programs. Compute and check
Anchor's 8-byte account, instruction, and event discriminators and expose the
body after a checked discriminator without pulling in Anchor itself. This
crate does not validate the account owner, body length for a target type,
alignment, Pod safety, or the complete foreign layout; the caller or a typed
external-account adapter must perform those checks before typed access.

Part of the **[Hopper](https://hopperzero.dev)** framework.

## When to reach for this

Cross-program reads where the foreign program is Anchor-authored. Anchor's
account discriminator is `SHA256("account:<Type>")[..8]`. Hopper's own
`hopper_interface!` path uses a distinct Hopper header and wire-layout
fingerprint contract. The two identify different ABIs and are not
interchangeable; use this crate specifically for Anchor-shaped discriminator
checks, then apply the foreign program's owner and layout contract separately.

For emitting an Anchor-shaped IDL from a Hopper manifest, see
`hopper schema export --anchor-idl` in [`hopper-cli`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/tools/hopper-cli)
(implemented in [`hopper-schema`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/crates/hopper-schema/src/anchor_idl.rs)).

Docs: <https://docs.rs/crate/hopper-anchor>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
