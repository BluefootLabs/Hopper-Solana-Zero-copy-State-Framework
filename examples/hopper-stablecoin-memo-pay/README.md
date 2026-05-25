# Hopper Stablecoin Memo Pay

Devnet-ready Hopper example for a stablecoin payment ledger. It proves the
payment path without asking Hopper to become a custody layer:

- A merchant-owned `StablecoinMerchant` account stores the accepted mint,
  cumulative settled amount, payment count, and latest 32-byte reference.
- `pay` verifies the payer and merchant token accounts against the mint and
  owner, then uses Hopper's polymorphic `interface_transfer_checked` helper.
- The same instruction emits a signed SPL Memo CPI through Hopper's `Memo`
  builder so indexers have a transaction-log reference without extra state.
- The transfer helper supports both SPL Token and Token-2022 accounts by
  selecting the program from the source token account owner.

## Instructions

| Tag | Handler | Purpose |
|---|---|---|
| `0` | `init_merchant` | Create the merchant ledger for one stablecoin mint. |
| `1` | `pay` | Transfer checked stablecoin units, memo the payment, and update totals. |

## Account Shape

```rust
#[account(discriminator = 72, version = 1)]
pub struct StablecoinMerchant {
    pub merchant: Address,
    pub stable_mint: Address,
    pub total_collected: WireU64,
    pub payment_count: WireU64,
    pub last_reference: [u8; 32],
}
```

`pay` keeps the audit checks explicit: amount must be non-zero, the memo must be
non-empty UTF-8 bytes, payer and merchant token accounts must be initialized,
both token accounts must use the configured mint, the payer token account must be
owned by the signing payer, the destination token account must be owned by the
merchant, and the mint decimals must match the caller-supplied value before CPI.

## Local Checks

```powershell
cargo check -p hopper-stablecoin-memo-pay
cargo test -p hopper-stablecoin-memo-pay
cargo run -q -p hopper-cli -- solana-check --manifest-path examples/hopper-stablecoin-memo-pay/Cargo.toml
```

## Devnet Shape

Build and deploy the program with the Solana SBF toolchain:

```powershell
cargo build-sbf -- -p hopper-stablecoin-memo-pay
solana program deploy target/deploy/hopper_stablecoin_memo_pay.so --url devnet
```

Use a devnet SPL or Token-2022 mint for the stablecoin stand-in. Production
deployments should pin an allowlisted mint and route client checkout sessions
through a backend that verifies the finalized transaction before granting goods.
