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

## Production deployment and hosted checks

The production website deployed commit
4d327a2d8274fcd3cfe03a677d2adb931891deaa successfully. The live crawl returned
HTTP 200 for all 47 routes and checked 3,974 internal links/heading targets with
zero failures. Production host links account for the higher count than the
local crawl. Release labels and the new named-initialization guide are live.

GitHub checks for framework commit 764af32b20a900c5f2e519a5095a65385000b667
could not start: the annotations report that the account is locked due to a
billing issue. Local passes do not establish hosted Linux/Windows, Kani, or
RustSec success. The exact check URLs and annotations are archived. Hosted
0.4.0 docs.rs pages still returned 404 at the final check; local rustdoc and
published crate source are verified separately.
