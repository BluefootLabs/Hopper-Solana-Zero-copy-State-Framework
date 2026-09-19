# hopper-anchor

Anchor interoperability helpers for Hopper programs. Compute and check
Anchor's default 8-byte account, instruction, and event discriminators and
expose the body after a checked discriminator without pulling in Anchor
itself. Current Anchor retains those SHA-256 defaults but also supports
custom, variable-length discriminators. These helpers compute the defaults;
callers reading a custom-discriminator program must use that program's exact
published bytes. This crate does not validate the account owner, body length
for a target type, alignment, Pod safety, or the complete foreign layout. The
caller or a typed external-account adapter must perform those checks before
typed access.

Part of the **[Hopper](https://hopperzero.dev)** framework.

The API is a handful of free functions: `anchor_disc`, `anchor_ix_disc`, and
`anchor_event_disc` (`const fn`, computing `SHA256("account:<Type>")`,
`SHA256("global:<fn>")`, and `SHA256("event:<Event>")` prefixes),
`check_anchor_disc`, `check_and_body`, `check_ix_and_body`, `anchor_body`, and
`anchor_body_mut`. Through the root crate, enable the `anchor-interop` feature
on `hopper-lang` and use `hopper::anchor`.

## When to reach for this

Cross-program reads where the foreign program is Anchor-authored. Anchor's
account discriminator is `SHA256("account:<Type>")[..8]`. Hopper's own
`hopper_interface!` path uses a distinct Hopper header and wire-layout
fingerprint contract. The two identify different ABIs and are not
interchangeable; use this crate specifically for Anchor-shaped discriminator
checks, then apply the foreign program's owner and layout contract separately.

For emitting the current Solana IDL v0.1.0 shape from a Hopper manifest, see
`hopper schema export --anchor-idl <manifest>
--program-id <pubkey>` in [`hopper-cli`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/tools/hopper-cli/README.md)
(implemented in [`hopper-schema`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/crates/hopper-schema/src/anchor_idl.rs)).
The projection is lossless-only and marks Hopper account bodies with custom
`hopper-zero-copy-v1` serialization; it neither makes them Anchor-Borsh
decodable nor proves that the supplied program address is deployed.

Docs: <https://docs.rs/crate/hopper-anchor>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
