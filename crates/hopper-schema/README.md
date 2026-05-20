# hopper-schema

Schema export, ABI fingerprinting, and migration tooling for Hopper.

Part of the **[Hopper](https://hopperzero.dev)** framework.

This crate is how Hopper programs talk to the outside world. It turns layout
definitions into manifests, IDLs, and Codama-compatible schema that clients,
CLIs, and explorers can consume. It also handles version diffing and migration
planning between layout versions.

`no_std`, `no_alloc`.

## What's in here

- **Layout manifests** - Full schema for every account type, field, segment, and compatibility pair.
- **Program manifests** - Program-level metadata: instructions, events, policies, and layouts.
- **Manager metadata** - `SchemaExport` and `ManagerMetadata` bridge runtime layout identity into tooling metadata.
- **IDL projection** - Public-facing IDL with instructions, accounts, events, and PDA seed hints.
- **Codama projection** - Ecosystem-compatible format for Kinobi/Umi client generators.
- **Schema diff** - Field-level diffing between layout versions.
- **Compatibility classification** - Identical, WireCompatible, AppendSafe, MigrationRequired, or Incompatible.
- **Migration planner** - Segment-role-aware migration steps between layout versions.
- **Client generation** - TypeScript, Kotlin (`org.sol4k`), Python, and Rust SDK generators from program manifests.
- **Field intents** - Semantic annotations such as Balance, Authority, Timestamp, and Counter.
- **Account decoding** - Header and field-level decode from raw bytes using manifest metadata.

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

Docs: <https://docs.rs/crate/hopper-schema/0.2.1>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

Apache-2.0
