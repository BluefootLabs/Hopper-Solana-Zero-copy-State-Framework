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

The framework-neutral semantics, invocation-resolution rules, commitment
domains, and explicit nonclaims are specified in
[`docs/EFFECT_ABI_V0_1.md`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/EFFECT_ABI_V0_1.md).

The *verdict* half lives in [`grillo-verifier`](https://crates.io/crates/grillo-verifier), which
takes pre/post account snapshots plus the instruction's emitted touch map and
proves `changed ⊆ acquired ⊆ authorized`.

Manifests are produced by `hopper compile --emit manifest` from the program
source. Across Hopper-generated contexts and supported governed APIs, one
authored `WRITE_RANGES` const feeds the manifest `writeRanges` and installed
runtime `WritePolicy`. Direct Hopper Native access, unchecked CPI, arbitrary
unsafe/FFI code, and dependencies are outside that enforcement boundary and
must not be described as “published equals enforced” without separate review.

Not yet published to crates.io; part of the Hopper workspace and packaged
for the workspace release (dependency-first publish order: `grillo-manifest`
precedes `grillo-verifier`).
