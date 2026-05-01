# hopper-runtime

Canonical low-level runtime surface for [Hopper](https://hopperzero.dev).
Hopper Native is the primary backend. Pinocchio is available only through the
explicit `legacy-pinocchio-compat` migration/benchmark feature; solana-program
compatibility is a separate opt-in backend.

## What this crate owns

- **Typed AccountView** with checked + unchecked borrow paths.
- **`Context<T>`** — the typed entry point every Hopper handler receives.
- **CPI** — `invoke`, `invoke_signed`, plus the unchecked Tier C variants
  with seven-item `# Safety` invariants documented inline.
- **PDA helpers** — `find_program_address`, `create_program_address`, plus
  Hopper's verify-only sha256 path that skips `curve_validate` for stored-bump
  PDA verification.
- **Layout contract** — `LayoutContract` trait, header read/write, layout
  fingerprint comparison.
- **Guard macros** — full Anchor-parity family (`require!`, `require_eq!`,
  `require_neq!`, `require_keys_eq!`, `require_keys_neq!`, `require_gt!`,
  `require_gte!`, `require_lt!`, `require_lte!`), plus `err!` / `error!`
  short-form aliases.
- **Backend bridge** — feature-gated routing to `hopper-native` (primary),
  `legacy-pinocchio-compat` migration shims, or `solana-program` substrates.
- **System Program builders** — `Transfer`, `CreateAccount`, `Allocate`,
  `Assign`.
- **Rent-exemption helper** — `rent::check_rent_exempt(account)` backing the
  `#[account(rent_exempt = enforce)]` field keyword.
- **Token / Token-2022 readers** — base-layout readers for Mint and
  TokenAccount, plus the TLV scanner that powers the `extensions::*`
  constraints.

Most users touch this crate transitively through the `hopper` umbrella crate
and `hopper::prelude::*`. Reach for `hopper-runtime` directly when writing a
crate that needs the runtime surface without the higher-level framework
features.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
