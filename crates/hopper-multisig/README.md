# hopper-multisig

M-of-N signer threshold checks for Hopper programs. Duplicate-signer
prevention, zero heap allocation, and a fixed stack footprint.

Part of the **[Hopper](https://hopperzero.dev)** framework.

Pass the account views that are allowed to satisfy the threshold. Hopper checks
that each address is unique, counts real transaction signers, and returns
`MissingRequiredSignature` or `InvalidArgument` when the set is not valid.

```rust
use hopper_multisig::check_threshold;

check_threshold(&[admin_a, admin_b, admin_c], 2)?;
```

Docs: <https://docs.rs/crate/hopper-multisig/0.1.0>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
