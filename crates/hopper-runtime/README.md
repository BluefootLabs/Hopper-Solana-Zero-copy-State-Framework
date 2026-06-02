# hopper-runtime

Canonical low-level runtime surface for [Hopper](https://hopperzero.dev).
Hopper Native is the primary backend. Pinocchio is available only through the
explicit `legacy-pinocchio-compat` migration and benchmark feature;
`solana-program` compatibility is a separate opt-in backend.

## What this crate owns

- **Typed AccountView** with checked and unchecked borrow paths.
- **`Context<'a>`** - the raw runtime context that proc macros bind into typed
  handler contexts.
- **CPI** - `invoke` / `invoke_checked`, `invoke_signed` /
  `invoke_signed_checked`, plus the unchecked Tier C variants with seven-item
  `# Safety` invariants documented inline.
- **PDA helpers** - `find_program_address`, `create_program_address`, plus
  Hopper's verify-only sha256 path that skips `curve_validate` for stored-bump
  PDA verification.
- **Crypto syscalls** - SHA-256, Keccak-256, BLAKE3, curve validation, and
  secp256k1 public-key recovery behind Hopper-owned wrappers.
- **Layout contract** - `LayoutContract` trait, header read/write, layout
  fingerprint comparison.
- **Guard macros** - full Anchor-parity family (`require!`, `require_eq!`,
  `require_neq!`, `require_keys_eq!`, `require_keys_neq!`, `require_gt!`,
  `require_gte!`, `require_lt!`, `require_lte!`), plus `err!` / `error!`
  short-form aliases.
- **Backend bridge** - feature-gated routing to `hopper-native` (primary),
  `legacy-pinocchio-compat` migration shims, or `solana-program` substrates.
- **System Program builders** - `Transfer`, `CreateAccount`, `Allocate`,
  `Assign`.
- **Rent-exemption helper** - `rent::check_rent_exempt(account)` backing the
  `#[account(rent_exempt = enforce)]` field keyword.
- **Foreign and remaining accounts** - `ExternalZeroCopy` /
  `ExternalAccount<'info, T>` for known non-Hopper account bytes, plus strict,
  passthrough, raw, bounded, and typed sequential remaining-account parsers.
- **Stored instructions** - `StoredAccountMeta` and `StoredInstruction<'a>` for
  governance/proposal executors that persist arbitrary CPI payloads.
- **Token / Token-2022 readers** - base-layout readers for Mint and
  TokenAccount, plus the TLV scanner that powers the `extensions::*`
  constraints.
- **Dynamic-tail runtime** - `HopperString`, `HopperVec`, `TailStr`,
  `TailBytes`, and compact-tail codecs used by Quasar-style `#[account]`
  dynamic fields.

Hopper headers are for Hopper-owned accounts. Hopper's framework layer verifies
owner, account role, discriminator, version, and layout fingerprint before typed
borrows reach program code, while runtime helpers keep known foreign accounts,
interfaces, raw accounts, and remaining-account tails explicit rather than
forcing them into a Hopper-owned schema.

Most users touch this crate transitively through the `hopper` umbrella crate
and `hopper::prelude::*`. Reach for `hopper-runtime` directly when writing a
crate that needs the runtime surface without the higher-level framework
features.

Crypto API coverage is tracked in
[`docs/CRYPTO_CAPABILITIES.md`](../../docs/CRYPTO_CAPABILITIES.md), including
the shipped hash/recover helpers and the planned feature-gated heavy-crypto
surface.

Docs: <https://docs.rs/crate/hopper-runtime/0.2.1>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
