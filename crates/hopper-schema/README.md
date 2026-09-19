# hopper-schema

Schema export, ABI fingerprinting, and migration tooling for Hopper.

Part of the **[Hopper](https://hopperzero.dev)** framework.

This crate is how Hopper programs talk to the outside world. It turns layout
definitions into manifests, IDLs, and Codama-compatible schema that clients,
CLIs, and explorers can consume. It also handles version diffing and migration
planning between layout versions.

`no_std`. Manifest types do not require `std`; client generators use `alloc`
for their output buffers and strings.

## What's in here

- **Layout manifests** - Account and field wire schema for each layout.
- **Program manifests** - Program-level layouts plus instructions, events, policies, segment-role metadata, and compatibility pairs.
- **Release-interface commitments** - A canonical, const-evaluable SHA-256 commitment used by `hopper::program_manifest!` and `hopper verify --release` to bind manifest-projected interface/effect declarations to compiled ELFs.
- **Manager metadata** - `SchemaExport` and `ManagerMetadata` bridge runtime layout identity into tooling metadata.
- **Solana IDL projection** - Current specification v0.1.0 with an explicit expected program address, exact Hopper wire discriminators, current account flags, PDA seed hints, and custom zero-copy account serialization. Export fails closed when Hopper's wire or account contract cannot be represented losslessly.
- **Codama projection** - Ecosystem-compatible format for Kinobi/Umi client generators.
- **Schema diff** - Field-level diffing between layout versions.
- **Compatibility classification** - Identical, WireCompatible, AppendSafe, MigrationRequired, or Incompatible.
- **Migration planner** - Segment-role-aware migration steps between layout versions.
- **Client generation** - TypeScript, Kotlin (`org.sol4k`), Python, Rust, Go, and C generators from program manifests.
- **Field intents** - Semantic annotations such as Balance, Authority, Timestamp, and Counter.
- **Account decoding** - Header and field-level decode from raw bytes using manifest metadata.

## Schema layering

```
ProgramManifest      Rich Hopper tooling declaration
   |-- ProgramIdl           Hopper public subset
   |-- CodamaProjection     Codama-shaped ecosystem interop
   `-- Solana IDL v0.1.0    Lossless-only external projection
```

Code is the release source of truth: release manifests should be generated from
declarations. The library and CLI still accept supplied JSON, which must be
parsed and validated rather than assumed to be generated or authentic.

The versioned release commitment covers the declared executable interface. It
does not prove handler behavior, deployment address, artifact freshness, or
ledger deployment identity. Release evidence binds those properties
separately.

The Solana projection describes Hopper instructions in the current ecosystem
IDL shape. It does not replace Hopper's account decoder: headered layouts carry
Hopper's offset-aware header, compact layouts use `[disc][body]`, and both are
marked with custom `hopper-zero-copy-v1` serialization instead of being
mislabeled as Anchor Borsh accounts. The
projection refuses unsupported bounded wire encodings, remaining-account
contracts, unresolved or unsafe PDA seeds, unresolved fixed addresses, and
ambiguous discriminator prefixes.

Docs: <https://docs.rs/crate/hopper-schema>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

Apache-2.0
