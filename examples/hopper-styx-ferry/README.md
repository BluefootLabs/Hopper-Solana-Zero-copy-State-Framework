# Hopper Styx Ferry

Hopper port of the Styx ferry and messaging control plane. The sample keeps
Signal-style X3DH, Double Ratchet state, and XChaCha20-Poly1305 payload
encryption in the client protocol, while Hopper enforces the on-chain parts that
must be deterministic:

- prekey ownership and signed-prekey freshness through the Ed25519 precompile;
- bounded forward-secret message envelopes with monotonic ratchet counters;
- a Styx VSL proof envelope with the 513-byte v2 proof format and eight public
  inputs;
- Keccak-derived BN254 field domain separation tied to the deployed program id;
- a pinned verifier-program CPI boundary for the ZK proof.

The point of the example is framework parity: Hopper programs can use the same
Solana crypto and introspection building blocks that Pinocchio/Jiminy/Quasar
style programs rely on, while still writing the state model with Hopper accounts,
bounded vectors, and typed CPI.

## Instructions

| Tag | Handler | Purpose |
|---|---|---|
| `0` | `init_config` | Create the ferry config, derive the Styx VSL domain separator, and pin a verifier program. |
| `1` | `publish_prekey_bundle` | Publish a user prekey bundle after verifying the signed prekey with an Ed25519 precompile sibling instruction. |
| `2` | `refresh_prekey_bundle` | Rotate the one-time-prekey root and update the bounded prekey count. |
| `3` | `init_thread` | Create a sender-to-recipient ratchet thread. |
| `4` | `send_ratchet_message` | Emit a bounded ciphertext envelope and update monotonic ratchet state. |
| `5` | `submit_zk_ferry` | Validate the Styx proof envelope, check fee/domain public inputs, and CPI into the verifier. |

## Account Shape

```rust
#[account(discriminator = 73, version = 1)]
pub struct FerryConfig {
    pub authority: Address,
    pub verifier_program: Address,
    pub domain_separator: [u8; 32],
    pub base_fee_lamports: WireU64,
    pub message_count: WireU64,
    pub prekey_update_count: WireU64,
    pub zk_ferry_count: WireU64,
}
```

`PrekeyBundle` stores the owner identity key, signed prekey, signed-prekey
signature, and one-time-prekey root. `MessageThread` stores only ratchet routing
metadata and hashes; plaintext and ratchet secrets never touch chain state.

## VSL Proof Boundary

The sample follows the Styx proof envelope shape:

- byte `0`: proof version, currently `2`;
- bytes `1..257`: proof bytes used by the verifier;
- bytes `257..513`: eight 32-byte public inputs.

The public inputs are read as:

| Index | Meaning |
|---|---|
| `0` | root |
| `1` | nullifier |
| `2` | out0 commitment |
| `3` | out1 commitment |
| `4` | asset id |
| `5` | domain separator |
| `6` | fee tier id |
| `7` | base fee lamports |

Before invoking the verifier program, Hopper checks the domain separator against
the Keccak-derived config domain, matches the base fee, caps the fee tier, and
ensures the verifier account is the pinned program id.

## Local Checks

```powershell
cargo check -p hopper-styx-ferry
cargo run -q -p hopper-cli -- solana-check --manifest-path examples/hopper-styx-ferry/Cargo.toml
```

Build with the Solana SBF toolchain when you want to deploy the sample:

```powershell
cargo build-sbf -- -p hopper-styx-ferry
solana program deploy target/deploy/hopper_styx_ferry.so --url devnet
```