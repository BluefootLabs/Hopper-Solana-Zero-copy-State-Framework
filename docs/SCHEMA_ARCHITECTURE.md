# Hopper Schema Architecture

## One canonical schema model

Hopper has one source of truth for state and program semantics: the Rust
code. Layout macros, instruction declarations, event definitions, and
policy bindings are the authoritative definitions. From that single source,
Hopper generates two Hopper-native schema layers:

1. **Hopper Manifest** -- rich internal schema for tooling
2. **Hopper IDL** -- lighter public schema for clients and integrations

It also derives Codama JSON, a fail-closed Solana IDL v0.1 projection when the
wire contract is losslessly representable, and six SDK targets: TypeScript,
Kotlin, Python, Go, C, and off-chain Rust. This keeps one truth while serving
different consumers without calling every projection the same IDL.

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

Current `hopper compile --emit schema` output is top-level manifest JSON. A
trimmed compact-layout example is:

```json
{
  "name": "hopper_compact_vault",
  "version": "0.3.0",
  "description": "Compact vault example",
  "layouts": [
    {
      "name": "Vault",
      "disc": 1,
      "version": 1,
      "layoutId": "437141907c09344f",
      "totalSize": 41,
      "hasDynamicTail": false,
      "fieldCount": 2,
      "fields": [
        { "name": "authority", "type": "Pubkey", "size": 32, "offset": 1, "intent": "custom" },
        { "name": "balance", "type": "u64", "size": 8, "offset": 33, "intent": "custom" }
      ],
      "semanticFingerprint": "633512a09da83eb5"
    }
  ],
  "instructions": [
    {
      "name": "deposit",
      "tag": 1,
      "discriminatorBytes": [1],
      "args": [
        { "name": "amount", "type": "u64", "size": 8, "encoding": "fixed" }
      ],
      "accounts": [
        { "name": "vault", "writable": true, "signer": false, "layoutRef": "Vault" },
        { "name": "authority", "writable": false, "signer": true }
      ],
      "capabilities": ["MutatesState"],
      "policyPack": "COMPACT_VAULT_WRITE",
      "receiptExpected": false,
      "strictWrites": false,
      "writeRanges": [],
      "parametricWriteRanges": []
    }
  ],
  "events": [],
  "policies": [
    {
      "name": "COMPACT_VAULT_WRITE",
      "capabilities": ["CreatesAccount", "MutatesState"],
      "requirements": ["SignerAuthority", "ExactAccountSize"],
      "invariants": [],
      "receiptProfile": ""
    }
  ],
  "layoutMetadata": [],
  "contexts": [],
  "compatRules": [],
  "toolingHints": ["account_encoding=compact", "compact_body_offset=1"]
}
```

The full normalized manifest also emits the current receipt schema. Fields are
omitted above only to keep the example readable.

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

Hopper's public IDL is a Hopper-native, lighter projection for:

- TypeScript client generation
- Kotlin, Python, Go, C, and Rust client generation
- Block explorers
- External integrations
- external tooling that consumes Hopper's own schema

It is not the Solana Foundation IDL v0.1 projection used by
`hopper schema export --anchor-idl` and `hopper publish-idl`, and it is not the
Codama-shaped projection. All three derive from the same manifest.

### File format

`hopper.idl.json`

### Structure

```json
{
  "name": "hopper_compact_vault",
  "version": "0.3.0",
  "description": "Compact vault example",
  "instructions": [
    {
      "name": "deposit",
      "tag": 1,
      "args": [{ "name": "amount", "type": "u64", "size": 8, "encoding": "fixed" }],
      "accounts": [
        { "name": "vault", "writable": true, "signer": false, "layoutRef": "Vault" },
        { "name": "authority", "writable": false, "signer": true }
      ]
    }
  ],
  "accounts": [
    {
      "name": "Vault",
      "disc": 1,
      "version": 1,
      "layoutId": "437141907c09344f",
      "totalSize": 41,
      "fieldCount": 2,
      "fields": [
        { "name": "authority", "type": "Pubkey", "size": 32, "offset": 1, "intent": "custom" },
        { "name": "balance", "type": "u64", "size": 8, "offset": 33, "intent": "custom" }
      ],
      "semanticFingerprint": "633512a09da83eb5"
    }
  ],
  "events": [],
  "fingerprints": [
    { "layoutId": "437141907c09344f", "name": "Vault" }
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

Hopper does not flatten its richer state model to fit Codama. The manifest
preserves full richness; Hopper public IDL and Codama-shaped JSON are separate
projections generated from that same manifest.

```bash
hopper schema export --manifest @hopper.manifest.json  # Normalized manifest
hopper schema export --idl @hopper.manifest.json       # Hopper public IDL
hopper schema export --codama @hopper.manifest.json    # Codama-shaped JSON
```

## Generation Pipeline

```
Rust declarations (hopper_layout!, hopper_dispatch!, etc.)
    |
    v
Schema extraction (hopper-schema crate)
    |
    v
Hopper Manifest (canonical generated contract)
    |
    v
Hopper public IDL + Codama JSON + conditional Solana IDL v0.1
    |
    v
TypeScript / Kotlin / Python / Go / C / off-chain Rust clients
    |
    v
CLI / Manager / Planner / Receipts / external integrations
```

The extraction layer lives in `hopper-schema`. It reads LayoutManifest
constants generated by macros and assembles them into these projections. The
Solana IDL branch fails closed when Hopper wire semantics cannot be represented
losslessly. No runtime reflection or dynamic discovery is involved.

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
