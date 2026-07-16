# grillo-manifest

Parser and types for the **Hopper mutation manifest** (`hopper.manifest.json`):
the machine-readable contract listing, per instruction, exactly which byte
ranges of which accounts the program is authorized to write
(`strictWrites` + `writeRanges`, the same `&'static [WriteRange]` const the
runtime enforces at borrow acquisition).

This crate is the *contract* half of Grillo, the behavioural-verification
layer: it loads a manifest, resolves an instruction's `InstructionContract`
(account contracts, byte-range contracts, lamport permissions,
mutation-completeness), and computes a SHA-256 commitment over the
canonicalized contract so a verifier can pin the exact contract it verified
against.

The *verdict* half lives in [`grillo-verifier`](../grillo-verifier), which
takes pre/post account snapshots plus the instruction's emitted touch map and
proves `changed ⊆ acquired ⊆ authorized`.

Manifests are produced by `hopper emit-manifest` from the program source —
published == enforced, one const, three surfaces (authored `WRITE_RANGES`,
manifest `writeRanges`, installed runtime `WritePolicy`).

Not published to crates.io yet (`publish = false`); part of the Hopper
workspace.
