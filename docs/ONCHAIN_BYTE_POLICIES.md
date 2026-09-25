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

The generated accessors arrive in the 0.3.2 source. Until its publication is
verified, use a local checkout for this example. The accessor captures the
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
gate too. Whole-account writable CPI delegation cannot fit this narrow grant.

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

For current network boundaries, see the [September 25 source and activation
review](SOURCE_REVIEW_2026-09-25.md). Alpenglow is a consensus change, not a
sub-account locking feature or a new Hopper byte-fee mechanism.
