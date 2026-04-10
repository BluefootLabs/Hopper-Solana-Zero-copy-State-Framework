# On-Chain Schema Publication

How Hopper programs publish their schema on-chain so that Hopper Manager
and other tools can discover and decode them by program address.

## Problem

Without on-chain schema, tools must obtain manifests out-of-band (via
Git repos, package registries, or JSON files). On-chain publication
enables **program address → schema** lookup.

## Design

### Truth Hierarchy

```text
ProgramManifest    ← rich internal truth (local / version control)
ProgramIdl         ← public schema (published or stored off-chain)
CodamaProjection   ← interop subset (for Codama/Kinobi tooling)
HopperSchemaPointer ← on-chain pointer to the above
```

The on-chain account stores **hashes** and **URIs**, not the full manifest.
This keeps account size small and avoids rent-exempt bloat.

### HopperSchemaPointer Account

```rust
#[repr(C)]
pub struct HopperSchemaPointer {
    // -- Hopper header (16 bytes) --
    // disc: 255 (reserved discriminator for schema pointers)
    // version: 1
    // flags: 0x0001 (INITIALIZED)
    // layout_id: sha256("hopper:v1:SchemaPointer:1:...")[..8]

    // -- Payload --
    pub schema_version: u16,      // Schema format version (1)
    pub pointer_flags: u16,       // Feature flags (see below)
    pub manifest_hash: [u8; 32],  // SHA-256 of hopper.manifest.json
    pub idl_hash: [u8; 32],       // SHA-256 of hopper.idl.json
    pub codama_hash: [u8; 32],    // SHA-256 of hopper.codama.json
    pub uri_len: u16,             // Length of the URI string
    pub uri: [u8; 192],           // UTF-8 URI to the manifest
    // Total payload: 2 + 2 + 32 + 32 + 32 + 2 + 192 = 294 bytes
    // Total with header: 294 + 16 = 310 bytes
}
```

### Pointer Flags

| Bit | Name | Description |
|-----|------|-------------|
| 0 | `HAS_MANIFEST` | `manifest_hash` is populated |
| 1 | `HAS_IDL` | `idl_hash` is populated |
| 2 | `HAS_CODAMA` | `codama_hash` is populated |
| 3 | `HAS_URI` | `uri` contains a valid URI |
| 4 | `URI_IS_IPFS` | URI points to IPFS (content-addressed) |
| 5 | `URI_IS_ARWEAVE` | URI points to Arweave (permanent storage) |

### Account Address Derivation

The schema pointer account is at a deterministic PDA:

```
seeds = ["hopper-schema", program_id]
```

This makes discovery trivial: given a program ID, derive the PDA and
read the schema pointer.

### URI Strategies

| Strategy | URI Format | Pros | Cons |
|----------|-----------|------|------|
| IPFS | `ipfs://Qm...` | Content-addressed, cheap | Needs pinning |
| Arweave | `ar://...` | Permanent, no pinning | Costs AR |
| HTTPS | `https://...` | Simple | Mutable, trust required |
| On-chain | `hopper://account/...` | Fully on-chain | Expensive rent |

**Recommended**: IPFS for manifests (content-addressed, hash-verifiable),
with the SHA-256 hash on-chain for integrity verification.

### Workflow

1. **Build**: `hopper schema export --manifest @manifest.json > hopper.manifest.json`
2. **Hash**: `sha256sum hopper.manifest.json`
3. **Upload**: Pin to IPFS, Arweave, or host via HTTPS
4. **Publish**: `hopper publish --program <PROGRAM_ID> --manifest-hash <HASH> --uri <URI>`
5. **Discover**: `hopper manager summary --address <PROGRAM_ID>` fetches schema pointer,
   downloads manifest, verifies hash, decodes program.

### Manager Integration

Hopper Manager currently accepts `@manifest.json` for local files. With
on-chain schema publication, it can also accept `--address <PROGRAM_ID>`:

```bash
# Local (current)
hopper manager summary @manifest.json

# On-chain
hopper manager summary --address <PROGRAM_ID>
```

The flow:
1. Derive PDA from program ID
2. Fetch `HopperSchemaPointer` account data
3. Extract URI and manifest hash
4. Fetch manifest from URI
5. Verify SHA-256 matches on-chain hash
6. Deserialize and use as `ProgramManifest`

## Security Considerations

- **Hash verification**: Always compare the fetched manifest's SHA-256
  against the on-chain `manifest_hash`. Reject mismatches.
- **Authority**: Only the program's upgrade authority (or a designated
  schema authority) should be able to update the schema pointer.
- **Immutability**: Consider making the schema pointer non-closeable
  to prevent denial-of-service via account deletion.

## Implementation Status

- [x] `HopperSchemaPointer` type defined in `hopper-schema`
- [x] Layout with `hopper_layout!` macro
- [x] Spec document (this file)
- [ ] PDA derivation helper
- [ ] CLI `hopper publish` command
- [ ] Manager `--address` flag for on-chain lookup
- [ ] IPFS upload integration
