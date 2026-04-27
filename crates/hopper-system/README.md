# hopper-system

Hopper-owned System Program builders. `Transfer`, `CreateAccount`, `Allocate`,
`Assign`. Stack-allocated instruction data, no heap.

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
`SYSTEM_PROGRAM_ID`. License: Apache-2.0.
