# Cross-Program Read Example

Demonstrates Hopper's **interface pinning** pattern: reading another program's
account with zero crate dependencies, secured by deterministic layout fingerprints.

## Architecture

```text
cross-program-read/
|-- program-a/    Defines and owns a Vault account (hopper_layout!)
|-- program-b/    Reads Program A's Vault (hopper_interface!), no dependency on A
`-- runner/       Host-only devnet proof that deploy-time program IDs compose
```

## How It Works

1. **Program A** uses `hopper_layout!` to define `Vault` with fields
   `authority`, `balance`, and `bump`.

2. **Program B** uses `hopper_interface!` to declare an identical `Vault`
   struct with the same field spec. Because the SHA-256 hash of the field
   descriptors is deterministic, both structs produce the **same `LAYOUT_ID`**.

3. When Program B receives Program A's executable account and calls
   `Vault::load_cross_program(vault, program_a.address())`:
   - Verifies `vault.owner == program_a.address()`
   - Verifies `account.layout_id == VaultView::LAYOUT_ID`
   - Verifies `account.data.len() == VaultView::LEN`
   - Returns a `VerifiedAccount<Vault>` typed overlay

4. If Program A ever changes its Vault layout (adds/removes/reorders fields),
   the `LAYOUT_ID` changes and Program B's `load_foreign()` fails, preventing
   silent schema drift.

## Key Differences vs Import-Based Reads

| Approach | Coupling | Trust Model | Schema Drift Risk |
| --- | --- | --- | --- |
| Import Program A's crate | Compile-time | Full crate dependency | Caught at compile time |
| **hopper_interface! (this)** | **None** | **Runtime ABI proof** | **Caught at runtime via LAYOUT_ID** |
| Raw byte slicing | None | No safety | **Undetected** |

## Usage Patterns

### Basic Cross-Program Read

```rust
let verified = Vault::load_cross_program(vault, program_a.address())?;
let balance = verified.get().balance.get();
```

### With TrustProfile (configurable validation)

```rust
let profile = TrustProfile::strict(program_a.address(), &Vault::LAYOUT_ID, Vault::LEN);
let verified = Vault::load_with_profile(account, &profile)?;
```

### Multi-Owner (Token vs Token-2022)

```rust
let (verified, owner_idx) = Vault::load_foreign_multi(account, &[&OWNER_A, &OWNER_B])?;
```

### Fingerprint Pinning (explicit ABI contract)

```rust
// Pin to a known fingerprint -- fails at compile time if layout changes
hopper_assert_fingerprint!(Vault, [0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81]);
```

## Verify

```bash
cargo check -p hopper-xp-program-a
cargo check -p hopper-xp-program-b
cargo check -p hopper-xp-devnet-runner
hopper build --host -p hopper-xp-program-a
hopper build --host -p hopper-xp-program-b
```

## Devnet Proof

Build and deploy both programs, then run the host proof without changing your
configured Solana wallet or cluster:

```powershell
cargo build-sbf --manifest-path examples\cross-program-read\program-a\Cargo.toml
cargo build-sbf --manifest-path examples\cross-program-read\program-b\Cargo.toml

$keypair = 'C:\path\to\deployer.json'
solana --keypair $keypair --url devnet program deploy --program-id target\deploy\hopper_xp_program_a-keypair.json target\deploy\hopper_xp_program_a.so
solana --keypair $keypair --url devnet program deploy --program-id target\deploy\hopper_xp_program_b-keypair.json target\deploy\hopper_xp_program_b.so

cargo run -p hopper-xp-devnet-runner -- --keypair $keypair --program-a <PROGRAM_A_ID> --program-b <PROGRAM_B_ID> --rpc https://api.devnet.solana.com
```

The runner creates a fresh Program A Vault, deposits into it through Program A,
asks Program B to read the account with `load_cross_program`, asks Program B to
verify a minimum balance through `TrustProfile::strict`, then fetches the Vault
and checks owner, layout length, authority bytes, and balance locally.

Program B still has no on-chain crate dependency on Program A. Only the
host-only runner imports Program A so it can decode the proof account after the
transactions land.

Latest verified devnet deployment from this workspace:

```text
Program A Id: C4rvXfXHZgRsy4rtHhkHTuovonsbVQ3jE2kX8jzLPBvv
Program A ProgramData Address: BqGfuR58sLSK9q6GdgFu8tEAuF3QJ1b3o9ZhmBsUJmir
Program A Last Deployed In Slot: 463314426
Program A Data Length: 14184 bytes
Program A Deploy Signature: 5CxPNMxX5Tg2gv68zskLi3evaRQT1xygoNGKAsSZYjzKpQc9xQUGGSifKXXspZsEC56a1Z99HyUqpFBqCD9afCTH

Program B Id: Qv3BjA7RztC92zykoQLpSgNQEycsUXdcskV517HGBcc
Program B ProgramData Address: 7P7JGyyquH5mYfiCnyh5c2i9aNCEeZQqKtaVfULRGzjL
Program B Last Deployed In Slot: 463314437
Program B Data Length: 2800 bytes
Program B Deploy Signature: 3w1hBH78LumJ892HwMjmY1NL6v48edjeYqQWXo6T3i7mTt8C4y6EVqKvSGNnQQcyZuc1gWwzRhsu4qThdYTf7hcm
```

Latest verified cross-program run from this workspace:

```text
Vault: DRTFuwejtQrt3eArowrSZsFzbcEwRUVz66BTYF3H3KSz
program_a:init: 2eu1dSbBgPxGY71bMH5kiojdVx5x7VKpYNZdBVai3HhC23usVhb1w9UR47qEU8HGYB286YRVbdk6vbDXUDPnLTFd
program_a:deposit: 3GxRmWC1yiPVFiKwTAhmHfFEGjUuj8cFW588sf6ebNhVTfqm4HTCfNjpkoBjmSHZUgLBqg4bLPtposHKTiP8jC35
program_b:read: 5AJnLHutrYkQ1puP4M59dvmuYrfU69agZAGGHkTSePYK97nwA6KAghiTN4HSkk6W54bi9aKxHx7ASbvH8K8D9Tgf
program_b:min: 5dpo1SWLuHMuMtZmLDubAKUmoQgx72Ud5FF8HYauYYk52RZmqyPAAnuBUNntJ2mGZY7zMgGg8jyw3CmVuwQYWxm2
verified: owner=a46dbf23.., balance=42, layout=[a0, 6d, 32, 92, 39, d4, f8, fb]
```

## Manifest Path

This example is interface-first and does not ship a checked-in
`ProgramManifest` JSON for either program yet.

Canonical generation path:

1. publish Program A and Program B with on-chain Hopper manifests
2. fetch them with `hopper fetch <program-id>`
3. inspect Program A's layout and Program B's interface assumptions with
   `hopper manager` and `hopper explain`

## CLI Walkthrough

```bash
hopper build --host -p hopper-xp-program-a
hopper build --host -p hopper-xp-program-b
hopper test -p hopper-xp-program-a
hopper test -p hopper-xp-program-b
```

Use the devnet proof after every interface layout change.
