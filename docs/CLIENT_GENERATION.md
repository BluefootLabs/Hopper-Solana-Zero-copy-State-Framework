# Client Generation

> `hopper client gen --ts <manifest.json>` → TypeScript SDK  
> `hopper client gen --kt <manifest.json>` → Kotlin SDK (`org.sol4k`)

## Motivation

Anchor wins social adoption because people know how to get from
"program" to "client." Hopper needs a clean answer.

The client generator reads a Hopper `ProgramManifest` (or its
IDL/Codama projections) and emits a typed SDK that any frontend
developer can drop into their project.

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

## Usage

```bash
# Generate from manifest JSON
hopper client gen --ts @my-program.manifest.json

# Generate Kotlin client bundle
hopper client gen --kt @my-program.manifest.json

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
  └─ hopper-schema::clientgen::KtClientGen
       ├─ Types.kt
       ├─ Accounts.kt
       ├─ Instructions.kt
       └─ Events.kt
```

The generator operates on `ProgramManifest` directly. It does NOT
parse JSON. The CLI is responsible for loading the manifest; the
generator formats TypeScript and Kotlin output bundles.
