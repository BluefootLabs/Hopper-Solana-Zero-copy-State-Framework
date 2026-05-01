# hopper-schema

Schema export, ABI fingerprinting, and migration tooling for Hopper.

Part of the **[Hopper](https://hopperzero.dev)** framework.

This crate is how Hopper programs talk to the outside world. It takes your
layout definitions and produces manifests, IDLs, and Codama-compatible schema
that clients, CLIs, and explorers can consume. Also handles version diffing
and migration planning between layout versions.

`no_std`, `no_alloc`.

## What's in here

- **Layout manifests** - Full schema description of every account type, field, segment, and compatibility pair
- **Program manifests** - Top-level program metadata including instruction descriptors, events, policies, and all layouts
- **Manager metadata** - `SchemaExport` / `ManagerMetadata` bridge runtime layout identity + field maps into inspectable tooling metadata
- **IDL projection** - Public-facing IDL with instructions, accounts, events, and PDA seed hints
- **Codama projection** - Ecosystem-compatible format for Kinobi/Umi client generators
- **Schema diff** - Field-level diffing between layout versions (added, removed, resized, retyped)
- **Compatibility classification** - Identical, WireCompatible, AppendSafe, MigrationRequired, or Incompatible
- **Migration planner** - Step-by-step migration plans between layout versions, segment-role-aware
- **Client generation** - TypeScript, Kotlin (`org.sol4k`), Python, and Rust off-chain SDK generators from program manifests. All generated account decoders enforce the 8-byte `LAYOUT_ID` fingerprint before reading fields
- **Field intents** - Semantic annotations (Balance, Authority, Timestamp, Counter, etc.) for smarter tooling
- **Account decoding** - Header and field-level decode from raw bytes using manifest metadata

## Schema layering

```
ProgramManifest      Full truth (layouts, instructions, events, policies)
   |
   v
ProgramIdl           Public-facing (instructions, accounts, events, fingerprints)
   |
   v
CodamaProjection     Ecosystem interop (Codama-shaped for client generators)
```

Code is the source of truth. Schema is always derived, never hand-written.

## License

Apache-2.0
