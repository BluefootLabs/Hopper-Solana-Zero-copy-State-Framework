# Hopper native execution

`hopper-native` owns loader parsing, duplicate-account resolution, eager and lazy
entrypoints, account views, syscalls, PDA helpers, and native CPI. The runtime
layer adds the account contracts and guarded operations used by authored programs.

The normal path is `no_std` with no heap allocation. Advanced applications can
choose entrypoint account ceilings, explicit raw APIs, or cluster-gated features.
The default scanning entrypoint remains usable without optional account-table
or static-syscall assumptions.

Checked CPI validates identities, privileges, and borrow compatibility before
invocation. Signed CPI provides authority only for PDAs derived by the caller's
program. Returning from CPI does not remove the application's responsibility to
validate balances and downstream behavior when its contract requires that.

Read [program architecture](ARCHITECTURE.md) for layer responsibilities and
[unsafe invariants](UNSAFE_INVARIANTS.md) before using raw pointers or unchecked
invocation. Performance work belongs in reproducible complete-program fixtures.

## Runtime 0.4.3 and Solana integration 0.4.1 — token receipts

**Published on crates.io:** `hopper-runtime` 0.4.3 and `hopper-solana` 0.4.1.
Native remains 0.4.2; framework and CLI remain 0.4.0. Downloaded package sources
and checksums match the release commit, and a registry-only payout consumer
compiles with framework 0.4.0.

`TokenTransferSnapshot` binds a source and destination to an exact debit and an
explicit minimum receipt. It re-reads their program owner, base account shape,
initialized state, mint, and token authorities after CPI. No data borrow remains
held during the transfer. Applications keep control of authorization, extension
policy, and their choice of transfer builder. [Payout guide](TOKEN_RECEIPTS.md).

The devnet fixture finalized **34 transactions** using real classic SPL Token and
Token-2022 accounts with a 1% transfer fee. All complete account snapshots matched,
including withheld fees, lamports and transaction fees. Thirteen transactions
were expected refusals; seven proved rollback after a successful nested token
CPI. The deployed v0 ELF matched the tested binary before and after the run.

The runtime patch also fixes two host regressions. Deduplicated System-transfer
infos now resolve by address instead of list position. Host mutable borrow guards
retain a release lease without moving a parent mutable reference after deriving
a pointer. The baseline transfer debited an unrelated extra account; Miri caught
the borrow-wrapper provenance error. Both fixes pass their regressions and Miri.
The existing on-chain representations did not have these two host defects.

Compiled v0/v3 token scenarios, changed-crate tests, framework/core tests, Clippy,
local API docs, and the 29-package unsafe-contract scan passed. Host token CPI
no-ops are not used as evidence of token movement. This is targeted validation,
not an independent security audit or a whole-framework speed comparison.

[Signatures, snapshots, builds and registry evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/token-outcomes-2026-09-26).

```sh
cargo update -p hopper-runtime -p hopper-solana
```

## Native/runtime 0.4.2 — account lifecycle and borrow safety

**Published on crates.io:** `hopper-native` and `hopper-runtime` 0.4.2.
Registry downloads match the release source and checksums.

The 0.4.2 patch keeps an account's native borrow alive for the full lifetime of
an SBF segment guard. Conflicting whole-account access, another registry,
closure, resizing, and writable checked CPI are refused while that guard lives.
Use `split_segments_mut` to edit several disjoint fields together; it validates
all ranges and holds one exclusive account borrow. Its hidden unchecked
constructor is now crate-private.

Close helpers preflight borrows, writable requirements, aliases, arithmetic,
and applicable runtime policies before changing balances. A caught refusal
leaves both accounts intact. Direct self-transfers are balance-checked net zero;
an underfunded account cannot serve as its own resize payer.

The native `batch::ResizeWithPayer` builder funds missing rent through a checked
System Program CPI, checks the current program owner and entry-time growth
limit before charging, and zeroes newly exposed bytes. Shrinking retains excess
lamports. Applications still authorize the resize and propagate CPI errors.
`Ref` and `RefMut` now provide `map`, `try_map`, and `filter_map` for field access
that retains the original account borrow.

The old published runtime fails the compiled segment regression; the patched
runtime passes. Native/runtime fixtures and the treasury, multisig, token escrow,
byte allowance, and ambient write-gate suites pass on sBPF v0 and v3. The new
lifecycle run finalized **24 devnet transactions**, including two expected
transaction refusals and successful instructions that catch and inspect local
refusals. Complete lifecycle snapshots include fees, data, balances, owners,
and the closed-account result. Both deployed v0 ELFs matched before and after.
The native lifecycle and mapped-borrow tests also pass Miri. The final projection
correction produces byte-identical lifecycle ELFs; source lineage records that
relationship instead of treating a host-only result as on-chain evidence.

[Source, test logs, signatures, snapshots, and hashes](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-lifecycle-2026-09-26).

Update existing lockfiles with `cargo update -p hopper-native -p hopper-runtime`.
The framework and CLI remain 0.4.0. These are targeted correctness and developer
experience improvements; they do not establish universal performance leadership.

## Native/runtime 0.4.1 patch — September 26, 2026

`hopper-native` and `hopper-runtime` **0.4.1 are published**. The framework and
CLI remain 0.4.0; support packages keep their independent versions. Existing
lockfiles must update the native/runtime dependencies to pick up the patch:

```sh
cargo update -p hopper-native -p hopper-runtime
```

The patch rejects missing signers in specialized checked CPI, uses immediate
SVM aborts for no-allocation failures and panics, and validates the producer
and typed prefix of CPI return data. A nested callee's unforwarded return data
is rejected. Application-level value and outcome checks are still required.

[Validation evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-multisig-2026-09-26).
