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
$env:HOPPER_DEVNET='1'
cargo run -p hopper-devnet-audit --features devnet-client --bin devnet_audit -- --keypair C:\path\to\deployer.json --program-id <PROGRAM_ID> --rpc https://api.devnet.solana.com
```

The live runner is fail-closed. It requires `HOPPER_DEVNET=1`, checks the
cluster genesis hash and deployed program account, uses finalized commitment,
and polls every transaction and state read until finalized. Before the
positive instruction sequence it submits three expected failures and proves
the complete state account snapshot did not change:

- a signer that does not match the stored authority
- too few strict remaining-account signers
- a mutation with the state account marked read-only

A passing run emits one line prefixed with
`HOPPER_DEVNET_EVIDENCE_JSON=`. The JSON binds the cluster genesis and node
version, program id, public account ids, finalized signatures and slots,
pre/post account SHA-256 values, rollback results, and exact decoded state. It
never includes the keypair path or RPC URL. A custom RPC URL is always reported
as `redacted`.

`account_sha256` hashes this canonical byte sequence: lamports as little-endian
u64, owner pubkey, executable as one byte, rent epoch as little-endian u64,
data length as little-endian u64, then the complete account data.

Historical deployment record (not a fresh finalized attestation):

```text
Program Id: 4LPSXhMpx2DrFvMSHXRB3yaGmz7iKP4nKkfD92mAtAdT
Deploy Signature: 4KGzT5XH9KjtGH5JR4A2WfYRv6uWAdTYuJw5Qu2rACnsLNiP9wq1Zuu8ZUQWvpUWwKAQFV6CZmdv5UZBUQQcPDYM
Artifact Size: 30408 bytes
```

Historical audit record (superseded by the structured evidence format above):

```text
State: EAQdR2FjcEHuerPV4c2yhwc9Z8crtk6YRmeMtsuRntCV
verified: counter=1, substrate_passes=1, remaining_signer_checks=2, proof_checks=1, token_policy_checks=1, field_capability_checks=1, label=hopper-live, members=1
```
