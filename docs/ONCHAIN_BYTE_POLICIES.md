# On-chain byte policies

Hopper can constrain a handler to selected bytes inside a writable Solana
account. The policy runs in the program: no indexer, manifest server, Grillo
report, or off-chain proof is needed to admit or refuse a supported write.
Solana still locks the whole writable account and charges its normal fees.

## An application you can build

[`hopper-byte-allowance`](../examples/hopper-byte-allowance) stores four
application quotas in a 272-byte account. Each cell has a delegate, a limit,
spent units, and a revision. These units are application accounting, not
lamports or SPL tokens; the example does not hold or transfer token custody.

Initialization creates the account through a System Program CPI. A consume
instruction requires the selected delegate's signature and the current
revision, rejects zero amounts and overflow, checks the limit, and changes
only that cell's spent units and revision. The authority can change a limit
without reducing it below the amount already spent. That also advances the
revision, so a consumption prepared against an older limit is refused.
There is no close instruction or rent-recovery path in this small example.

```rust,ignore
#[derive(Accounts)]
#[accounts(strict_writes, lamports())]
#[instruction(slot: u16)]
pub struct Consume<'info> {
    pub delegate: Signer<'info>,
    #[account(cells(slot; spent, revisions))]
    pub book: Account<'info, AllowanceBook>,
}

// After authenticating the delegate and checking quota/revision:
*ctx.book_spent_cell_mut()? = WireU64::new(next_spent);
*ctx.book_revisions_cell_mut()? = WireU64::new(next_revision);
```

The generated accessors are available in the published 0.3.2 framework.
The accessor captures the
selector during context binding, infers the column's element type, checks the
array bound and offset arithmetic, and acquires the normal policy-checked
segment lease. Callers do not repeat a byte offset or provide a second selector.
`book_spent_cell_ref()` supplies the matching read convenience; it does not
restrict other reads. Out-of-range access returns `InvalidInstructionData`.

Selectors must be `u8`, `u16`, or `u32`; a context supports up to eight distinct
selectors. Composite contexts do not yet support `cells(...)`. The existing
whole-column accessor does not gain whole-column write permission from a
selected-cell declaration.

## What is enforced

Typed binding checks the account's owner and layout as well as its declared
signer and writable requirements. `cells(slot; spent, revisions)` grants only
the selected elements in those columns. Neighboring cells, authority,
delegates, limits, and the header stay outside the consumer's mutable grant.
Explicit `lamports()` permits no lamport mutation through the governed APIs.
Supported direct `AccountView` writes are subject to the macro-bound ambient
gate too. With this explicit lamport dimension, writable delegation through
validated CPI helpers requires whole-account data permission and a lamport
grant; these selected-cell contexts provide neither. Bare `strict_writes`
without the lamport dimension retains legacy ungoverned writable-CPI behavior.

The handler still owns business rules: who may spend, what a unit means,
which revision is acceptable, and how much is allowed. `SEALED` rejects unsafe
code in handlers unless explicitly opted in; it is not an independent audit
or a sandbox for arbitrary unsafe dependencies. Raw-pointer writes outside
the governed APIs can bypass Hopper's policy. Always propagate failures when
transaction rollback is required.

## Where Cicada and Grillo fit

Cicada applies the same cell-policy idea to a larger on-chain intent lifecycle,
with custody checks, route commitments, and settlement postconditions. Its
application logic remains useful independently of a report producer. A route
program ID or call commitment does not freeze upgradeable callee code.

Grillo checks supplied execution evidence separately. It can support release
inspection and incident analysis, but it is not a dependency of byte-policy
enforcement and should not be marketed as consensus-enforced runtime proof.

## Reproduce the program tests

Build the example with pinned SBF tools, then run the compiled suite:

```sh
cargo build-sbf --tools-version v1.54 --arch v0 \
  --manifest-path examples/hopper-byte-allowance/Cargo.toml \
  --sbf-out-dir target/byte-allowance -- --locked
HOPPER_BYTE_ALLOWANCE_SBF="$PWD/target/byte-allowance/hopper_byte_allowance.so" \
  cargo test --manifest-path bench/framework-comparison/verifier/Cargo.toml \
  --test byte_allowance_sbf --locked -- --ignored --nocapture
```

Repeat with `--arch v3` and a separate output directory. The suite checks the
full resulting accounts, all four selectors, failed authentication, stale
revisions, quota failures, overflow, malformed accounts, and reinitialization.
`scripts/test-byte-allowance-devnet.py` exercises a deployed ELF on public
devnet and records finalized transactions and exact account snapshots.

## Verified 0.3.2 release

The September 25 capture passed 40 finalized allowance transactions, 20
runtime-gate transactions, and two orderbook transactions. It includes successful
updates, expected refusals, exact account-state checks, and exact deployed ELF
checks before and after each lane. A prior harness attempt stopped at CLI account
parsing after two delegate-funding transactions; those are outside the 62-case
completed capture.

The v0 allowance ELF is 24,920 bytes. On devnet, ordinary consumption used 889 CU
and a limit update used 831 CU. These are whole-instruction measurements for
this program and input shape, not intrinsic accessor costs or a framework ranking.
Fresh programs using only crates.io dependencies reproduce the allowance and
runtime-gate ELFs exactly and pass all four compiled suites.

See the [on-chain evidence](../audit/onchain-byte-policies-2026-09-25/README.md)
and [registry verification](../audit/registry-publication-2026-09-25/README.md).

For current network boundaries, see the [September 25 source and activation
review](SOURCE_REVIEW_2026-09-25.md). Alpenglow is a consensus change, not a
sub-account locking feature or a new Hopper byte-fee mechanism.


## Published 0.4.0 revalidation — September 26

The 0.4.0 byte-allowance artifact passed 40 fresh finalized devnet transactions,
including all four quota slots and refusal cases. Complete expected account
snapshots were checked after every transaction. Program dumps before and after
match the gated ELF and the registry-only consumer build. Consumption measured
889 CU and limit updates 831 CU in this fixture. See the
[use-case review evidence](../audit/use-case-review-2026-09-26/README.md).
The earlier 0.3.2 capture above remains separately dated.

## Example instruction ABI

| Opcode | Accounts in order | Arguments after the one-byte opcode |
|---|---|---|
| 0 initialize | authority signer+writable, new book signer+writable, System Program | four delegate addresses, u64 initial limit |
| 1 consume | delegate signer, book writable | u16 slot, u64 expected revision, u64 amount |
| 2 set_limit | authority signer, book writable | u16 slot, u64 expected revision, u64 limit |

Integers are little-endian. Instruction payload lengths are exact. The bounded
entrypoint may ignore surplus account metas after the declared accounts;
`sealed` does not enforce an exact account count. The compiled suite checks
that an extra account remains unchanged and only the expected cells change.
There is no reset, close, or rent-recovery instruction.
