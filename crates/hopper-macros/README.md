# hopper-macros

[![Crates.io](https://img.shields.io/crates/v/hopper-macros.svg)](https://crates.io/crates/hopper-macros)
[![Docs.rs](https://img.shields.io/docsrs/hopper-macros)](https://docs.rs/hopper-macros)

Declarative macros for the Hopper zero-copy state framework. All macro_rules!, no proc macros. You can build a full Hopper program without ever touching a derive or attribute macro.

## Core macros

hopper_layout!: Define a zero-copy account layout with auto-generated header, SHA-256 fingerprint, and tiered load methods.

hopper_register_discs!: Assert discriminator uniqueness across a program.

hopper_check!: Composable constraint checks with clear error messages.

hopper_validate!: Validation combinator blocks.

hopper_error!: Define program error codes.

hopper_require!: Assert-or-error shorthand.

hopper_init!: Account initialization with header write.

hopper_close!: Account closure with lamport drain.

hopper_verify_pda!: PDA verification using a layout's cached bump offset.

hopper_invariant!: Inline invariant check runner.

hopper_segment!: Define typed segment regions within an account.

hopper_virtual!: Map state across multiple accounts.

hopper_interface!: Cross-program account reading by fingerprint.

hopper_accounts!: Declare typed account context structs.

hopper_manifest!: Declare a program manifest for schema export.

hopper_assert_compatible!: Compile-time layout compatibility assertion.

hopper_assert_fingerprint!: Compile-time fingerprint equality assertion.

const_assert_pod!: Compile-time checks for manual Pod implementations.

Instruction-routing macros such as hopper_dispatch!, hopper_dispatch_lazy!, and hopper_dispatch_8! live in hopper-systems and are re-exported by the main hopper crate.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
