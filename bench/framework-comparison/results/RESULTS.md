# Framework comparison: Hopper rows against pina's fixtures

Generated 2026-09-22 22:22 UTC by `scripts/bench-framework-comparison.py`.

Same fixtures, same verifier contract, same release recipe as pina's
`benchmarks/framework-comparison` (pinned at pina commit
`aa81c8d1d5816bb932832adce1c5550cd74b6782`). Size is the whole `.so`;
compute units are one Mollusk run per instruction.

Rows marked `measured here` were built and run on this machine:

- `cargo-build-sbf 4.1.0 platform-tools v1.54`
- `rustc 1.96.0 (ac68faa20 2026-05-25)`
- Mollusk 0.15.1
- Source `ad1e209daacc5fbee5fa1a9da58b6df561d6badd`; clean tree: `True`
- Artifact build mode: `rebuilt`; ELF and lockfile SHA-256 hashes in `results.json`

Rows marked `pina published` are copied from pina's
`docs/src/framework-comparison.md` (generated at pina commit
`625d03476052`, cargo-build-sbf from Agave 4.2.2, platform-tools v1.54,
Mollusk 0.14). The rebuilt pinocchio reference row, when present, is the
cross-check: it is pina's own fixture source built and measured here, so
the gap between it and the published pinocchio row is the toolchain
delta to keep in mind when reading the Hopper rows.

## Hello world

| Framework | Source | Size (bytes) | `hello` CU | vs Pinocchio size |
| --- | --- | ---: | ---: | ---: |
| Hopper (substrate) | measured here | 1,656 | 116 | -48% |
| Hopper (macro) | measured here | 1,792 | 138 | -43% |
| Pinocchio (pina reference, rebuilt here) | measured here (cross-check) | 3,160 | 111 | +0% |
| Pina | pina published | 4,680 | 145 | +48% |
| Pinocchio (hand-written) | pina published | 3,160 | 111 | +0% |
| Quasar | pina published | 2,520 | 115 | -20% |
| Anchor v2 (lang-v2, rc.1) | pina published | 1,880 | 127 | -41% |

## Counter

| Framework | Source | Size (bytes) | `initialize` CU | `increment` CU | Account bytes | vs Pinocchio size |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Hopper (substrate) | measured here | 6,728 | 1,606 | 1,742 | 10 | +3% |
| Hopper (macro) | measured here | 8,376 | 1,549 | 358 | 25 | +29% |
| Pinocchio (pina reference, rebuilt here) | measured here (cross-check) | 6,512 | 1,490 | 1,721 | 10 | +0% |
| Pina | pina published | 13,024 | 3,301 | 1,753 |  | +100% |
| Pinocchio (hand-written) | pina published | 6,512 | 1,490 | 1,721 |  | +0% |
| Quasar | pina published | 7,808 | 3,488 | 330 |  | +20% |
| Anchor v2 (lang-v2, rc.1) | pina published | 8,696 | 3,458 | 2,117 |  | +34% |

## Reading the counter rows

- `Hopper (substrate)` is the raw `program_entrypoint!` path with a
  `#[hopper::state(compact, disc = 1)]` account: the same 10-byte
  `[disc][bump][count]` layout, the same plain `CreateAccount` CPI, and
  the same `create_program_address` re-derivation on `increment` as the
  pinocchio and Pina fixtures. It is the like-for-like row.
- `Hopper (macro)` is `#[derive(Accounts)]` plus `#[program]`: `init`,
  `payer`, `seeds`, `bump = <arg>` on `initialize`, `bump = stored` on
  `increment`. The account carries Hopper's 16-byte universal header, so
  it is 25 bytes; Anchor's row has the same caveat at 24 bytes. The
  verifier is told the offsets and checks the same post-state.
- Quasar and the Hopper macro row verify the PDA with SHA-256 without
  a curve check after account validation. The Hopper substrate,
  Pinocchio, and Pina rows use the full PDA derivation syscall.
- The pinned Anchor fixture disables default features and enables
  `alloc`; its row does not include the default `guardrails` feature.
- Pinocchio and Pina take the bump from instruction data; Quasar and
  Anchor search for it on chain inside `initialize`. Both Hopper rows
  take it from instruction data.
