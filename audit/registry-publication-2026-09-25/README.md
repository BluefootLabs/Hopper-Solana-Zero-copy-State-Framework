# Verified crates.io publication — 2026-09-25

`publication.json` records 26 newly published 0.3.2
framework/CLI packages and 3 unchanged 0.1.0
packages. Publication source is `5f5053d34526c86724ca2289030447cdc55cae06`. Each new
package passed its publication dry run and upload; the receipt checks indexed
registry checksums against the exact packaged archive hashes.

`publish-train.json` and its transcript are the refreshed prepublication check
at the publication commit. The original gated train remains under the on-chain
archive's `gates/`. `source-lineage.json` binds both captures, records the exact
authorized source diff and stripped policy-code hash, and requires all 62
finalized devnet transactions and exact deployed ELF bytes. `policy-tests.log`
records the targeted policy tests after the comment/documentation correction.

`downloads.json` verifies 29 actual registry downloads,
matching checksums, and clean embedded VCS source commits. The downloaded
`.crate` payloads are not duplicated in this archive. No credentials are included.
Later repository documentation may describe availability and these completed
release results. Those post-publication prose updates do not alter the immutable
published archives or their pinned source commit.

`consumer/` contains 2 fresh registry-only programs,
their exact source, dependency manifests and locks, command logs, and SBF v0
ELFs. Their execution captures report 4 passing compiled suites.
2 of 2 consumer ELFs are byte-identical to the
corresponding gated v0 ELFs; the receipt preserves each artifact identity flag,
source hash, dependency checksum, and verifier identity. Passing behavior tests
alone is not a claim of identical executable bytes.

Use `cargo add hopper-lang@0.3.2 --rename hopper --features proc-macros` and
`cargo install hopper-cli --version 0.3.2 --locked`.
The related on-chain evidence is in
[`../onchain-byte-policies-2026-09-25`](../onchain-byte-policies-2026-09-25/).
Publication and maintainer execution evidence do not establish an independent audit.
