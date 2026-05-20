# hopper-anchor

Anchor interoperability for Hopper programs. Read accounts created by
Anchor programs, decode their 8-byte SHA-256 discriminators, and validate
the layouts without pulling in the Anchor framework itself.

Part of the **[Hopper](https://hopperzero.dev)** framework.

## When to reach for this

Cross-program reads where the foreign program is Anchor-authored. Hopper's own
cross-program path (`hopper_interface!`) uses 8-byte SHA-256 layout
fingerprints, which are functionally compatible with Anchor's
`anchor:account:Foo`-style discriminators. This crate exposes that compatibility
surface explicitly.

For emitting an Anchor-shaped IDL from a Hopper manifest, see
`hopper schema export --anchor-idl` in [`hopper-cli`](../../tools/hopper-cli)
(implemented in [`hopper-schema`](../hopper-schema/src/anchor_idl.rs)).

Docs: <https://docs.rs/crate/hopper-anchor/0.2.1>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
