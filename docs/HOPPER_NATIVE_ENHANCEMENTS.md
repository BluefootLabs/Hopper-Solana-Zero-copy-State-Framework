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

## In the tree after native 0.4.4 and runtime 0.4.5 (not yet published)

Source changes on `main` since the 0.4.4 / 0.4.5 publication; the registry
packages do not carry them until the next patch train.

- Every sysvar reader goes through `sol_get_sysvar` (110 CU for an image
  under 2,500 bytes) instead of the dedicated getters (100 plus the struct
  size). Clock is read in place, Rent as its 17-byte prefix, EpochSchedule
  decoded around its padded bool; the SlotHashes and StakeHistory latest
  readers make one call instead of two.
- `hopper_runtime::rent::live_rent` caches the Rent sysvar for the rest of
  the invocation in the reserved heap scratch, so `init`, realloc, rent
  checks, and mint plans in one instruction share one syscall.
- `hopper_native::arith` keeps 64-bit checked and saturating products off
  the `__multi3` helper; the wire integers and the ABI integers use it.
- The hash wrappers accept the runtime's 20,000-slice limit, write into an
  uninitialized buffer, and skip the dead result-code branch; `sha256` is
  real off-chain. `sha512` exists behind the `sha512-syscall` feature for
  clusters where SIMD-0512 is active (devnet and testnet on 2026-09-27).
- `#[derive(Accounts)]` refuses one account passed in two undeclared
  mutable roles (`ERR_ALIASED_MUTABLE_ACCOUNTS`) with one record-pointer
  compare per mutable pair; `dup = other` still declares an intended alias.
- The count-exact entrypoint refuses accounts past the matched bound
  (`ERR_TOO_MANY_ACCOUNTS`) instead of running on a truncated list;
  `cu_trace!`, `cu_measure!`, and the DSL's `context_schema` work from a
  downstream crate; user error codes cannot land in the framework's refusal
  pages; oversized CPI events fail at compile time; `hopper build` fails on
  a builder-reported stack frame overflow.
- The token builders cover the whole SPL Token instruction set and every
  Token-2022-only instruction, target either program (`invoke_on`,
  `invoke_for_owner`), batch into one p-token `Batch` CPI (`TokenBatch`),
  and are checked byte for byte, metas included, against the canonical
  `spl-token-2022-interface` constructors; `MintPlan` initializes thirteen
  fixed-size extensions.

The changelog's Unreleased section carries the measured numbers.

## Native 0.4.4 and runtime 0.4.5: instruction inspection

**Published on crates.io:** `hopper-native` 0.4.4 and `hopper-runtime` 0.4.5.
Framework and CLI remain 0.4.0; Solana integration remains 0.4.1. Registry
downloads match the publication source and checksums. A registry-only consumer
compiles the new inspection API alongside token payouts and framework 0.4.0.

Programs can inspect prior sibling calls directly on chain using caller-owned
data and account buffers. `get_processed_instruction_into` queries exact sizes,
copies only when both buffers fit, exposes account identities and privileges,
and distinguishes absence from insufficient capacity. It needs no heap or
off-chain trace service. [Instruction inspection guide](INSTRUCTION_INTROSPECTION.md).

This fixes the old wrappers' reversed syscall result handling and incorrect
length assumptions. Published native 0.4.2/runtime 0.4.3 reproduce four failures
in compiled SBF. The fix passes all 11 scenarios on both SBF v0 and v3, including
empty and missing instructions, sibling order, child exclusion, a 1,300-byte CPI
payload and two capacity refusals. Host tests, Miri, framework/core tests, Clippy,
local API docs and the 29-package unsafe-contract scan passed.

The September 27 devnet run finalized **11 transactions** with matching complete
payer/program snapshots, including two expected capacity refusals. The deployed
v0 ELF matched before and after the run. The implementation shipped in native
0.4.3/runtime 0.4.4; the final patch corrects packaged README wording. Its Rust
implementation is unchanged and rebuilt v0/v3 ELFs are byte-identical to those
tested. Source and binary lineage is recorded in the evidence.

Sibling inspection is scoped to the same depth and caller. It is not a complete
transaction trace, transfer-outcome proof, or signature-payload validator.
Applications retain explicit authorization and outcome checks. These targeted
results do not establish a whole-framework security audit or universal speed lead.

[Signatures, snapshots, source pins and publication evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-introspection-2026-09-27).

```sh
cargo update -p hopper-native -p hopper-runtime
```

## Runtime 0.4.3 and Solana integration 0.4.1: token receipts

**Published on crates.io:** `hopper-runtime` 0.4.3 and `hopper-solana` 0.4.1.
At that release, native was 0.4.2; framework and CLI were 0.4.0. Downloaded package sources
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

## Native/runtime 0.4.2: account lifecycle and borrow safety

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

## Native/runtime 0.4.1 patch: September 26, 2026

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
