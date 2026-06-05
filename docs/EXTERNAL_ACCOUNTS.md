# External Accounts

Hopper verifies what it owns, adapts what it knows, and gets out of the way when raw Solana is needed.

That is the account model Hopper uses for serious Solana programs:

| Account class | Hopper surface | Contract |
| --- | --- | --- |
| Hopper-owned | `Account<'info, T>` / `InitAccount<'info, T>` | Owner, role, discriminator, version, layout fingerprint, schema epoch, borrow guards |
| Known foreign | `ExternalAccount<'info, T: ExternalZeroCopy>` | Owner/discriminator/min-length/custom adapter checks, no Hopper header required |
| Interface | `Interface<'info, I>` / `InterfaceAccount<'info, T>` / `ExternalResolve` | Owner-selected dispatch over several valid programs or layouts |
| Remaining | `RemainingAccounts`, `RemainingTyped`, `RemainingGroup`, `RemainingLazy` | Strict duplicate policy by default, typed/grouped/lazy parsing |
| Raw | `UncheckedAccount<'info>` / `AccountView` | Explicit raw control, caller-owned validation |
| Stored instruction | `StoredInstruction<'a>` / `StoredAccountMeta` | Arbitrary CPI payloads for governance/proposal execution |

Hopper-owned accounts get contract-checked zero-copy. External known accounts get adapter-checked zero-copy. Raw accounts stay explicit.

## Typed External Views

External adapters implement `ExternalZeroCopy`. The view returned by the adapter owns the account-data borrow guard, so typed access stays zero-copy without returning references after the borrow is released.

```rust
use hopper::prelude::*;

pub const PYTH_ID: Address = Address::new_from_array([1; 32]);

pub struct PythPrice;

pub struct PythPriceView<'a> {
    data: Ref<'a, [u8]>,
}

impl PythPriceView<'_> {
    pub fn price(&self) -> i64 {
        i64::from_le_bytes(self.data[8..16].try_into().unwrap())
    }
}

impl ExternalZeroCopy for PythPrice {
    type View<'a> = PythPriceView<'a>;

    const OWNER: Option<Address> = Some(PYTH_ID);
    const DISCRIMINATOR: Option<&'static [u8]> = Some(b"PX");
    const MIN_LEN: usize = 16;

    fn view<'a>(data: Ref<'a, [u8]>) -> Result<Self::View<'a>> {
        Ok(PythPriceView { data })
    }
}

#[derive(Accounts)]
pub struct UpdateRisk<'info> {
    pub oracle: ExternalAccount<'info, PythPrice>,

    #[account(mut)]
    pub market: Account<'info, Market>,
}

pub fn update_risk(ctx: Ctx<UpdateRisk>) -> ProgramResult {
    let price = ctx.accounts.oracle.view()?.price();
    ctx.accounts.market.with_mut(|market| market.update_price(price))
}
```

Use `with_view` when the borrow should stay visually scoped:

```rust
let price = ctx.accounts.oracle.with_view(|oracle| Ok(oracle.price()))?;
```

## Lenses

When a protocol only needs one checked field, a full adapter view is optional. A lens reads a copyable value from a checked offset while carrying the same account-data borrow guard.

```rust
let amount = ctx.accounts.token
    .require_owner(&SPL_TOKEN_ID)?
    .lens::<u64, { TOKEN_ACCOUNT_AMOUNT_OFFSET }>()?
    .get();

let mint = ctx.accounts.token
    .lens::<Address, { TOKEN_ACCOUNT_MINT_OFFSET }>()?
    .get();
```

Built-in lens values include integers, `[u8; N]`, and `Address`. Integers are read as little-endian values.

## Proof Tokens

External adapters can add proof-carrying validation with `ExternalProof` and
`ExternalChecked`. A proof is a focused validation result that downstream APIs
can require instead of accepting any adapter-checked account.

```rust
let checked = ctx.accounts.oracle.checked::<FreshPythPrice>()?;
risk_engine.update_price(checked.proof())?;
```

Adapters can also expose dynamic proof helpers when the expected value is an
instruction argument or another account key. The SPL Token adapter does this for
mint, authority, and decimals checks.

## Owner-Selected Resolve

Use `ExternalResolve` for multi-owner or multi-format external account families:

```rust
match ctx.accounts.oracle.resolve()? {
    OraclePrice::Pyth(price) => price.checked_price()?,
    OraclePrice::Switchboard(feed) => feed.checked_price()?,
    OraclePrice::Custom(custom) => custom.checked_price()?,
}
```

The resolver decides which adapter view to return after owner, discriminator, and custom checks.

## Remaining Accounts

Strict duplicate rejection is the default. Typed parsing can make the policy explicit and then consume protocol-shaped groups:

```rust
let mut remaining = ctx.remaining_typed().no_duplicates()?;

let mut oracle_group = remaining.take_group(oracle_count as usize)?;
let oracles = oracle_group.parse_external::<OraclePrice, 32>()?;

let signer = remaining.next_signer()?;
remaining.assert_empty()?;
```

For branch-dependent instructions, use lazy indexed access. Only the selected slot is validated as an external account:

```rust
let oracle = ctx.remaining_lazy()
    .at(oracle_index as usize)?
    .external::<PythPrice>()?
    .view()?;
```

## Snapshots

External accounts often participate in CPI or oracle-consistency checks. Snapshot helpers hash the validated external bytes with Hopper's runtime SHA-256 helper:

```rust
let before = ctx.accounts.oracle.snapshot_hash()?;
invoke_checked(&ix, &accounts)?;
ctx.accounts.oracle.assert_snapshot(&before)?;
```

Or scope the check around a closure:

```rust
ctx.accounts.oracle.assert_unchanged_after(|| {
    // CPI or state transition
    Ok(())
})?;
```

## SPL Token External Adapters

Hopper ships first-party SPL Token external adapters for the base Token account
and Mint layouts. They validate the SPL Token owner, borrow the bytes directly,
and expose no-alloc accessors.

```rust
use hopper::prelude::*;

#[derive(Accounts)]
pub struct TokenFlow<'info> {
    pub source: ExternalAccount<'info, hopper::token::SplTokenAccount>,
    pub destination: ExternalAccount<'info, hopper::token::SplTokenAccount>,
    pub mint: ExternalAccount<'info, hopper::token::SplMint>,
}

pub fn checked_token_flow(ctx: Ctx<TokenFlow>, amount: u64) -> ProgramResult {
    let source_before = ctx.accounts.source.amount_snapshot()?;
    let destination_before = ctx.accounts.destination.amount_snapshot()?;

    let mint = ctx.accounts.mint.view()?.decimals();
    ctx.accounts.source.checked_mint(ctx.accounts.mint.key())?;
    ctx.accounts.mint.checked_decimals(mint)?;

    // CPI goes here.

    ctx.accounts.source.assert_amount_delta(source_before, -(amount as i128))?;
    ctx.accounts.destination.assert_amount_delta(destination_before, amount as i128)
}
```

The exposed types are:

- `hopper::token::SplTokenAccount` / `SplTokenAccountView`.
- `hopper::token::SplMint` / `SplMintView`.
- `CheckedTokenMint`, `CheckedTokenAuthority`, `CheckedMintDecimals`.
- `TokenAmountSnapshot`.

This is the first concrete adapter crate surface. Token-2022 TLV, oracle,
governance, loader, and Drift-compatible adapters can follow the same pattern.

## Explain Hooks

External adapters can implement `ExplainExternal` and emit structured fields to
an `ExternalExplainSink`. The runtime does not prescribe the final renderer;
CLI and SVM explain tools can choose to show decoded values, redact unknown
data, or hash fields.

```rust
impl ExplainExternal for MyOracle {
    fn explain<S: ExternalExplainSink>(account: &AccountView, sink: &mut S) -> ProgramResult {
        let oracle = ExternalAccount::<MyOracle>::try_new(account)?;
        oracle.with_view(|view| {
            sink.field_str("adapter", "MyOracle")?;
            sink.field_i64("price", view.price()?)?;
            sink.field_u64("slot", view.published_slot()?)
        })
    }
}
```

## Large Zero-Copy Accounts

For Hopper-owned large accounts, use pre-created accounts plus `#[account(zero)]` / initialization helpers where the account is too large for normal CPI allocation. `InitAccount<'info, T>::load_init()` is available as an Anchor-compatible alias for `load_after_init()`.

For known foreign large accounts, use `ExternalAccount<T>` plus a view type that owns the `Ref<'a, [u8]>` guard and exposes bounds-checked accessors. Do not cast packed foreign bytes to aligned Rust references unless the adapter can prove the alignment contract.

## Current Implementation Surface

Implemented now:

- `ExternalZeroCopy::View<'a>`.
- `ExternalAccount::view()` and `with_view()`.
- `ExternalResolve`.
- Checked external lenses.
- `RemainingTyped::no_duplicates()`.
- `RemainingTyped::take_group()` and `RemainingGroup`.
- `RemainingLazy::at()` for branch-dependent validation.
- `ExternalAccount::snapshot_hash()`, `assert_snapshot()`, and `assert_unchanged_after()`.
- `ExternalProof`, `ExternalChecked`, `ExplainExternal`, and `ExternalExplainSink`.
- SPL Token external account and mint adapters with proof tokens and amount delta checks.
- `StoredInstruction` and explicit checked CPI aliases.

Roadmap items still intentionally separate from this core layer:

- Adapter crates for Token-2022 TLV, oracles, governance, loader, and Drift-compatible accounts.
- Proc-macro sugar for `#[account(lazy)]`, `#[remaining(group = ...)]`, and proof-carrying external constraints.
- SVM explain adapter registry and redacted unknown-account output.
- Token CPI postcondition macros over the current `TokenAmountSnapshot` helpers.
