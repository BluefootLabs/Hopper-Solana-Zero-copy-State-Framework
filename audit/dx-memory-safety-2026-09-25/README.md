# Hopper 0.4 DX and memory-safety evidence

Local gates and finalized devnet tests used clean source 8ca9f5c984b81a18a87edbd1ed19c5ac3106d2cb.
Publication uses a4ed5fdd4b291592cfc1240e5ba20936b96e27c3; the registry archive's source-lineage
receipt proves that only README and release-validation documentation changed.

Named fixed-field inputs and composed initialization preserve the existing wire
layout and ordinary Rust handler model. Safe projections now check real type
bounds; typed DSL wrappers respect header offsets; malformed segment geometry
is rejected. The vault uses the required System Program CPI account.

All 19 vault transactions finalized with complete expected-state checks. The
deployed v0 ELF matches before and after. Deposit: 1,602 CU; withdrawal: 240 CU.
SBF v0/v3 fixture suites, 23 Cicada compiled lifecycle cases, and 698 Cicada host
semantic cases passed. These scopes are distinct. Host emulation is not runtime
rollback or CU evidence. Peer benchmark rows were not rerun for this release.

The website build/lint and all 47 rendered routes passed. Local QA checked
3,263 internal links/heading targets with zero failures. Browser connection was
unavailable, so no visual inspection is claimed. Hosted CI is recorded
separately once the source has been pushed.

The placement compiler and broad wrapper-extension model remain proposals.
This archive is reproducible development evidence, not an independent audit.
Every public artifact is sealed in SHA256SUMS; private keys and package archives
are excluded.
