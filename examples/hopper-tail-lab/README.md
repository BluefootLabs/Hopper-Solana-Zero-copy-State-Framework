# Hopper Tail Lab

Devnet-ready example for Hopper's bounded dynamic fields plus Hopper's explicit bare final tails.

This program exists to prove the public story in one place:

> Declare bounded fields. Validate the account contract before typed access.

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
| `1` | `rewrite_note` | Replace the label and the body with the in-place setters (`set_label`, `set_body`). |
| `2` | `add_reviewer` | Push one reviewer in place; the raw body behind the list moves by 32 bytes and is never read. |
| `3` | `init_blob` | Create a binary blob backed by `TailBytes<'a>`. |
| `4` | `write_blob` | Replace binary bytes while incrementing revision. |

## Compute units

`tests/cu.rs` runs `add_reviewer` and `rewrite_note` under Mollusk against
`target/deploy/hopper_tail_lab.so` (or the stem `HOPPER_TAIL_LAB_ELF` names)
and prints both costs. With the in-place setters (2026-09-28) against the
editor path they replaced:

| Instruction | Editor (decode, edit, re-encode) | In place |
|---|---:|---:|
| `add_reviewer` (one address, 160-byte body behind it) | 1,504 CU | 628 CU |
| `rewrite_note` (label and body) | 1,147 CU | 1,016 CU |

The ELF went from 24,328 to 23,784 bytes.

## Local Checks

```powershell
cargo check -p hopper-tail-lab
cargo test -p hopper-tail-lab
cargo run -q -p hopper-cli -- solana-check --manifest-path examples/hopper-tail-lab/Cargo.toml
```

The repository SBF workflow also runs `hopper solana-check --all --build-sbf`, so this example stays deployable on devnet.