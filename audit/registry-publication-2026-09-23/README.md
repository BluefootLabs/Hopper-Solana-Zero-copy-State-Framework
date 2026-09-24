# Hopper 0.3.0 registry publication, September 23, 2026

All 29 public packages were published and independently downloaded from
crates.io. Publication timestamps are September 24 UTC, September 23 in the
maintainer's America/Chicago time zone. The framework and CLI are 0.3.0;
grillo-manifest, grillo-verifier and hopper-topology are 0.1.0.

`publication.json` records each registry checksum and source commit. The first
17 packages use clean source `e47a4d5`; the remaining 12 use `471fc5a`. Their
only source difference is the Grillo verifier README installation correction,
committed before that package was uploaded. No Rust code or dependency changed.
`download-verification.json` checks all 29 downloaded archives, embedded clean
VCS commits, versions, README presence, and internal dependency lock checksums.
The command logs record every successful registry dry run and upload.

Cargo 1.96 stages `cargo publish` archives under `target/package/tmp-crate`.
The older `cargo package` output can differ in generated dependency checksums
after earlier packages have indexed. The vesting receipt retains the detected
archive-lookup correction: its actual publish archive matched the registry
byte-for-byte, and only the generated Cargo.lock differed from the earlier
package preflight. The two preflight JSON files describe package-content
checks; their package archives are not the authoritative upload checksums.

## Registry-only compiled consumer

`consumer/` contains a standalone program with an exact crates.io dependency
on hopper-lang 0.3.0, no path dependencies or patches, and a fresh lockfile.
All dependency sources are crates.io; the release dependency checksums match
the publication receipt. It copies the reviewed canonical-PDA fixture and
builds with cargo-build-sbf 4.1.0, platform-tools v1.54 and SBPF v0.
Its 8,288-byte ELF is byte-identical to the canonical-PDA artifact tested on
devnet, including the 71-CU literal check and 1,074-CU runtime search.

The SDK-oracle execution test passes all seven modes: canonical admission,
every other bump refused, all off-curve noncanonical addresses refused, and
complete account state unchanged. The receipt records the executable hash,
size and CU rows. This is compiled local execution of registry packages. The
[25 finalized devnet transactions](../canonical-pda-2026-09-23/README.md)
retain their separate source and deployed-artifact boundary.

To reproduce, copy `consumer/` outside the workspace, build its manifest with
the pinned SBF tools and `--locked`, then set `HOPPER_CANONICAL_PDA_SBF` to the
new ELF and run the `canonical_pda_sbf` verifier test with `--ignored --nocapture`.
Use the verifier at source `471fc5a`; do not treat missing SBF artifacts as a pass.

## Other release checks

The fresh `cicada/` attestation binds clean `471fc5a` to a 157,672-byte ELF,
generated manifest, exact interface commitment and full successful publish
check. The ELF is byte-identical to the one that passed all 23 compiled Cicada
lifecycle tests. It was not deployed by this run. Authenticated upgradeable
route-artifact policy and third-party AMM integration remain open.

The core host run passed 2,214 tests, with 225 ignored and zero failures.
The subsequent CLI discovery fix passed all 262 CLI tests; warnings-denied
workspace Clippy passed after that change. The unsafe inventory covers all
29 public packages. Source and log boundaries remain explicit in these records.
Grillo checks supplied effects; its RPC and replay labels are not authenticated
ledger evidence. No independent security audit is claimed.

GitHub hosted jobs could not start because GitHub reported an account billing
lock. These are passing local checks and registry verifications, not green
hosted CI. Keys, credentials, generated program keypairs and payer files are
excluded from this archive.
