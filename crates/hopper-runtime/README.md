# hopper-runtime

[![Crates.io](https://img.shields.io/crates/v/hopper-runtime.svg)](https://crates.io/crates/hopper-runtime)
[![Docs.rs](https://img.shields.io/docsrs/hopper-runtime)](https://docs.rs/hopper-runtime)

Canonical low-level runtime surface for [Hopper](https://hopperzero.dev). This is the runtime boundary for account memory, CPI, syscalls, validation, and zero-copy state access.

## What's here

`layout::AccountFields` is an open input trait for generated or application-defined
initialization values. It writes through an existing mutable layout and grants
no ownership, signer, or write-policy authority. Propagate failures from a
composed initialization helper to the instruction boundary for transaction
rollback; the input trait does not undo earlier writes locally.

Typed AccountView with checked and unchecked borrow paths.

Safe headered, compact, and compact-dynamic typed loads independently check the
actual Rust type's byte range. Custom validation and reported-size methods cannot
bypass this memory bound. Compact-tail initialization also rejects overlapping
and overflowing tail-prefix offsets before writing.

Context<'a>: the canonical execution object for typed and raw handlers. The
separate LazyContext surface defers loader parsing for lazy programs.

CPI: invoke, invoke_signed, invoke_checked, invoke_signed_checked, plus the unsafe cpi::invoke_unchecked / cpi::invoke_signed_unchecked variants, whose `# Safety` contract requires the caller to rule out conflicting account-data borrows.

PDA helpers: find_program_address, create_program_address, plus Hopper's verify-only sha256 path that skips curve_validate for stored-bump PDA verification.

All PDA paths reject oversized seed lists instead of truncating them. The
16-seed limit includes the bump, and each seed is limited to 32 bytes.
`const_pda!` evaluates the supplied-bump hash at compile time; it does not
find or prove a canonical bump. SHA-only checks require the documented
program-owned account or signed-creation binding. For unchecked accounts,
use `verify_pda_address_checked` or `find_canonical_bump_checked`.

`find_bump_for_address` finds a matching bump and does not establish
canonicality, even for an owned account. Use `find_canonical_bump_checked`
for uniqueness. The facade's optional `canonical_pda!` proc macro derives
both the canonical address and bump at build time for explicit literal seeds.

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

New in 0.3.1: `token_mint` provides `InitializeMint2` and a
checked, allocation-free `MintPlan` for legacy and Token-2022 mints. It ties exact
space to explicit extension initialization, reads live rent, and supports PDA
and prefunded creation. It does not initialize unsupported extensions or infer
application authority policy. Propagate CPI errors to preserve rollback.

Most users touch this crate transitively through hopper::prelude::*. Reach for hopper-runtime directly when writing a crate that needs the runtime surface without higher-level framework features.

## License

Apache-2.0. See [LICENSE](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/LICENSE).
