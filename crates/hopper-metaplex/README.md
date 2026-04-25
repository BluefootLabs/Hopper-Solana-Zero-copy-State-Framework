# hopper-metaplex

Hopper-owned Metaplex Token Metadata builders, PDA helpers, and a stack-buffer
Borsh encoder. Powers the `metadata::*` / `master_edition::*` field keywords on
`#[hopper::context]` and the [`hopper-nft-mint`](../../examples/hopper-nft-mint/src/lib.rs)
reference program.

[![Crates.io](https://img.shields.io/crates/v/hopper-metaplex.svg)](https://crates.io/crates/hopper-metaplex)
[![Docs.rs](https://img.shields.io/docsrs/hopper-metaplex)](https://docs.rs/hopper-metaplex)

Part of the **[Hopper](https://hopperzero.dev)** framework.

## When to reach for this

Anything that mints, updates, or reads NFT metadata on Solana through the
Metaplex Token Metadata program. The crate ships the three calls every NFT
program reaches for:

- `CreateMetadataAccountV3` — initialise the metadata PDA for a mint.
- `CreateMasterEditionV3` — lock a mint as a master edition (set
  `max_supply = Some(0)` for a 1-of-1 NFT).
- `UpdateMetadataAccountV2` — mutate an existing metadata account.

Each builder uses a stack buffer to encode the Borsh payload — no heap, no
`Vec`, no `alloc::String`. Buffer overflow returns
`ProgramError::InvalidInstructionData` so a malicious oversized name can't
push the program into UB.

## Quick start

```toml
[dependencies]
hopper = { version = "0.1", features = ["metaplex"] }
```

```rust
use hopper::prelude::*;

CreateMetadataAccountV3 {
    metadata,
    mint,
    mint_authority: authority,
    payer: authority,
    update_authority: authority,
    system_program,
    rent: None,
    data: DataV2::simple("Boobies #001", "BOOB", "https://...", 500),
    is_mutable: true,
}
.invoke()?;

CreateMasterEditionV3 {
    edition: master_edition,
    mint,
    update_authority: authority,
    mint_authority: authority,
    payer: authority,
    metadata,
    token_program,
    system_program,
    rent: None,
    max_supply: Some(0), // 1-of-1 NFT
}
.invoke()?;
```

PDA helpers:

```rust
let (metadata, _bump) = hopper_metaplex::metadata_pda(&mint_address);
let (master_edition, _) = hopper_metaplex::master_edition_pda(&mint_address);
```

## Optional, opt-in

The crate is gated behind the `metaplex` feature on the root `hopper` crate.
Programs that don't touch Metaplex get no extra compile time and no extra
dependencies pulled in. Enable with:

```toml
hopper = { version = "0.1", features = ["metaplex"] }
```

## What's not (yet) shipped

- Bubblegum compressed NFTs.
- pNFT (programmable NFT) lifecycle.
- Edition prints (`MintNewEditionFromMasterEditionViaToken`).
- Collection verification (`VerifyCollection`, `SetAndVerifyCollection`).

Each of these is mechanical given the existing `BorshTape` encoder. Open an
issue if you want one prioritised.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
