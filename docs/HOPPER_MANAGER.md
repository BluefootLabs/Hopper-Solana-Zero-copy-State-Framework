# Hopper Manager

## Current Model

Hopper Manager is the program-level inspection surface built into
`hopper-cli`. Today it is manifest-centric: point it at a manifest on disk.
`hopper fetch <program-id>` / `hopper manager fetch <program-id>` can read only
the legacy `MANIFEST_SEED` PDA when an application has provisioned one with its
own publisher; Hopper ships no generic publisher for that PDA.

That makes Manager the operational layer of Hopper's state-contract model,
not a separate product bolted on after the fact.

## What the Manager Does

Given a `ProgramManifest`, Hopper Manager can:

1. **Summarize** a program's layouts, instructions, policies, events, and contexts.
2. **Identify** a headered account's layout from supplied raw bytes.
3. **Decode** every field of a matching headered account with type-aware formatting.
4. **Inspect** layout fingerprints, segment metadata, and receipt payloads.
5. **Compare** before/after account states against manifest metadata.
6. **Simulate** instruction requirements from manifest-declared accounts, args, and policies.
7. **Explore interactively** through the TUI.

## CLI Command Reference

`hopper manager` is a subcommand family:

```bash
hopper manager summary <manifest>
hopper manager identify <manifest> <hex>
hopper manager decode <manifest> <hex>
hopper manager instruction <manifest> <tag|name>
hopper manager layouts <manifest>
hopper manager policies <manifest>
hopper manager events <manifest>
hopper manager fingerprints <manifest>
hopper manager compat <manifest> <hex-old> <hex-new>
hopper manager receipt <hex-64-or-72-bytes>
hopper manager explain <manifest>
hopper manager diff <manifest> <hex-before> <hex-after>
hopper manager simulate <manifest> <instruction>
hopper manager fetch <program-id> [--rpc <url>]
hopper manager interactive <manifest>
```

Related commands outside the `manager` family:

```bash
hopper fetch <program-id> [--rpc <url>] [--json]
hopper explain context <manifest> [--type <ContextName>]
hopper interactive <manifest>
```

Manifest arguments accept inline JSON or `@path/to/file.json`. The current
`identify`, `decode`, and byte-diff paths require Hopper's 16-byte header and do
not identify compact `[disc][body]` accounts; use a generated compact client or
the compact manifest offsets for those bytes.

## Typical Workflows

### Start from a local manifest

```bash
hopper manager summary @hopper.manifest.json
hopper manager layouts @hopper.manifest.json
hopper manager instruction @hopper.manifest.json deposit
```

### Decode supplied account bytes against a known schema

```bash
hopper manager identify @hopper.manifest.json <hex-data>
hopper manager decode @hopper.manifest.json <hex-data>
hopper manager diff @hopper.manifest.json <hex-before> <hex-after>
```

### Read an application-provisioned legacy manifest PDA

```bash
hopper fetch <program-id>
hopper manager fetch <program-id>
```

These commands do not read Program Metadata IDL records and do not imply that
the program published a legacy manifest PDA. Start from a checked-in manifest
unless application-specific tooling created that PDA.

### Explore interactively

```bash
hopper manager interactive @hopper.manifest.json
hopper explain context @hopper.manifest.json
```

## Illustrative output: Account Identification

The fingerprints in this example are placeholders, not generated evidence.

```
=== Account Identification ===
  Data size    : 57 bytes
  Header disc  : 1
  Header ver   : 1
  Layout ID    : a1b2c3d4e5f60718

  MATCH: Vault v1
  Expected size: 57 bytes
  Fields       : 3

Use 'hopper manager decode' to see field values.
```

## Illustrative output: Program Summary

The fingerprints in this example are placeholders, not generated evidence.

```
Program: hopper_registry v0.3.0

Layouts (3):
  Vault      v1  disc=1  57 bytes   fingerprint=a1b2c3d4e5f60718
  VaultV2    v2  disc=1  73 bytes   fingerprint=b2c3d4e5f6071829
  Config     v1  disc=2  43 bytes   fingerprint=c3d4e5f607182930

Instructions (4):
  0  init_vault      accounts=3  caps=CreatesAccount
  1  deposit         accounts=2  caps=MutatesState,MutatesTreasury
  2  withdraw        accounts=2  caps=MutatesState,MutatesTreasury
  3  update_config   accounts=2  caps=MutatesState,ModifiesAuthority

Policies (2):
  TREASURY_WRITE      Authority + Snapshot + Conservation + Invariants
  AUTHORITY_CHANGE     Authority + CPI Guard + PostMutation + Invariants

Compatibility:
  Vault v1 -> v2: append-safe, backward-readable
```

## Schema Requirements

Hopper Manager depends on:

- **Hopper Manifest** for layout definitions and instruction metadata
- **hopper-schema** types for decoding, compatibility, and migration
- **Field maps + layout info** for manager-readable runtime metadata

The manifest is the bridge between raw bytes and meaningful state.
Without a manifest, Hopper can still do header-level analysis with
`hopper inspect` and `hopper explain`. With a manifest, Manager delivers
field-level decoding, instruction metadata, policies, contexts, and
semantic program summaries.

## Architecture

```
                    ┌─────────────────────┐
                    │   hopper manager    │
                    │  subcommand family  │
                    └──────────┬──────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
    ┌─────────v─────┐ ┌───────v───────┐ ┌──────v──────┐
    │  Manifest     │ │  Account      │ │  Receipt    │
    │  Loader       │ │  Decoder      │ │  Decoder    │
    └─────────┬─────┘ └───────┬───────┘ └──────┬──────┘
              │                │                │
    ┌─────────v─────────────────v────────────────v──────┐
    │              hopper-schema                        │
    │  ProgramManifest, ManagerMetadata, decode_*      │
    └──────────────────────────────────────────────────┘
```

The Manager is a CLI subcommand family that coordinates existing schema
primitives into a unified operational interface. It adds no new
dependencies and reuses the same decode, compare, explain, and clientgen
metadata that power the rest of the Hopper CLI.

## Value Proposition

Hopper Manager makes Hopper:

- **Easier to inspect than raw cast frameworks** -- load a program, see everything
- **More operable than hex-dump tooling** -- structured decode, not guesswork
- **More production-friendly than schema-light frameworks** -- compatibility and receipts are built in
- **More coherent than fragmented toolchains** -- one manifest-driven surface for the whole lifecycle

When a protocol team can run `hopper manager` and instantly understand
every layout, every instruction, every policy, and every receipt in
their program, Hopper becomes operationally legible in a way most
Solana frameworks still are not.
