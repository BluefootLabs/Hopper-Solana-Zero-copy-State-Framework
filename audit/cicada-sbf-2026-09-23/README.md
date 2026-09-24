# Cicada isolated SBF release evidence, 2026-09-23

The clean source `685deff6a14f00675a4e0f7dac410d94f5e69f57` produced a
157,672-byte SBF v0 ELF with cargo-build-sbf 4.1.0 and platform tools v1.54.
The generated manifest passes the full publish check, including exact
versioned release-interface binding. `attestation.json` records the source
before and after, the previously absent output paths, commands and hashes.
This is a local clean-build attestation, not a hosted CI result or deployment.

All 23 compiled lifecycle tests pass with zero skips against this program
and freshly rebuilt hostile and canonical route fixtures. Missing artifacts
were fatal through `HOPPER_REQUIRE_CICADA_SBF=1`. The matrix includes SPL Token
and Token-2022 custody, route commitments, policy mutation refusal, observed
token/lamport deltas, refund, reclaim and rollback. Exact artifact hashes and
the source boundary are in `lifecycle-receipt.json`; test names are retained.

The build, manifest-generation and publish-check command JSON files are copied
byte-for-byte and match the hashes in the attestation. Their original paths
remain in that record. Private generated keypairs are excluded. Rebuild with:

```sh
python scripts/attest-sbf-release.py --package hopper-cicada --program-manifest examples/hopper-cicada/Cargo.toml --manifest target/hopper/cicada-proof/manifest.json --binary target/hopper/cicada-proof/sbf/hopper_cicada.so --tools-version v1.54 --arch v0 --hopper target/debug/hopper --out target/hopper/cicada-proof/attestation.json
```

Use new ignored output paths, and `hopper.exe` on Windows. Build both route
fixtures with the same pinned tools before running the lifecycle test. The
test currently reads the three ELFs from `target/deploy`; verify the copied
hashes against the receipts before invoking it from Cargo.

Exact-route mode still commits the CPI envelope rather than an upgradeable
route's executable bytes. A third-party AMM integration and authenticated
route-artifact policy remain open. These results do not establish an
independent security audit or production readiness.
