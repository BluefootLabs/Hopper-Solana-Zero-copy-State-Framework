# hopper-runtime

[![Crates.io](https://img.shields.io/crates/v/hopper-runtime.svg)](https://crates.io/crates/hopper-runtime)
[![Docs.rs](https://img.shields.io/docsrs/hopper-runtime)](https://docs.rs/hopper-runtime)

Canonical low-level runtime surface for [Hopper](https://hopperzero.dev). This is the runtime boundary for account memory, CPI, syscalls, validation, and zero-copy state access.

## What's here

Typed AccountView with checked and unchecked borrow paths.

Context<'a>: the canonical execution object for typed and raw handlers. The
separate LazyContext surface defers loader parsing for lazy programs.

CPI: invoke, invoke_signed, invoke_checked, invoke_signed_checked, plus the unsafe cpi::invoke_unchecked / cpi::invoke_signed_unchecked variants, whose `# Safety` contract requires the caller to rule out conflicting account-data borrows.

PDA helpers: find_program_address, create_program_address, plus Hopper's verify-only sha256 path that skips curve_validate for stored-bump PDA verification.

Layout contract: LayoutContract trait, header read/write, layout fingerprint comparison.

Ambient write policies: data ranges, lamport authority, and writable CPI
delegation remain enforced while a policy is installed. On SBF, installation
registers the evaluator in reserved VM memory. Programs with no installation
path can omit the evaluator from their binary without disabling guard APIs.

Guard macros: require!, require_eq!, require_neq!, require_keys_eq!, require_keys_neq!, require_gt!, require_gte!, require_lt!, require_lte!, plus err! / error! short-form.

Native boundary: direct routing to hopper-native for loader input, account memory, CPI, PDA helpers, and syscall access.

System Program builders: Transfer, CreateAccount, CreateAccountAllowPrefund (one CPI that creates a possibly pre-funded account; the `init` lifecycle uses it), Allocate, Assign.

Rent-exemption helper: rent::check_rent_exempt(account) backing the #[account(rent_exempt = enforce)] field keyword.

Token / Token-2022 readers: base-layout SplMint and SplTokenAccount external views, plus the token_2022_ext TLV scanner that powers the extensions::* constraints.

Most users touch this crate transitively through hopper::prelude::*. Reach for hopper-runtime directly when writing a crate that needs the runtime surface without higher-level framework features.

## License

Apache-2.0. See [LICENSE](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/LICENSE).
