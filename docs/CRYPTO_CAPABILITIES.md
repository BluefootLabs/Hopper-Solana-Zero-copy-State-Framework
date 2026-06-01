# Crypto capabilities

Hopper's crypto surface is intentionally Solana-shaped: small no-alloc wrappers
around runtime syscalls, plus strict helpers for native precompile instructions.
The goal is not to bundle a general-purpose crypto library into every program.
The goal is to make the cryptography Solana already verifies easier to use
without losing the exact bytes your protocol depends on.

## Current coverage

| Primitive | Hopper API | Solana mechanism | Status |
|---|---|---|---|
| SHA-256 | `hopper::crypto::sha256`, `sha256_single` | `sol_sha256` / backend hasher | Shipped |
| Keccak-256 | `hopper::crypto::keccak256`, `keccak256_single` | `sol_keccak256` / backend hasher | Shipped |
| BLAKE3 | `hopper::crypto::blake3`, `blake3_single` | `sol_blake3` | Shipped |
| Curve point validation | `hopper::crypto::curve_validate_point`, `curve25519_edwards_validate_point` | `sol_curve_validate_point` | Shipped |
| Ed25519 precompile | `check_ed25519_signature`, `check_ed25519_signature_at`, `check_ed25519_signer`, `check_ed25519_signer_at` | Ed25519 program + instructions sysvar | Shipped |
| Secp256k1 precompile | `check_secp256k1_instruction`, `check_secp256k1_instruction_at`, `check_secp256k1_instruction_at_cross_instruction`, `check_secp256k1_message_hash` | secp256k1 native program + instructions sysvar | Shipped |
| Secp256k1 recover | `secp256k1_recover`, `recover_ethereum_address` | `sol_secp256k1_recover` | Shipped on-chain |
| Merkle inclusion | `hopper_solana::crypto::merkle` | SHA-256 helper | Shipped |
| Curve group operations | `curve_group_add`, `curve_group_sub`, `curve_group_mul`, `curve_multiscalar_mul` | `sol_curve_group_op`, `sol_curve_multiscalar_mul` | Shipped behind `crypto-curve` |
| Poseidon | `poseidon_hashv`, `poseidon_hash`, `poseidon_bn254_x5` | `sol_poseidon` | Shipped behind `crypto-poseidon` |
| alt_bn128 / BN254 | `alt_bn128_add`, `alt_bn128_mul`, `alt_bn128_pairing`, compression helpers | `sol_alt_bn128_group_op`, `sol_alt_bn128_compression` | Shipped behind `crypto-bn254` |
| Big modular exponentiation | `big_mod_exp` | `sol_big_mod_exp` | Shipped behind `crypto-big-mod-exp` |

Host tests that need real digest bytes should use a software hasher in the test
crate. Hopper's on-chain helpers stay dependency-light and route through the
active Solana backend.

## Hash helpers

```rust
use hopper::crypto::{blake3, keccak256, sha256};

let domain = b"hopper:v1:auth";
let user = user_key.as_ref();

let sha = sha256(&[domain, user])?;
let eth = keccak256(&[b"\x19Ethereum Signed Message:\n32", &sha])?;
let compact = blake3(&[b"receipt", &eth])?;
```

Hopper rejects more than 16 hash segments instead of silently dropping tail
segments. That guard matters: hash APIs must never ignore bytes.

## Ed25519 precompile checks

```rust
use hopper::crypto::check_ed25519_signature_at;

check_ed25519_signature_at(
    instructions_sysvar_data,
    ed25519_ix_index,
    0,
    expected_signer,
    expected_message,
)?;
```

Hopper validates the Ed25519 program id, selected signature index, inline
signature/public-key/message indexes, and all referenced offset bounds before it
compares signer and message bytes. The runtime verifies the signature; Hopper
verifies that the verified payload is the payload your program intended.

## Secp256k1 precompile checks

```rust
use hopper::crypto::check_secp256k1_instruction_at;

check_secp256k1_instruction_at(
    instructions_sysvar_data,
    secp_ix_index,
    0,
    expected_eth_address,
    expected_message,
)?;
```

The strict checker only accepts signature, recovery id, Ethereum address, and
message bytes stored in the secp256k1 instruction itself. If a protocol
deliberately stores those bytes in another transaction instruction, use the
cross-instruction variant so that choice is visible in code review:

```rust
use hopper::crypto::check_secp256k1_instruction_at_cross_instruction;

check_secp256k1_instruction_at_cross_instruction(
    instructions_sysvar_data,
    secp_ix_index,
    signature_index,
    expected_eth_address,
    expected_message,
)?;
```

## Secp256k1 recover

```rust
use hopper::crypto::{recover_ethereum_address, secp256k1_recover};

let pubkey64 = secp256k1_recover(message_hash, recovery_id, signature)?;
let eth_address = recover_ethereum_address(message_hash, recovery_id, signature)?;
```

`secp256k1_recover` is a low-level primitive. Prefer the precompile checker
when the transaction can include a native secp256k1 verification instruction,
especially for batches. Reach for recovery when the protocol truly needs the
recovered key in-program.

## Feature-gated heavy crypto

The heavy-crypto surface is feature-gated so tiny programs do not pay for APIs
they never call:

| Feature | API family |
|---|---|
| `crypto-curve` | curve group add/sub/mul and multiscalar wrappers |
| `crypto-poseidon` | Poseidon hash wrappers |
| `crypto-bn254` | alt_bn128 add/mul/pairing wrappers |
| `crypto-big-mod-exp` | bounded big modular exponentiation wrappers |

The wrappers deliberately expose Solana's byte-level contracts. BN254 helpers
use the same operation ids and byte lengths as Solana's `solana-bn254` crate;
Poseidon accepts 1 to 12 32-byte inputs; curve multiplication passes the scalar
as the left operand and point as the right operand, matching Solana's syscall
ABI. Big modular exponentiation writes into a caller-provided output buffer whose
length must match the modulus length.

SBF tests remain the next hardening lane before treating the heavy crypto path as
fully exercised across validator versions.