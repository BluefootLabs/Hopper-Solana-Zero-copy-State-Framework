# Hopper Schema Architecture

## One canonical schema model

Hopper has one source of truth for state and program semantics: the Rust
code. Layout macros, instruction declarations, event definitions, and
policy bindings are the authoritative definitions. From that single
source, Hopper generates two output formats:

1. **Hopper Manifest** -- rich internal schema for tooling
2. **Hopper IDL** -- lighter public schema for clients and integrations

This keeps one truth while supporting two audiences.

## Code-First Doctrine

Canonical truth lives in code. The authoritative declarations are:

- `hopper_layout!` -- fields, offsets, sizes, versions, fingerprints
- `hopper_segment!` / segmented layouts -- segment structure, roles
- `hopper_dispatch!` -- instruction set, discriminators
- `hopper_error!` -- error codes and variants
- Policy constants -- capability-requirement bindings
- `hopper_interface!` -- cross-program read-only views

These declarations produce compile-time constants (LAYOUT_ID, LEN, DISC,
VERSION) that are deterministic and verifiable. The schema layer reads
these constants to build manifests and IDLs without duplicating truth.

## Proc Macro Policy

No proc macros are required for correctness or core functionality.
Proc macros are allowed only for:

- Schema derivation (`#[derive(HopperSchema)]`)
- Manifest export
- IDL generation
- Optional boilerplate reduction

See [PROC_MACRO_POLICY.md](PROC_MACRO_POLICY.md) for the full doctrine.

## The Hopper Manifest

### Purpose

The Manifest is Hopper's rich internal schema. It powers:

- `hopper explain` / `hopper inspect`
- `hopper compat` / `hopper diff` / `hopper plan`
- `hopper manager` (program introspection)
- Receipt rendering and migration planning
- Docs generation and test tooling

### File format

`hopper.manifest.json`

### Structure

```json
{
  "format": "hopper-manifest",
  "version": 1,
  "program": {
    "name": "hopper_registry",
    "program_id": "...",
    "version": "0.1.0",
    "description": "Segmented registry example"
  },
  "layouts": [
    {
      "name": "Vault",
      "kind": "fixed",
      "version": 1,
      "discriminator": 1,
      "layout_id": "a1b2c3d4e5f60718",
      "size": 57,
      "header_size": 16,
      "fields": [
        { "name": "authority", "type": "[u8;32]", "size": 32, "offset": 16 },
        { "name": "balance", "type": "WireU64", "size": 8, "offset": 48 },
        { "name": "bump", "type": "u8", "size": 1, "offset": 56 }
      ],
      "segments": [],
      "compatibility": {
        "append_safe": true,
        "compatible_from": [1],
        "migration_required_from": []
      }
    }
  ],
  "instructions": [
    {
      "name": "deposit",
      "tag": 1,
      "args": [
        { "name": "amount", "type": "u64", "size": 8 }
      ],
      "accounts": [
        { "name": "depositor", "writable": false, "signer": true },
        { "name": "vault", "writable": true, "signer": false, "layout_ref": "Vault" }
      ],
      "capabilities": ["MutatesState", "MutatesTreasury"],
      "policy_pack": "TREASURY_WRITE",
      "receipt_expected": true
    }
  ],
  "events": [
    {
      "name": "DepositEvent",
      "tag": 1,
      "fields": [
        { "name": "authority", "type": "[u8;32]", "size": 32 },
        { "name": "amount", "type": "u64", "size": 8 }
      ]
    }
  ],
  "policies": [
    {
      "name": "TREASURY_WRITE",
      "capabilities": ["MutatesState", "MutatesTreasury"],
      "requirements": ["Authority", "StateSnapshot", "LamportConservation", "InvariantCheck"]
    }
  ],
  "compatibility": {
    "pairs": [
      {
        "from": "Vault@1",
        "to": "Vault@2",
        "toVersion": 2,
        "policy": "append-only",
        "backwardReadable": true
      }
    ]
  }
}
```

### Segment metadata in manifests

For segmented accounts, each segment entry includes:

| Field | Type | Meaning |
|-------|------|---------|
| name | string | Segment identifier |
| role | string | Core / Extension / Journal / Index / Cache / Audit / Shard |
| segment_id | hex | FNV-1a hash of segment name |
| layout_ref | string | Layout name for this segment |
| required | bool | Must be present in every account instance |
| append_only | bool | Only append operations allowed |
| rebuildable | bool | Can be reconstructed from other data |
| immutable | bool | Cannot be modified after init |

## The Hopper IDL

### Purpose

The IDL is the public-facing schema for:

- TypeScript client generation
- Kotlin/Swift client generation
- Block explorers
- External integrations
- Codama-compatible tooling

### File format

`hopper.idl.json`

### Structure

```json
{
  "format": "hopper-idl",
  "version": 1,
  "program": {
    "name": "hopper_registry",
    "program_id": "...",
    "version": "0.1.0"
  },
  "instructions": [
    {
      "name": "deposit",
      "discriminator": [1],
      "args": [{ "name": "amount", "type": "u64" }],
      "accounts": [
        { "name": "depositor", "writable": false, "signer": true },
        { "name": "vault", "writable": true, "signer": false }
      ]
    }
  ],
  "accounts": [
    {
      "name": "Vault",
      "discriminator": [1],
      "size": 57,
      "fields": [
        { "name": "authority", "type": "publicKey", "offset": 16 },
        { "name": "balance", "type": "u64", "offset": 48 },
        { "name": "bump", "type": "u8", "offset": 56 }
      ]
    }
  ],
  "events": [
    {
      "name": "DepositEvent",
      "discriminator": [1],
      "fields": [
        { "name": "authority", "type": "publicKey" },
        { "name": "amount", "type": "u64" }
      ]
    }
  ]
}
```

### What IDL excludes

- Migration planning data
- Trust profile internals
- Policy wiring details
- Receipt render metadata
- Unsafe invariant catalog
- Segment migration hints

These live in the Manifest only.

## Codama Compatibility

Hopper is Codama-compatible where it improves developer experience:

- Client generation
- Instruction/account metadata for explorers
- TypeScript ecosystem interop

Hopper does not flatten its richer state model to fit Codama. The
Manifest preserves full richness; the IDL exposes the clean public
subset; a Codama projection can be generated from the IDL.

```bash
hopper schema export --manifest    # Full manifest
hopper schema export --idl         # Public IDL
hopper schema export --codama      # Codama-compatible projection
```

## Generation Pipeline

```
Rust declarations (hopper_layout!, hopper_dispatch!, etc.)
    |
    v
Schema extraction (hopper-schema crate)
    |
    v
Hopper Manifest + Hopper IDL + Codama projection
    |
    v
CLI / Manager / Clients / Planner / Receipts
```

The extraction layer lives in `hopper-schema`. It reads LayoutManifest
constants generated by macros and assembles them into the output formats.
No runtime reflection. No dynamic discovery. Everything is compile-time
deterministic.

## File Layout

```
project/
  hopper.manifest.json    # Rich manifest (generated)
  hopper.idl.json         # Public IDL (generated)
  src/
    lib.rs                # Canonical code declarations
  docs/
    SCHEMA_ARCHITECTURE.md
    PROC_MACRO_POLICY.md
```
