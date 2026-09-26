# Build your application

Start with what your users need: settle a trade, claim an allocation, or buy an
asset. Hopper supplies account declarations, zero-copy state access, validation,
and cross-program calls. Your program supplies the product rules.

This guide describes available building blocks, not turnkey audited products.
Evidence is specific to the linked example and version.

## Trading and settlement

**User outcome:** agree trade terms and release tokens when those terms are met.

[Cicada](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-cicada)
demonstrates token custody and intent settlement with explicit on-chain limits.
Its production route integration and independent audit remain unfinished.
[Segmented order storage](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-orderbook)
shows how to update orders and events in account-backed collections. It has no
collateral, matching engine, price sorting, or token settlement.

Use these to study state and settlement separately. You still design market
rules, supported assets, pricing, custody, cancellation, and recovery for your
application. The smaller
[escrow example](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-escrow)
demonstrates state transitions and closure only; it does not transfer tokens.

## Token claims and airdrops

**User outcome:** claim a reward, an allocation, or tokens as they vest.

Use typed accounts for campaign terms and claim records, token CPI helpers for
transfers, and
[vesting math](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/crates/hopper-vesting)
for time-based entitlement.
[Distribution math](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/crates/hopper-distribute)
calculates proportional shares and fees; it does not distribute tokens itself.

A complete claim program must authenticate the recipient, establish eligibility,
prevent repeated claims, validate the mint and token accounts, and release tokens
from funded custody. It also needs campaign administration and recovery rules.
Those checks belong on chain; a client or indexer is not a substitute.

Hopper does not currently present these helpers as a complete, devnet-validated
airdrop template. The byte-allowance example tracks application credits, not
token entitlements or custody.

## NFT and cNFT markets

**User outcome:** list an asset, make an offer, and exchange payment for ownership.

Use Hopper state for listings, offers, and purchase rules. The
[NFT mint reference](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-nft-mint)
demonstrates Token Metadata calls for a pre-created SPL mint. It is not a
marketplace. The shipped
[Metaplex helpers](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/crates/hopper-spl/hopper-metaplex)
cover metadata creation, metadata updates, and master-edition creation;
they do not cover Bubblegum or the programmable-NFT lifecycle.

For compressed NFTs, integrate the
[Metaplex Bubblegum v2 program](https://www.metaplex.com/docs/smart-contracts/bubblegum-v2).
cNFT operations use Merkle proofs; clients typically obtain asset data and
proofs through DAS infrastructure. Asset authorization and the transaction's
ownership/payment rules must still be enforced on chain.

A cNFT marketplace therefore requires additional Bubblegum CPI integration,
proof handling, asset validation, and atomic payment/transfer logic. Hopper
does not yet ship a Bubblegum adapter or a validated cNFT-marketplace example.
Do not assume all Token Metadata assets use the same transfer lifecycle.

## Start with a working program

The [SOL vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-vault)
teaches account initialization, authority checks, and real deposits and
withdrawals. The
[byte-allowance program](https://hopperzero.dev/docs/byte-allowance)
teaches delegated limits and writes restricted to selected usage fields.

The published 0.4.0 vault has a captured 19-transaction devnet run. A fresh
0.4.0 allowance run captured 40 finalized transactions with full expected
account-state checks. These runs do not validate an airdrop or NFT marketplace.
See the [release evidence](https://hopperzero.dev/docs/release-status) before
using a result as a deployment claim.
