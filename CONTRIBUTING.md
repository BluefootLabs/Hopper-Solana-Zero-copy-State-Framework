# Contributing to Hopper

Thanks for your interest. Hopper is a zero-copy state framework for Solana,
built and maintained by [BluefootLabs](https://github.com/BluefootLabs)
and a growing pool of contributors. This document covers what we expect
from PRs and how to land one cleanly.

## Quick links

- **Website**: [hopperzero.dev](https://hopperzero.dev)
- **Issues**: [github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/issues](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/issues)
- **Audit**: [AUDIT.md](AUDIT.md) — full feature audit and parity findings vs Pinocchio, Quasar, Anchor.
- **Architecture**: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Unsafe inventory**: [docs/UNSAFE_INVARIANTS.md](docs/UNSAFE_INVARIANTS.md)

## What to work on

Open issues labelled `good-first-issue` are scoped for newcomers. The
[AUDIT.md](AUDIT.md) gap list at the end of the document is the
authoritative roadmap for parity additions; pick something marked
"deferred" and open an issue to claim it.

We particularly welcome:

- **Token-2022 extension keywords** (`group_pointer`,
  `group_member_pointer`, `confidential_transfer`).
- **`#[hopper::view]`** read-only handler attribute.
- **Field-keyword sugar** for the existing Metaplex builders
  (`metadata::name`, `master_edition::max_supply`, etc.).
- **Bench expansions** — particularly anything that exercises lazy
  dispatch under realistic dispatch shapes.
- **Example programs** demonstrating real protocol patterns: AMM,
  lending, multisig, escrow.

## How to land a PR

1. **Open an issue first** for anything bigger than a typo. We move
   fast on small fixes; design discussions belong in issues.
2. **Branch from `main`** and keep your PR focused — one concern per
   PR.
3. **Match the existing style.** Hopper uses prose comments that
   explain *why* (not what), the `// SAFETY:` convention on every
   `unsafe` block, and the macro-vocabulary patterns established in
   `crates/hopper-macros` and `crates/hopper-macros-proc`.
4. **Tests required for behaviour changes.** New runtime helpers need
   unit tests. New macros need a `tests/hopper-trybuild` fixture
   (pass + fail). New CLI commands need a smoke-test invocation.
5. **`cargo fmt` and `cargo clippy`** before pushing. CI will reject
   unformatted code.
6. **Update CHANGELOG.md** under `[Unreleased]` for any
   user-observable change.

## Safety reviews

Anything that touches `unsafe` gets an extra round of review. The
expectation:

- Every `unsafe` block has a `// SAFETY:` comment naming the
  invariant the caller is upholding.
- Every `pub unsafe fn` carries a `# Safety` doc section listing the
  caller's obligations.
- New entries land in [`docs/UNSAFE_INVARIANTS.md`](docs/UNSAFE_INVARIANTS.md)
  with the file:line reference.

## Audit posture

Hopper aims for protocol-grade safety. Submissions that loosen a
safety invariant — even by accident — will get pushed back hard. If
you genuinely believe a check is unnecessary, open an issue
explaining the proof, and we'll discuss it before any code lands.

## Code of conduct

Be kind, be specific, and assume good faith. No harassment, no
personal attacks. The Galápagos blue-footed booby is the project
mascot — even she gets along with the iguanas.

## License

By contributing you agree your work is licensed under
[Apache-2.0](LICENSE) — the same license the rest of the project
uses.
