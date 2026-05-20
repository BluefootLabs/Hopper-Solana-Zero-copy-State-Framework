# hopper-system

Hopper-owned System Program builders: `Transfer`, `CreateAccount`, `Allocate`,
and `Assign`. Stack-allocated instruction data, no heap.

Part of the **[Hopper](https://hopperzero.dev)** framework.

```rust
use hopper::prelude::*;

system::Transfer {
    from: payer,
    to: vault,
    lamports: amount,
}
.invoke()?;
```

Re-exported through `hopper::prelude::*` as the `system` module and
`SYSTEM_PROGRAM_ID`.

Docs: <https://docs.rs/crate/hopper-system/0.2.1>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
