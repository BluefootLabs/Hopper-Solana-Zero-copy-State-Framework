# Reference fixtures from pina

`hello/` and `counter/` are verbatim copies of the hand-written pinocchio
fixtures in the pina repository:

- source: https://github.com/pina-rs/pina
- path: `benchmarks/framework-comparison/programs/{hello,counter}/pinocchio`
- commit: `aa81c8d1d5816bb932832adce1c5550cd74b6782` (2026-09-21)
- license: Apache-2.0, as declared by pina's workspace `Cargo.toml`

They are the floor every framework row is measured against. This tree
rebuilds them with the same release recipe pina uses and runs them through
the same verifier contract, so the Hopper rows in `../results/RESULTS.md`
can be read against pina's published table with the toolchain delta made
explicit. Each fixture is a standalone crate (its own `[workspace]` and
`Cargo.lock`), excluded from the Hopper workspace on purpose.

Nothing here is Hopper code and nothing here is modified. Regenerate with
`py -3.12 scripts/bench-framework-comparison.py`.
