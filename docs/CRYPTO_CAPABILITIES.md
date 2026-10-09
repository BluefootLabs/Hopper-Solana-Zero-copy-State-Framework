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
| BLAKE3 | `hopper::crypto::blake3`, `blake3_single` | `sol_blake3` | Binding shipped; native and runtime calls failed on public devnet on October 7, 2026 |
| Curve point validation | `hopper::crypto::curve_validate_point`, `curve25519_edwards_validate_point` | `sol_curve_validate_point` | Shipped |
| Ed25519 precompile | `check_ed25519_signature`, `check_ed25519_signature_at`, `check_ed25519_signer`, `check_ed25519_signer_at` | Ed25519 program + instructions sysvar | Shipped |
| Secp256k1 precompile | `check_secp256k1_instruction`, `check_secp256k1_instruction_at`, `check_secp256k1_instruction_at_cross_instruction`, `check_secp256k1_message_hash` | secp256k1 native program + instructions sysvar | Shipped |
| Secp256k1 recover | `secp256k1_recover`, `recover_ethereum_address` | `sol_secp256k1_recover` | Shipped on-chain |
| Merkle inclusion | `hopper_solana::crypto::merkle` | SHA-256 helper | Shipped |
| Curve group operations | `curve_group_add`, `curve_group_sub`, `curve_group_mul`, `curve_multiscalar_mul` | `sol_curve_group_op`, `sol_curve_multiscalar_mul` | Shipped behind `crypto-curve` |
| Poseidon | `poseidon_hashv`, `poseidon_hash`, `poseidon_bn254_x5` | `sol_poseidon` | Shipped behind `crypto-poseidon` |
| alt_bn128 / BN254 | `alt_bn128_add`, `alt_bn128_mul`, `alt_bn128_pairing`, compression helpers | `sol_alt_bn128_group_op`, `sol_alt_bn128_compression` | Shipped behind `crypto-bn254` |
| Big modular exponentiation | `big_mod_exp` | `sol_big_mod_exp` | Binding behind `crypto-big-mod-exp`; SIMD-0529 absent on devnet, October 7, 2026 |

Host tests that need real digest bytes should use a software hasher in the test
crate. Hopper's on-chain helpers stay dependency-light and route through the
active Solana backend.

A Rust binding and an accepted program deployment are not proof that a syscall
executes on a particular cluster. The October 7 BLAKE3 probe deployed after
overriding only the CLI's local feature selection, with transaction preflight
retained. Both API calls then finalized with `ProgramFailedToComplete` and an
unsupported-instruction log. The default function lab excludes BLAKE3; a
separate opt-in build exercises it in the local VM. SHA-256 and Keccak have
separate successful live probes.

## Hash helpers

```rust
use hopper::crypto::{keccak256, sha256};

let domain = b"hopper:v1:auth";
let user = user_key.as_ref();

let sha = sha256(&[domain, user])?;
let eth = keccak256(&[b"\x19Ethereum Signed Message:\n32", &sha])?;
let receipt_digest = sha256(&[b"receipt", &eth])?;
```

The unreleased runtime wrappers share the native boundary's 20,000-slice limit;
0.5.0 runtime wrappers allow 16. Excess slices are rejected, never silently
dropped. Stack, heap and compute budgets impose lower practical bounds. The
[function lab](../bench/function-lab/README.md) checks 0 through 64 slices on
compiled native and runtime paths. Host SHA-256 computes a digest; host Keccak
and BLAKE3 syscall stubs are not digest oracles.

## Ed25519 precompile checks

The strict checkers below differ from native/runtime `require_*_instruction`
helpers, which check only a processed sibling's program ID. See
[instruction inspection](INSTRUCTION_INTROSPECTION.md) for trace scope, scratch
capacity, and the authorization checks required when using those low-level APIs.

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
length must match the modulus length. Its operands and output are little-endian,
each operand is limited to 512 bytes, and the modulus must be odd and greater
than one. These are the [Solana SDK contracts](https://github.com/anza-xyz/solana-sdk/tree/master/big-mod-exp).

The function lab exercises named curve, Poseidon and BN254 vectors. This is
bounded test coverage, not exhaustive cryptographic validation. Modular
exponentiation is excluded from its default build: the public devnet feature
account was absent on October 7, and the pinned Agave 4.2.1 VM's implementation
returns a failure stub. A declared Rust API does not establish cluster support.
