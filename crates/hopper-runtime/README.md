# hopper-runtime

[![Crates.io](https://img.shields.io/crates/v/hopper-runtime.svg)](https://crates.io/crates/hopper-runtime)
[![Docs.rs](https://img.shields.io/docsrs/hopper-runtime)](https://docs.rs/hopper-runtime)

Canonical low-level runtime surface for [Hopper](https://hopperzero.dev). This is the runtime boundary for account memory, CPI, syscalls, validation, and zero-copy state access.

These Rust APIs execute inside your Solana program. Checked account borrows
track live references; checked CPI rejects incompatible borrows. Shared reads
can coexist with read-only CPI, while a writable CPI requires exclusive access.
Release a mutable guard before invoking and load the result afterward. Raw and
unsafe paths have separate caller obligations. Validation does not replace
Solana's ownership, signer, writable-account, or PDA rules.

The unreleased framework facade can accept `&MyArgs` from a `#[hopper::args]`
layout in a normal program handler. Argument representation checks run before
account binding; the runtime's account and CPI checks still apply. This is a
`hopper-lang`/`hopper-derive` authoring feature, and does not change the runtime
account ABI. See [borrowed arguments](https://hopperzero.dev/docs/borrowed-arguments).

The runtime `AccountView` wraps the native backend's view. The framework facade
uses the runtime wrapper; native entrypoints use the backend type. Porting code
requires reviewing its types and validation contract, not just imports. See
[the execution model](https://hopperzero.dev/docs/model).

## Runtime checks (0.6)

The [function lab](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/bench/function-lab)
checks runtime calls inside compiled SBF and on public devnet. Runtime hashes
now share the native 20,000-slice input bound; the prior runtime-only bound was
16. Memory and compute budgets still limit practical input sizes. Host SHA-256
computes a real digest; host Keccak and BLAKE3 stubs are not correctness oracles.
Modular exponentiation uses little-endian integers and requires the SIMD-0529
cluster gate, which was absent on public devnet on October 7, 2026.
Native and runtime BLAKE3 probes failed on that devnet despite an accepted
deployment. EpochSchedule also disagreed with Clock and RPC; its decoded
account bytes were correct. Consult the dated
[network baseline](https://hopperzero.dev/docs/network-baseline) before relying
on a cluster-dependent call. Host stubs and compilation do not prove execution.

## Composable value validation (0.6)

`Pod::validate_value(&value)` checks a value's protocol representation in place.
`OptionByte<T>` validates its tag and each present payload; `EnumByte<E>` checks
the declared variants. Arrays and macro-authored layouts delegate to their
fields, including through aliases. This hook is separate from the unsafe `Pod`
layout contract and from account ownership or application rules. Raw overlays
retain their existing behavior. See
[borrowed arguments](https://hopperzero.dev/docs/borrowed-arguments) for checked
instruction parsing and custom-type behavior.

## Which crate should I start with?

For a new application, use [`hopper-lang`](https://crates.io/crates/hopper-lang)
under the Rust name `hopper`. This runtime crate is for teams that need direct
account, CPI, and validation APIs or are building framework integrations.
See the [vault walkthrough](https://hopperzero.dev/docs/start) for application authoring.

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

In 0.4.3, host System-transfer emulation resolves deduplicated account infos by
address even when reordered or accompanied by extra infos. Host mutable guards
also retain their native borrow through a movable release lease, fixing pointer
provenance during wrapping and projection. Both regressions pass Miri. Other
host CPIs remain validation-only no-ops; test callee behavior in an SVM or on devnet.

PDA helpers: find_program_address, create_program_address, plus Hopper's verify-only sha256 path that skips curve_validate for stored-bump PDA verification. They run in a plain `cargo test` with the cluster's answers, and `find_program_address_const` derives an address and bump at compile time.

In 0.5, `pda::try_find_program_address` returns
`Result<(Address, u8), ProgramError>` for canonical search without panicking
on malformed seeds. It uses Hopper's native, allocation-free implementation.

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

Token-2022 confidential transfers: a builder for each of the fifteen
instructions in `token_confidential_ix`, run end to end against mainnet's
Token-2022 with real proofs in the repository's confidential lab.

In 0.5, `AccountView::layout_id` returns `Option<[u8; 8]>` by value, and
`token::MintProgram` is an alias of `TokenProgram`. See the
[migration notes](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/MIGRATION_0_5.md).

Most users touch this crate transitively through hopper::prelude::*. Reach for hopper-runtime directly when writing a crate that needs the runtime surface without higher-level framework features.

## License

Apache-2.0. See [LICENSE](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/LICENSE).

## Native execution hardening

The 0.4.1 implementation uses the SVM abort syscall for no-allocation failures
and `no_std` panics. It requires no experimental inline assembly and terminates
without a compute-burning spin loop. Failure rolls back the transaction; it is
not a recoverable `ProgramError` return.

A token builder's `invoke()` rejects a missing authority signature before
the CPI. System and token builders check that no account in the instruction is
borrowed, plus the lamport gate when a write policy is installed; token builders
also refuse one account in two writable roles, since SPL Token accepts a
self-transfer and moves nothing. Signer and writable privileges are left to the
runtime, which refuses an escalation before the callee runs, and a multisig
authority's signers take the fully checked path; `invoke_signed` keeps every
local check. Nonempty seeds do not prove authority: Solana still derives and
verifies the PDA at the CPI boundary.

The native `invoke_and_read<T>` helper also checks the return-data producer and
typed prefix, rejecting a nested program's unforwarded result. Applications
remain responsible for validating the returned value's meaning.

## Account lifecycle safety in 0.4.2

Segment guards retain their native account borrow on SBF. Conflicting access,
resizing, closure, and writable checked CPI are refused until the guard drops.
Runtime callers edit multiple fields through the checked `split_segments_mut`
API; its raw constructor is no longer public. Close refusals preserve account
state even when caught. Direct self-transfers are balance-checked net zero.

Native `Ref` / `RefMut` mapping keeps the original lease while selecting a
field. Native `batch::ResizeWithPayer` (feature `cpi`) grows program-owned state
using live rent and a checked System transfer from a wallet or System-owned
PDA. It checks growth before charging, zeroes exposed bytes, and retains excess
rent on shrink. The application must authorize the operation. Runtime write
policies do not govern APIs deliberately called at the native layer.

[Compiled and devnet evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-lifecycle-2026-09-26).

## Processed sibling instructions

`crypto::get_processed_instruction_into` reads a prior sibling into caller-owned data
and account buffers, without heap allocation. It queries exact sizes before
copying, returns only the initialized prefixes, and distinguishes absence from
insufficient capacity. Account records include the address and signer/writable
flags. The list contains earlier calls at the same depth and caller; the
current instruction's parent and children are excluded.

The 0.4.3 native/0.4.4 runtime release corrected the wrappers' syscall return-code and length
handling. The owned convenience reader retains its 1,232-byte / 64-account limit;
the bounded data reader selects its byte capacity but still uses 64 account
records. The new API lets the program choose both scratch capacities. Host calls return
absence because host stubs have no instruction trace. Program-ID inspection does
not authorize a transfer or validate a signature payload.

[Instruction inspection guide](https://hopperzero.dev/docs/instruction-introspection).

## Token batches (0.5)

`TokenBatch` collects supported token instructions for one CPI. Accounts can
be reused across separate inner instructions; duplicate writable roles
within one are rejected, including token self-transfers. Failed pushes leave
the existing batch unchanged, including when a custom `TokenInstruction`
emits data and then returns an error. Each inner payload is limited to 255
bytes by the token batch wire format, in addition to the chosen buffer size.

## Borrow checked instruction batches (0.6)

`BoundedSlice<'_, T, N>` reads a u16-length-prefixed batch of alignment-1
`Pod` values. It checks capacity, byte length, and every nested representation
before returning a shared view of the original bytes. `parse_prefix` composes
with later arguments; `parse_exact` refuses a suffix. No element array is
allocated or copied. See [borrowed batches](https://hopperzero.dev/docs/borrowed-slices).

Owned instruction-vector metadata requires an exact element width. Custom
`TailCodec` implementations declare `FIXED_ENCODED_LEN = Some(width)` only when
every value has that encoding width; the default is unknown. Variable-length
account-tail codecs keep working. See the [0.6 migration guide](https://hopperzero.dev/docs/migration-0-6).
