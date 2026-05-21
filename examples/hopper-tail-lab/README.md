# Hopper Tail Lab

Devnet-ready example for Hopper's Quasar-style dynamic fields plus Hopper's explicit bare final tails.

This program exists to prove the public story in one place:

> Write like Quasar. Hopper verifies the bytes before it casts them.

## What It Exercises

- `#[hopper::account]` with pretty bounded dynamic fields: `String<'a, 32>` and `Vec<'a, Address, 4>`.
- Bare final tails: `TailStr<'a>` for UTF-8 note bodies and `TailBytes<'a>` for binary payloads.
- `#[derive(Accounts)]`, `Ctx<T>`, `Account<'info, T>`, `InitAccount<'info, T>`, `Signer<'info>`, and `Program<'info, System>`.
- Generated init helpers, `has_one` validation, dynamic-tail editors, raw-tail commits, layout fingerprints, and role metadata.
- SBF-compatible crate shape: `[lib] crate-type = ["cdylib", "lib"]`.

## Instructions

| Tag | Handler | Purpose |
|---|---|---|
| `0` | `init_note` | Create a note with bounded label/reviewers and final UTF-8 body. |
| `1` | `rewrite_note` | Update the label/body through the generated raw-tail editor. |
| `2` | `add_reviewer` | Preserve the raw body while editing the bounded reviewers list. |
| `3` | `init_blob` | Create a binary blob backed by `TailBytes<'a>`. |
| `4` | `write_blob` | Replace binary bytes while incrementing revision. |

## Local Checks

```powershell
cargo check -p hopper-tail-lab
cargo test -p hopper-tail-lab
cargo run -q -p hopper-cli -- solana-check --manifest-path examples/hopper-tail-lab/Cargo.toml
```

The repository SBF workflow also runs `hopper solana-check --all --build-sbf`, so this example stays deployable on devnet.