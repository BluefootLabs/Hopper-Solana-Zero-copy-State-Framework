# Client Generation

> - `hopper client gen --ts <manifest.json>` → TypeScript SDK
> - `hopper client gen --kt <manifest.json>` → Kotlin SDK (`org.sol4k`)
> - `hopper client gen --py <manifest.json>` → Python SDK (stdlib-only)
> - `hopper compile --emit rust-client <manifest.json>` → Rust off-chain SDK

## Motivation

Anchor wins social adoption because people know how to get from
"program" to "client." Hopper needs a clean answer.

The client generator reads a Hopper `ProgramManifest` and emits typed SDKs that
frontends, bots, scripts, and tests can drop into their project. All generated
account decoders assert the 8-byte `LAYOUT_ID` fingerprint before reading
fields, so stale clients fail closed instead of silently mis-decoding account
bytes.

## TypeScript Generation

### What it generates

```text
=== types.ts ===
=== accounts.ts ===
=== instructions.ts ===
=== events.ts ===
=== index.ts ===
```

The CLI prints a file-marked bundle to stdout. Each section can be split into
its own file, or consumed by a generation pipeline.

### Type mapping

| Hopper canonical type | TypeScript type | Encoding          |
|-----------------------|-----------------|-------------------|
| `u8`                  | `number`        | 1 byte LE         |
| `u16`                 | `number`        | 2 bytes LE        |
| `u32`                 | `number`        | 4 bytes LE        |
| `u64`                 | `bigint`        | 8 bytes LE        |
| `u128`                | `bigint`        | 16 bytes LE       |
| `i8`                  | `number`        | 1 byte LE signed  |
| `i16`                 | `number`        | 2 bytes LE signed |
| `i32`                 | `number`        | 4 bytes LE signed |
| `i64`                 | `bigint`        | 8 bytes LE signed |
| `bool`                | `boolean`       | 1 byte (0/1)      |
| `Pubkey`              | `PublicKey`     | 32 bytes          |
| `[u8; N]`             | `Uint8Array`    | N bytes            |

### Account decoders

Each account type gets:
- A TypeScript interface with typed fields
- A `decode<Name>(data: Buffer): <Name>` function
- A discriminator constant for account identification

### Instruction builders

Each instruction gets:
- An `<InstructionName>Args` interface for typed arguments
- An `<InstructionName>Accounts` interface for required accounts
- A `create<InstructionName>Instruction(args, accounts, programId)` function
  that returns a `TransactionInstruction`

### Events

Each event gets:
- A TypeScript interface with typed fields
- A `decode<Name>Event(data: Buffer): <Name>Event` function

## Kotlin Generation

Kotlin generation is implemented today. The generator emits a file-marked
bundle targeting `org.sol4k` primitives:

- `Types.kt` with header and discriminator helpers
- `Accounts.kt` with data classes and decoders
- `Instructions.kt` with typed argument/account classes and builders
- `Events.kt` with event data classes and decoders

Encoding and decoding use explicit little-endian `ByteBuffer` operations, so
the generated Kotlin stays close to Hopper's wire model.

## Python Generation

Python generation emits a single stdlib-only module:

- dataclasses for account layouts and events
- `decode` classmethods that verify layout identity first
- segment-aware field readers for partial account inspection
- `build_<instruction>` helpers that return raw instruction-data bytes

The generated module intentionally does not pick a Solana transport library.
Callers can pass the emitted bytes to `solders`, `solana-py`, or their own RPC
stack.

## Rust Client Generation

`hopper compile --emit rust-client` emits an off-chain Rust client using
Solana SDK types. It is separate from `hopper compile --emit rust`, which emits
the lowered Hopper runtime preview for auditing macro expansion, accessors, and
offset paths.

## Usage

```bash
# Generate from manifest JSON
hopper client gen --ts @my-program.manifest.json

# Generate Kotlin client bundle
hopper client gen --kt @my-program.manifest.json

# Generate Python client module
hopper client gen --py @my-program.manifest.json

# Generate off-chain Rust client
hopper compile --emit rust-client @my-program.manifest.json

# Generate from piped manifest
hopper schema export --manifest | hopper client gen --ts -
```

## Architecture

```text
ProgramManifest
  ├─ hopper-schema::clientgen::TsClientGen
  │    ├─ types.ts
  │    ├─ accounts.ts
  │    ├─ instructions.ts
  │    ├─ events.ts
  │    └─ index.ts
  ├─ hopper-schema::clientgen::KtClientGen
  │    ├─ Types.kt
  │    ├─ Accounts.kt
  │    ├─ Instructions.kt
  │    └─ Events.kt
  ├─ hopper-schema::python_client::PyClientGen
  │    └─ <program>_client.py
  └─ hopper-schema::rust_client::RsClientGen
       └─ client.rs
```

The generator operates on `ProgramManifest` directly. It does NOT
parse JSON. The CLI is responsible for loading the manifest; the
generator formats TypeScript, Kotlin, Python, and Rust output bundles.
