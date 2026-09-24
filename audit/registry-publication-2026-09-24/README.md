# Verified crates.io publication — 2026-09-24

All 26 framework/CLI updates are published as 0.3.1. `grillo-manifest`,
`grillo-verifier`, and `hopper-topology` retain their unchanged 0.1.0 releases.

`publication.json` records each exact dry-run archive hash, indexed registry
checksum and source commit. `downloads.json` verifies all 29 actual registry
downloads against those checksums and their clean `.cargo_vcs_info.json` source
commits. No credentials are included. Each new crate passed its registry
publication dry run; package metadata and topological ordering passed for all 29.

The source transition from `b9a688e` to `54ec16c` corrects only the CLI README
installation heading before that package's upload. Already uploaded packages
keep their original source records and bytes. Later repository documentation
clarifies release availability without changing immutable registry archives.

`consumer/` contains fresh registry-only mint and PDA programs, locked
dependencies and logs. Five compiled mint suites and three PDA suites pass.
Their 16,576-byte mint and 13,896-byte PDA executables exactly reproduce the
compiler artifacts tested locally and on devnet (the deployed PDA capacity
adds verified zero padding, described in the refinement archive).

Use `cargo add hopper-lang@0.3.1 --rename hopper --features proc-macros` and
`cargo install hopper-cli --version 0.3.1 --locked`.
Publication and execution evidence do not establish an independent audit.
