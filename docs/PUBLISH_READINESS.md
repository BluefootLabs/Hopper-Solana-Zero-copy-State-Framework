# Hopper Publish Readiness

- [x] hopper-runtime exposes canonical Hopper-owned runtime surface
- [x] hopper-native is the default backend
- [x] pinocchio is compatibility only
- [x] solana-program is compatibility only (structural support added)
- [x] publishable crates declare homepage, readme, and docs.rs metadata
- [x] checked CPI validates:
  - [x] account count
  - [x] account order (address identity)
  - [x] address matching
  - [x] signer requirements
  - [x] writable requirements
  - [x] borrow compatibility
- [x] no silent truncation in bounded CPI
- [x] README no longer says "Built on Pinocchio"
- [x] Writing Hopper Programs guide exists
- [x] Hopper Native backend guide exists
- [x] Hopper workspace `cargo check --workspace` passes
- [x] Hopper workspace `cargo test --workspace` passes
- [x] canonical example builds under `pinocchio-backend`
- [x] canonical example builds under `solana-program-backend`
- [x] at least one canonical example builds and runs
- [x] examples do not require proc macros for correctness

## Packaging status

- [x] `cargo package -p hopper-native --allow-dirty --no-verify` succeeds
- [x] `hopper-runtime` no longer has an unversioned path dependency blocker
- [ ] first crates.io release still needs staged publish order

The remaining packaging limitation is expected for a fresh multi-crate workspace:
`hopper-runtime` and higher-level crates depend on Hopper crates that are not yet
published on crates.io. The first release should publish in dependency order,
starting with `hopper-native`.
