# Hopper Devnet Audit

A deployable audit program for checking Hopper capability paths on devnet.

It exercises:

- `#[hopper::account]` pretty dynamic fields: `String<'a, 32>` and `Vec<'a, Address, 8>`.
- Typed `#[derive(Accounts)]` contexts with `InitAccount`, `Account`, `Signer`, and `Program<System>`.
- Generated dynamic-tail helpers through `AuditStateAccountTailExt`.
- Strict and passthrough remaining-account parsing through `ctx.remaining_accounts()`.
- Segment leases through `AccountView::segment_mut` and `SegmentBorrowRegistry`.
- Proof-carrying account checks through `AccountView::proof()`.
- Token-2022 no-alloc extension policies through `ExtensionPolicy`.
- Field capability metadata through `FieldCapability`.
- `hopper::substrate::CuBudget` from the raw substrate export layer.

Instruction table:

| Id | Handler | Purpose |
| --- | --- | --- |
| 0 | `initialize(bump: u8)` | Creates `AuditState`, writes the fixed body, and initializes the compact tail. |
| 1 | `rename()` | Mutates the bounded dynamic label through the generated tail helper. |
| 2 | `add_member()` | Pushes the authority into the bounded dynamic member list with duplicate protection. |
| 3 | `increment_segment()` | Increments the `counter` field through a segment lease. |
| 4 | `substrate_probe()` | Runs a substrate compute-budget probe and records a fixed-body counter. |
| 5 | `audit()` | Checks owner-linked authority, non-empty label, and member membership. |
| 6 | `remaining_signers()` | Validates strict remaining-account signer parsing and passthrough length parity. |
| 7 | `proof_probe()` | Exercises signer, writable, owner, and layout proof chains. |
| 8 | `token_policy_probe()` | Builds a small Token-2022 TLV buffer and validates required/forbidden extensions. |
| 9 | `field_capability_probe()` | Confirms generated field offsets compose with field capability policy flags. |

Build for SBF:

```powershell
cargo build-sbf --manifest-path examples\hopper-devnet-audit\Cargo.toml
```

Verify the generated artifact before deploy:

```powershell
readelf -h target\deploy\hopper_devnet_audit.so | Select-String -Pattern 'OS/ABI|Entry point|Flags'
readelf -s target\deploy\hopper_devnet_audit.so | Select-String -Pattern 'entrypoint'
```

Expected signs are `OS/ABI: UNIX - System V`, a non-zero entry point, and a global `entrypoint` symbol.

Deploy to devnet without changing the configured Solana wallet or cluster:

```powershell
$keypair = 'C:\path\to\deployer.json'
solana --keypair $keypair --url devnet program deploy --program-id target\deploy\hopper_devnet_audit-keypair.json target\deploy\hopper_devnet_audit.so
```

If TPU writes fail with `30 write transactions failed`, close the buffer account printed by the CLI to recover lamports, then retry with `--use-rpc`:

```powershell
solana --keypair $keypair --url devnet program deploy --use-rpc --program-id target\deploy\hopper_devnet_audit-keypair.json target\deploy\hopper_devnet_audit.so
```

If devnet airdrop is rate-limited, fund the deployer shown by:

```powershell
solana --keypair $keypair --url devnet address
```

Run the host-only audit client against devnet:

```powershell
cargo run -p hopper-devnet-audit --features devnet-client --bin devnet_audit -- --keypair C:\path\to\deployer.json --program-id <PROGRAM_ID> --rpc https://api.devnet.solana.com
```

The client creates a fresh `AuditState` account with `AuditState::ALLOC_SPACE`, sends all ten instructions, then fetches and decodes the account. A passing run prints `verified` with `counter=1`, `substrate_passes=1`, `remaining_signer_checks=2`, `proof_checks=1`, `token_policy_checks=1`, `field_capability_checks=1`, `label=hopper-live`, and `members=1`.

Latest verified deployment from this workspace:

```text
Program Id: 4LPSXhMpx2DrFvMSHXRB3yaGmz7iKP4nKkfD92mAtAdT
Deploy Signature: 4KGzT5XH9KjtGH5JR4A2WfYRv6uWAdTYuJw5Qu2rACnsLNiP9wq1Zuu8ZUQWvpUWwKAQFV6CZmdv5UZBUQQcPDYM
Artifact Size: 30408 bytes
```

Latest verified audit run from this workspace:

```text
State: 9gQf48rtnX36me4xhkgvoVi9VqBX3C5d2T3qoqBGLjFR
verified: counter=1, substrate_passes=1, remaining_signer_checks=2, proof_checks=1, token_policy_checks=1, field_capability_checks=1, label=hopper-live, members=1
```
