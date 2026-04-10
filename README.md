# hopper-macros

Declarative macros for the Hopper zero-copy state framework.

All `macro_rules!`. No proc macros. You can build a full Hopper program without
ever touching a derive or attribute macro. These are here to cut boilerplate,
not to hide complexity.

## Macros

| Macro | What it does |
|-------|-------------|
| `hopper_layout!` | Define a zero-copy account layout with auto-generated header, SHA-256 fingerprint, and tiered load methods |
| `hopper_dispatch!` | Route instructions by discriminator byte |
| `hopper_check!` | Composable constraint checks with clear error messages |
| `hopper_validate!` | Validation combinator blocks |
| `hopper_error!` | Define program error codes |
| `hopper_require!` | Assert-or-error shorthand |
| `hopper_init!` | Account initialization with header write |
| `hopper_close!` | Account closure with lamport drain |
| `hopper_register_discs!` | Assert discriminator uniqueness across a program |
| `hopper_verify_pda!` | PDA verification using a layout's cached bump offset |
| `hopper_invariant!` | Inline invariant check runner |
| `hopper_segment!` | Define typed segment regions within an account |
| `hopper_virtual!` | Map state across multiple accounts |
| `hopper_interface!` | Cross-program account reading by fingerprint |
| `hopper_accounts!` | Declare typed account context structs |
| `hopper_manifest!` | Declare a program manifest for schema export |
| `hopper_assert_compatible!` | Compile-time layout compatibility assertion |
| `hopper_assert_fingerprint!` | Compile-time fingerprint equality assertion |

## Usage

These are re-exported through the main `hopper` crate. You don't need to
depend on `hopper-macros` directly.

```rust
use hopper::prelude::*;

hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority>  = 32,
        balance:   WireU64                  = 8,
    }
}

hopper_dispatch! {
    match instruction_data {
        0 => process_init(accounts, data),
        1 => process_deposit(accounts, data),
    }
}
```

## License

Apache-2.0
