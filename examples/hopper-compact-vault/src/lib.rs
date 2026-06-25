//! # Hopper Compact Vault — three-tier metadata example
//!
//! Demonstrates two of the three tiers from `docs/THREE_TIER_METADATA.md`:
//!
//! - **Tier 1 (hot path):** [`Vault`] is a *compact* account stored as
//!   `[disc:u8][zero-copy body]` — no 16-byte universal header. A handler
//!   loads it with [`AccountView::load_compact`], which is just
//!   `check_len` + `check_disc` + cast-body-at-byte-1.
//! - **Tier 2 (on-chain registry):** [`program_registry`] builds a
//!   compact binary registry describing every account the program owns,
//!   and [`read_registry`] reads it back and verifies its hash.
//!
//! The `#[hopper::state(compact, ...)]` derive sugar is the documented
//! next step; until it lands, a compact layout is a plain `#[repr(C)]`
//! struct that implements `Pod` + [`CompactLayout`], as shown here.

#![cfg_attr(target_os = "solana", no_std)]

use hopper::account::{AccountView, CompactLayout};
use hopper::hopper_core::abi::WireU64;
use hopper::hopper_core::account::{Pod, Zeroable};
use hopper::manifest::{
    name_hash, write_registry, AccountLayoutEntry, ManifestProfile, ProgramManifestView,
    ENTRY_FLAG_COMPACT, REGISTRY_VERSION,
};
use hopper::prelude::{Address, ProgramError, ProgramResult};

/// Discriminator for the compact vault.
pub const VAULT_DISC: u8 = 1;

/// Tier-1 compact vault body: `[disc:u8][authority:32][balance:8]`.
///
/// All fields are alignment-1 (`Address` is a 32-byte array, `WireU64` is
/// `#[repr(transparent)]` over `[u8; 8]`), so the body is `Pod` and
/// overlays directly on bytes 1.. of the account.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Vault {
    pub authority: Address,
    pub balance: WireU64,
}

// SAFETY: `#[repr(C)]` over alignment-1 fields with no padding; every bit
// pattern is valid; `Copy`, no drop glue.
unsafe impl Zeroable for Vault {}
unsafe impl Pod for Vault {}

impl CompactLayout for Vault {
    const DISC: u8 = VAULT_DISC;
}

/// Hot-path handler shape: load the compact vault and add to its balance.
///
/// Note there is no header validation beyond the single discriminator
/// byte — layout identity is the program's Tier-2 registry, not a
/// per-account fact re-checked on every call.
pub fn deposit(vault: &AccountView, amount: u64) -> ProgramResult {
    let mut v = vault.load_compact_mut::<Vault>()?;
    v.balance.checked_add_assign(amount)
}

/// Initialise a fresh compact vault account.
pub fn initialize(vault: &AccountView, authority: Address) -> ProgramResult {
    vault.init_compact::<Vault>()?;
    let mut v = vault.load_compact_mut::<Vault>()?;
    v.authority = authority;
    v.balance = WireU64::ZERO;
    Ok(())
}

/// This program's manifest profile. `onchain` means it publishes and
/// reads the Tier-2 registry PDA.
pub const PROFILE: ManifestProfile = ManifestProfile::Onchain;

/// Build the Tier-2 binary registry into `buf`, returning bytes written.
///
/// One entry describes the compact [`Vault`]. A real program with several
/// account types would list them all here (typically code-generated).
pub fn program_registry(buf: &mut [u8], schema_hash: &[u8; 32]) -> Result<usize, ProgramError> {
    let entries = [AccountLayoutEntry::new(
        Vault::DISC,
        1,
        Vault::COMPACT_LEN as u32,
        Vault::BODY_SIZE as u32,
        0,
        ENTRY_FLAG_COMPACT,
        name_hash("Vault"),
    )];
    write_registry(
        buf,
        REGISTRY_VERSION,
        PROFILE.header_flags(),
        schema_hash,
        &entries,
    )
}

/// Read a Tier-2 registry, returning a bounds-checked parsed view.
pub fn read_registry(data: &[u8]) -> Result<ProgramManifestView<'_>, ProgramError> {
    ProgramManifestView::parse(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_vault_is_one_byte_header_plus_body() {
        // disc(1) + authority(32) + balance(8) = 41, NOT 16 + 40.
        assert_eq!(Vault::BODY_SIZE, 40);
        assert_eq!(Vault::COMPACT_LEN, 41);
    }

    #[test]
    fn registry_roundtrip_and_hash_verify() {
        let schema_hash = [7u8; 32];
        let mut buf = [0u8; 512];
        let n = program_registry(&mut buf, &schema_hash).unwrap();

        let view = read_registry(&buf[..n]).unwrap();
        assert_eq!(view.account_count(), 1);
        assert!(view.verify_registry_hash());

        let entry = view.find_by_disc(VAULT_DISC).unwrap();
        assert_eq!(entry.fixed_size.get(), 40);
        assert_eq!(entry.min_size.get(), 41);
        assert!(entry.has_flag(ENTRY_FLAG_COMPACT));
        assert_eq!(entry.name_hash, name_hash("Vault"));

        assert!(PROFILE.publishes_onchain());
    }
}
