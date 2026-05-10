# hopper-system

Hopper-owned System Program builders: `Transfer`, `CreateAccount`, `Allocate`,
and `Assign`. Stack-allocated instruction data, no heap.

Part of the **[Hopper](https://hopperzero.dev)** framework.

```rust
use hopper::prelude::*;

hopper_system::instructions::Transfer {
    from: payer,
    to: vault,
    lamports: amount,
}
.invoke()?;
```

Re-exported through `hopper::prelude::*` as `system_instructions::*` and
`SYSTEM_PROGRAM_ID`.

Docs: <https://docs.rs/crate/hopper-system/0.1.0>

License: Apache-2.0.
