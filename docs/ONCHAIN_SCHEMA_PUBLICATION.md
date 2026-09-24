# Schema and effect publication

Status: shipped IDL and custom-seed manifest publication; standardized effect
publication and the schema-pointer lifecycle remain design work. Reverified
2026-09-24.

## Shipped publication workflows

`hopper publish-manifest` publishes the normalized Hopper manifest through
Program Metadata under the custom `hopper-manifest` seed. It supports readback
and authority-baseline tooling. The seed is a Hopper convention, not a reserved
standard. `hopper publish-idl` publishes the supported Solana IDL projection.
Neither publication alone binds the declaration to the deployed ELF; use
release verification for that check. These workflows do not implement the
proposed `HopperSchemaPointer` account format described below.

## What ships

`hopper publish-idl` first attempts a lossless projection of a Hopper manifest
into the current Solana IDL v0.1.0 shape, then publishes compatible JSON through
the official
[Program Metadata program](https://github.com/solana-program/program-metadata).
Projection is fail-closed. The current Cicada manifest is refused because its
u16-prefixed bounded `route_data` and `execute_intent` remaining-account
contract cannot be represented faithfully. The CLI supports fresh inline
publication, a larger fresh Allocate/Write/Initialize path, and overwrite
through SetData when the replacement fits one transaction. Large chunked
overwrite is refused.

The dated 2026-07-12 devnet receipt exercised the small fresh-inline path with
the then-current legacy projection. A separate maintainer readback fetched the
Program Metadata-owned account, decompressed the payload, parsed the IDL, and
matched its name, version, and eight instruction names. That proves the dated
payload, transport, and readback; it does not attest today's rewritten Solana
IDL v0.1 projector or its representability checks. The large chunked and
overwrite paths were unit-tested but not exercised in that receipt.

The published object is an **IDL projection**. It contains the representable
public instruction/account/type interface and Hopper's wire discriminators.
Hopper account bodies are marked with custom `hopper-zero-copy-v1`
serialization, so consumers need a Hopper-aware decoder rather than an Anchor
Borsh decoder. It is not the full Hopper manifest and does not publish
byte-range write authority, touch evidence, an Effect ABI v0.2 contract, proof
coverage, remaining-account effect grammar, or handler behavior.

## What does not ship

The following older design artifacts are not complete product surfaces:

- `HopperSchemaPointer` exists as a 310-byte schema data type, but Hopper does
  not ship its account lifecycle, authority policy, or generic publication
  transaction;
- a `hopper publish` command;
- `hopper manager summary --address <PROGRAM_ID>` discovery through that
  pointer;
- automatic IPFS/Arweave upload; or
- a standardized Program Metadata `behavior` or `effect` record.

Do not present the proposed `["hopper-schema", program_id]` PDA flow merely
because the wire type exists. The type is not a deployed discovery protocol.

## Planned release-bound effect record

Program Metadata documents `idl` and `security` records and permits custom
seeds. It does not document a standardized behavior/effect record, and a custom
seed is not a reserved global namespace.

A Hopper convention should reference, rather than duplicate, the ELF-embedded
release-interface commitment, the separately distributed manifest-projected
declaration, and the Effect ABI commitment. The ELF does not contain the
manifest or prove handler behavior, deployment identity, or artifact freshness.
At minimum the publication body must bind:

- schema/version of the publication envelope;
- program address, loader, ProgramData address where applicable, and deployed
  executable digest;
- Hopper manifest/effect-contract commitment;
- authority diff from the previous release: ranges, writable roles, lamport
  powers, CPI targets, discriminator/schema changes; and
- a proof-coverage certificate naming verifier versions, covered paths,
  unsupported paths/syscalls, and loop assumptions.

Publishing the record gives upgrade signers inspectable authority deltas. It
does not stop an authorized upgrade, authenticate an off-chain invocation
frame, or prove every handler correct.

## Current local workflow

```sh
# Generate and release-check the local declaration/artifact pair.
hopper compile --emit manifest --package <package> --out hopper.manifest.json --force
hopper verify --package <package> --strict --release

# Project and publish only the compatible IDL.
hopper publish-idl --help

# Publish the full manifest next to the program (seed "hopper-manifest"),
# and the security.txt record (seed "security"); read either back.
hopper publish-manifest --manifest hopper.manifest.json --program-id <id> --cluster devnet
hopper publish-security --file security.json --program-id <id> --cluster devnet
hopper publish-manifest --read --program-id <id> --cluster devnet

# Grillo's current workspace CLI uses the v0.1 caller-supplied evidence format.
grillo verify hopper.manifest.json bundle.json
```

`hopper publish-manifest` publishes the declaration; it does not publish the
effect record above. The ELF-embedded release-interface commitment, checked by
`hopper verify --release` and, against a deployed program, by
`--baseline-program`, is what binds that declaration to a binary. Until the
effect publication ships, distribute the exact manifest, ELF, hashes, and
attestation together. Do not infer Hopper adoption from the total number of
accounts owned by Program Metadata.
