# hopper-runtime

[![Crates.io](https://img.shields.io/crates/v/hopper-runtime.svg)](https://crates.io/crates/hopper-runtime)
[![Docs.rs](https://img.shields.io/docsrs/hopper-runtime)](https://docs.rs/hopper-runtime)

Canonical low-level runtime surface for [Hopper](https://hopperzero.dev). This is the runtime boundary for account memory, CPI, syscalls, validation, and zero-copy state access.

## What's here

Typed AccountView with checked and unchecked borrow paths.

Context<'a>: the typed entry point every Hopper handler receives.

CPI: invoke, invoke_signed, plus the unchecked Tier C variants with seven-item Safety invariants documented inline.

PDA helpers: find_program_address, create_program_address, plus Hopper's verify-only sha256 path that skips curve_validate for stored-bump PDA verification.

Layout contract: LayoutContract trait, header read/write, layout fingerprint comparison.

Guard macros: full Anchor-parity family (require!, require_eq!, require_neq!, require_keys_eq!, require_keys_neq!, require_gt!, require_gte!, require_lt!, require_lte!), plus err! / error! short-form.

Native boundary: direct routing to hopper-native for loader input, account memory, CPI, PDA helpers, and syscall access.

System Program builders: Transfer, CreateAccount, Allocate, Assign.

Rent-exemption helper: rent::check_rent_exempt(account) backing the #[account(rent_exempt = enforce)] field keyword.

Token / Token-2022 readers: base-layout readers for Mint and TokenAccount, plus the TLV scanner that powers the extensions::* constraints.

Most users touch this crate transitively through hopper::prelude::*. Reach for hopper-runtime directly when writing a crate that needs the runtime surface without higher-level framework features.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
