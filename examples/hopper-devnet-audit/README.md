# Hopper Devnet Audit

A deployable audit program for checking Hopper capability paths on devnet.

It exercises:

- `#[hopper::account]` pretty dynamic fields: `String<'a, 32>` and `Vec<'a, Address, 8>`.
- Typed `#[derive(Accounts)]` contexts with `InitAccount`, `Account`, `Signer`, and `Program<System>`.
- Generated dynamic-tail helpers through `AuditStateAccountTailExt`.
- Strict and passthrough remaining-account parsing through `ctx.remaining_accounts()`.
- Segment leases through `AccountView::segment_mut` and `SegmentBorrowRegistry`.
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

If devnet airdrop is rate-limited, fund the deployer shown by:

```powershell
solana --keypair $keypair --url devnet address
```

Latest verified devnet deployment from this workspace:

```text
Program Id: 4LPSXhMpx2DrFvMSHXRB3yaGmz7iKP4nKkfD92mAtAdT
ProgramData Address: E6qeWd7GwZF13dY14BkaKLguKEPJkKmUqgoDxK4XSTzR
Authority: HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT
Last Deployed In Slot: 463314412
Data Length: 28664 bytes
Deploy Signature: 24PoRp6xziX7QAG5tygvhKTioySpq6d1YskKq5JuPHXukxSHqRpa6b7sBzJ83xqeUigCVCT2Z4Lkw9P9b6U6ZTz5
```

Run the host-only audit client against devnet:

```powershell
cargo run -p hopper-devnet-audit --features devnet-client --bin devnet_audit -- --keypair C:\path\to\deployer.json --program-id <PROGRAM_ID> --rpc https://api.devnet.solana.com
```

The client creates a fresh `AuditState` account with `AuditState::ALLOC_SPACE`, sends all seven instructions, then fetches and decodes the account. A passing run prints `verified` with `counter=1`, `substrate_passes=1`, `remaining_signer_checks=2`, `label=hopper-live`, and `members=1`.

Latest verified audit run from this workspace:

```text
State: CHm3CnZTwjiMY4AmRjEU2B6qLVR8CSSqwyqLpatSPHVy
initialize: 3dmbhLsKdTCRNxznHPhWUuMNNKk7Vh9QvMvGUXyfMLVY8kVv5dWS4a5Fu7yjjfpDwJZgCAaypa5bPLEpRsYmrL1S
rename: enPBxKfQRRbv67HYVKHvxLWCQczRpwKxW8cbjTndcWqkCU2KWZXMyK4nxPt7Gu4n3CcqYBLP6aJq2mfnRyMdvPx
add_member: 3ApDvyYyB4yDT7vP4MwAxxPwR4ZbdH132qGj3LSTyFZyNyw8sC8GrSHxy1dhBQhUpAWov5GJpWURd1h7vY4wyxY6
increment_segment: 5r6dozDCDoWK3epBaL4mTEfj1cBopfQYM9kQcrEmkx1TG7b78gPcWnmYGTz2St9D1jFpCXAb8biWmsEoC3znEFex
substrate_probe: 2GhoviwJqbydPz6ZJUmXgv4rDX9oimpGvg3KqmF9hGRL7VT4jo5y1NvfP47qGVDJijYVBRioW6ZxEiCVBu5sLqb1
remaining_signers: 5ZsADeEx7CuMTjgyqivv3agH9dn1QKvuUGfXj4s2P49PPe9x9wSnpbBZ6jbf6K8yJVW4gNBUn5Qnr8BWG8vUHLu6
audit: 4ur5XQMwBwaPd1RWyiWSGQknhmhaNqJg3sagbi7GVSuQ1Lz538EjvZuWZzi3cvfp9mSQdky2Buimd2y7bo9skQZt
verified: counter=1, substrate_passes=1, remaining_signer_checks=2, label=hopper-live, members=1
```
