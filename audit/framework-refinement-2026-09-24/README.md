# Hopper 0.3.1 refinement evidence — 2026-09-24

This maintainer capture records focused source, host, compiled, devnet and
benchmark evidence. It is not an independent audit or a universal ranking.

* `validation/`: 2,224 host tests passed, zero failed, 232 ignored; formatting,
  warnings-denied Clippy, 308-file unsafe scan and provenance checks passed.
  Cicada's separate CLI semantic adapter passed 698 cases with zero skips.
* `compiled/`: all five mint and three PDA suites passed on v0 and v3. The
  mint suites cover all 64 supported extension subsets, exact allocation,
  prefunding, PDA signing, owner forgeries, invalid configuration and rollback.
  Cicada's 23 compiled lifecycle tests passed; escrow's manifest-enabled SBF
  build also passed. Builder 4.1.0/platform-tools 1.54 are pinned in commands.
* `cicada/`: the 154,160-byte executable, generated manifest and full
  release-interface attestation, with command transcripts. SHA-256:
  `9c67f2e9015af8c18846710c4d8e6dc5b82552d16a05014a138d78eb1e1a1d8a`.
* `devnet/`: 45 finalized transactions: 11 mint, nine initialization/typed PDA,
  and 25 readonly PDA cases. Full expected snapshots and exact before/after
  deployed images were checked. The mint rollback case exhausts 10,000 CU
  after successful System and Token-2022 CPIs, preserving account state and
  spending only the transaction fee. A dedicated mint payer isolates that check.
* `comparison/`: clean `dfd1400` capture under Pina's recipe. Counter increment
  improves from 358 to 349 CU and executable size from 8,376 to 8,312 bytes.
  Quasar's pinned published increment is still 330 CU. Peer rows remain
  published values; the rebuilt Pinocchio reference reproduces its cross-check.
* Root feature observations: finalized public RPC with genesis and Feature
  ownership checks. These are endpoint observations, not ledger proofs.

Host/SBF source is clean `ab955e4`. Devnet source is `b9a688e`; its only change
corrects the live nested-VM error assertion. Publication also uses `54ec16c`,
which only fixes the CLI README installation heading. The Rust sources,
dependency manifests and lockfiles are identical across these three commits.
Every receipt retains its actual source. Publication is archived separately in
[`../registry-publication-2026-09-24`](../registry-publication-2026-09-24/).

Devnet required at least 10,240 additional bytes for the PDA program upgrade.
Its compiler ELF is 13,896 bytes; deployed capacity is 18,528. The suffix is
verified zero padding and the complete padded image passed all three compiled
PDA suites. Deployed SHA-256:
`fc7c043f77fcded944af4c4ea262426a90d8274aa2e7303ea25eb0882c348f69`.
The expanded fixture measures literal-PDA checking at 78 CU, runtime search
at 1,079 CU, canonical initialization at 2,541 CU and stored typed reads at
318 CU. These are complete fixture instructions, not intrinsic macro costs.

No keypairs are archived. The scripts contain captured local paths; use the
corresponding source revision and fresh output paths when reproducing. Hosted
GitHub jobs did not start because of an account billing lock. Cicada's evidence
here is local compiled execution, not a public-cluster Cicada deployment.

The production website at `36a35b5` passed its build and lint checks, then all
42 live routes and 3,368 internal links/anchors. The deployment and HTTP checks
are in `validation/`. No browser visual inspection was available.

For fresh live mint/PDA initialization captures, use a fresh mint payer/PDA
and an absent config PDA. The retained initialization fixture now has state;
reproducing its creation requires a fresh program identity and corresponding
literal-PDA constant. Readonly fixture checks remain repeatable.
