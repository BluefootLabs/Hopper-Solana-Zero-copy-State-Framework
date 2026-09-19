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
The crate also parses and commits the experimental strict Effect ABI v0.2
contract (`EffectContractV2`), specified in
[`docs/EFFECT_ABI_V0_2.md`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/EFFECT_ABI_V0_2.md).

The *verdict* half lives in the workspace's
[`grillo-verifier`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/crates/grillo-verifier/README.md), which
takes caller-supplied pre/post account snapshots plus the instruction's touch
map and checks `changed subset acquired subset authorized` within that supplied
evidence scope. It does not authenticate or complete the evidence bundle.

## Upgrade authority diff

`grillo_manifest::authority` compares two manifests and answers one upgrade
review question: does the new release grant any instruction more authority
than the old one did?

```rust
use grillo_manifest::authority::{AuthorityDiff, AuthorityVerdict};

let report = AuthorityDiff::between_json(&old_json, &new_json)?;
if report.verdict() != AuthorityVerdict::NotWidened {
    print!("{}", report.render());
}
```

Instructions are matched by their exact discriminator bytes and accounts by
role name. A finding is `Widened`, `Review`, `Narrowed`, or `Info`. Widening
covers a dropped signer, a read-only account that became writable, a new
instruction or writable account, byte ranges that gain a layout field (compared
per field, so a field that only moved offset is not a new permission), a
removed exact-cell rule, a new lamport permission, a lost `strict_writes` or
lamport contract, a raised remaining-account ceiling, and weaker context
constraints: removed PDA seeds, `has_one` relations, owner or address checks,
a type check replaced by an unchecked kind, an account that became optional,
or a new `init`, `realloc`, or `close` lifecycle. Changes that have no order,
such as different PDA seeds or a different expected CPI program, are `Review`.

The report records a SHA-256 over the canonical JSON of both manifests.
`AuthorityReport::check_approval` accepts a previously reviewed report only
for that exact manifest pair, so an approval cannot be replayed onto a later
upgrade. The diff compares declarations. It does not inspect bytecode or prove
that a handler honors its manifest; `hopper verify --release` binds each
manifest to its ELF, and the verifier checks observed effects.

Manifests are produced by `hopper compile --emit manifest` from the program
source. Across Hopper-generated contexts and supported governed APIs, one
authored `WRITE_RANGES` const feeds the manifest `writeRanges` and installed
runtime `WritePolicy`. Direct Hopper Native access, unchecked CPI, arbitrary
unsafe/FFI code, and dependencies are outside that enforcement boundary and
must not be described as “published equals enforced” without separate review.

This workspace source is version 0.1.0. It was not observed indexed on
crates.io on 2026-09-06. In Hopper's dependency-first publish order,
`grillo-manifest` precedes `grillo-verifier`; registry availability must be
confirmed before publishing the dependent package or advertising registry
installation.
