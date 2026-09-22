# Cicada local SBF regression, 2026-09-22

The newly rebuilt Cicada executable passes all 23 lifecycle tests with zero
skips, including canonical SPL Token and Token-2022 routes, refunds, reclaim,
hostile-route rejection, and rollback. Its 158,640-byte ELF matches the
manifest's versioned release-interface commitment. The exact executable and
route-fixture hashes, test names, and results are in `receipt.json`.

The implementation is from `ad1e209`; verification ran at documentation head
`9fad082`. These are local compiled-SBF results. The builds were not captured
by an isolated clean-checkout provenance driver, so this is not a release
build attestation or a public-cluster deployment record. Interface binding
checks declarations against the artifact; the lifecycle tests exercise behavior.

Each of the three programs was rebuilt with:

```sh
cargo build-sbf --manifest-path examples/<package>/Cargo.toml --sbf-out-dir target/deploy -- --locked
```

The packages are `hopper-cicada`, `hopper-cicada-route-fixture`, and
`hopper-cicada-canonical-route-fixture`. The test run set
`HOPPER_REQUIRE_CICADA_SBF=1`, making missing artifacts an error:

```sh
cargo test -p hopper-cicada --test lifecycle_sbf_e2e --locked -- --nocapture
target/debug/hopper verify --manifest examples/hopper-cicada/hopper.manifest.json --so target/deploy/hopper_cicada.so --release
```

The logs retain the actual results, with local workspace paths replaced by
`<workspace>` and trailing blank lines removed. The build duration includes waiting for another Cargo job and
is not a performance measurement. `SHA256SUMS` hashes the retained files.
