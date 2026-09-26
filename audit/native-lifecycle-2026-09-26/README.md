# Native lifecycle validation - September 26, 2026

Native/runtime 0.4.2 is published. Registry archive checksums and all packaged
Rust source/README files match the publication source. A registry-only consumer
compiles native/runtime 0.4.2 with framework 0.4.0.

The devnet run contains 24 finalized transactions: four setup transactions and
20 lifecycle cases with complete expected account snapshots, including two
transaction refusals. Successful cases also catch and verify local refusals.
Growth, shrink, regrowth, mapped writes, transfers, close and borrow conflicts
are exercised. Both deployed v0 programs match their local ELFs before and after.

The published 0.4.1 runtime fails the compiled segment-borrow regression before
attempting conflicting references. Fixed v0/v3 pass. Host native lifecycle tests
also reproduce baseline partial-close, alias and writable-preflight failures.
Miri passes the native mapped-borrow and lifecycle tests after correcting the
projection handoff and test allocation provenance. Source lineage records why
the later publication source corresponds to the devnet-tested ELF bytes.

Treasury, bounded multisig, classic-token escrow, byte allowance and ambient
write-gate suites pass in compiled v0/v3. Their final rebuilt ELFs are identical
to the tested binaries. This record does not claim those application examples
were all redeployed in this run, nor provide new whole-framework performance
comparisons. The framework and CLI remain 0.4.0.

The 22-feature-per-cluster network capture validates genesis, Feature ownership
and activation slots through finalized RPC. Research metadata distinguishes
downloaded source from reviewed boundaries. Third-party source, private keys,
raw deployment logs, downloaded crate archives and HTML are excluded.

Reproduce the on-chain cases with scripts/test-lifecycle-devnet.py and the
programs in bench/lifecycle. The local build helpers are under reproduction/.
This is internal validation, not an independent security audit or a complete
line-by-line audit of all upstream repositories. Hosted CI was previously
blocked by the GitHub account billing lock; local results are recorded here.
