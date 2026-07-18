//! # Hopper Compact Vault — three-tier metadata example
//!
//! Demonstrates two of the three tiers from `docs/THREE_TIER_METADATA.md`
//! using the **macro ergonomics** added on top of the foundation:
//!
//! - **Tier 1 (hot path):** [`Vault`] is declared with
//!   `#[hopper::state(compact, disc = 1)]`. The macro emits a
//!   [`CompactLayout`](hopper::account::CompactLayout) impl, the
//!   `[disc:u8][zero-copy body]` load helpers, and the compact field
//!   offsets — no 16-byte universal header.
//! - **Tier 2 (on-chain registry):** [`program_registry`] builds a
//!   compact binary registry from the macro-generated
//!   [`Vault::registry_entry`], and [`read_registry`] reads it back and
//!   verifies its hash.
//!
//! The hand-written `impl CompactLayout` shown in earlier revisions is no
//! longer needed: `#[hopper::state(compact, ...)]` generates it.

#![cfg_attr(target_os = "solana", no_std)]

use hopper::hopper_core::abi::WireU64;
use hopper::hopper_runtime::address::address_eq;
use hopper::manifest::{write_registry, ManifestProfile, ProgramManifestView, REGISTRY_VERSION};
use hopper::prelude::{AccountView, Address, ProgramError, ProgramResult};

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction);

/// Discriminator for the compact vault.
pub const VAULT_DISC: u8 = 1;

/// Initialize a pre-created 41-byte compact vault account.
pub const IX_INIT: u8 = 0;

/// Deposit lamports into the compact vault balance field.
pub const IX_DEPOSIT: u8 = 1;

/// Tier-1 compact vault body: `[disc:u8][authority:32][balance:8]`.
///
/// `#[hopper::state(compact, disc = 1)]` emits the `CompactLayout` impl,
/// `Pod`/`Zeroable` proofs, compact load helpers, the `registry_entry()`
/// row builder, and field metadata whose absolute offsets start at byte 1.
#[derive(Clone, Copy, Debug, Default)]
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Vault {
    #[role = "authority"]
    pub authority: Address,
    #[role = "balance"]
    pub balance: WireU64,
}

/// Hot-path handler shape: load the compact vault and add to its balance.
///
/// No header validation beyond the single discriminator byte — layout
/// identity is the program's Tier-2 registry, not a per-account fact.
pub fn deposit(vault: &AccountView, amount: u64) -> ProgramResult {
    let mut v = vault.load_compact_mut::<Vault>()?;
    v.balance.checked_add_assign(amount)
}

/// Deposit after proving the signing account matches the compact authority
/// field stored at byte offset 1.
pub fn authorized_deposit(
    vault: &AccountView,
    authority: &AccountView,
    amount: u64,
) -> ProgramResult {
    authority.require_signer()?;
    let expected = {
        let v = vault.load_compact::<Vault>()?;
        v.authority
    };
    if !address_eq(&expected, authority.address()) {
        return Err(ProgramError::IncorrectAuthority);
    }
    deposit(vault, amount)
}

/// Initialise a fresh compact vault account.
pub fn initialize(vault: &AccountView, authority: Address) -> ProgramResult {
    vault.init_compact::<Vault>()?;
    let mut v = vault.load_compact_mut::<Vault>()?;
    v.authority = authority;
    v.balance = WireU64::ZERO;
    Ok(())
}

/// Minimal deployable dispatcher used by the devnet proof.
///
/// Accounts:
/// - `init`: `[vault(w), authority(s)]`; the caller creates and funds the
///   exact 41-byte account with the System Program before invoking Hopper.
/// - `deposit`: `[vault(w), authority(s)]`, data `[1][amount:u64-le]`.
pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (tag, data) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match *tag {
        IX_INIT => process_init(program_id, accounts, data),
        IX_DEPOSIT => process_deposit(program_id, accounts, data),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn process_init(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    if !data.is_empty() || accounts.len() < 2 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let vault = &accounts[0];
    let authority = &accounts[1];
    vault.require_writable()?;
    vault.require_owned_by(program_id)?;
    authority.require_signer()?;
    initialize(vault, *authority.address())
}

fn process_deposit(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    if data.len() != 8 || accounts.len() < 2 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let vault = &accounts[0];
    let authority = &accounts[1];
    vault.require_writable()?;
    vault.require_owned_by(program_id)?;
    let amount = u64::from_le_bytes(
        data.try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    authorized_deposit(vault, authority, amount)
}

/// This program's manifest profile. `onchain` means it publishes and
/// reads the Tier-2 registry PDA.
pub const PROFILE: ManifestProfile = ManifestProfile::Onchain;

/// Build the Tier-2 binary registry into `buf`, returning bytes written.
///
/// The single entry is produced by the macro-generated
/// [`Vault::registry_entry`]. A real program with several account types
/// would push one `registry_entry()` per type.
pub fn program_registry(buf: &mut [u8], schema_hash: &[u8; 32]) -> Result<usize, ProgramError> {
    let entries = [Vault::registry_entry()];
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
    use hopper::manifest::{
        diff_descriptors_vs_registry, min_loaded_data_size, name_hash, recommend_loaded_data_limit,
        CostLint, LayoutDescriptor, LayoutKind, ManifestProfile, RegistryCompat, SizeClass,
        ENTRY_FLAG_COMPACT,
    };
    use hopper_schema::clientgen::{KtAccounts, TsAccounts};
    use hopper_schema::codama::{IdlJsonFromManifest, ManifestJson};
    use hopper_schema::{
        FieldDescriptor, FieldIntent, InstructionDescriptor, LayoutManifest, ProgramManifest,
    };

    static VAULT_FIELDS: [FieldDescriptor; 2] = [
        FieldDescriptor {
            name: "authority",
            canonical_type: "Pubkey",
            size: 32,
            offset: 1,
            intent: FieldIntent::Authority,
        },
        FieldDescriptor {
            name: "balance",
            canonical_type: "u64",
            size: 8,
            offset: 33,
            intent: FieldIntent::Balance,
        },
    ];

    static VAULT_LAYOUTS: [LayoutManifest; 1] = [LayoutManifest {
        name: "Vault",
        disc: VAULT_DISC,
        version: 1,
        layout_id: Vault::LAYOUT_ID,
        total_size: Vault::COMPACT_LEN,
        field_count: VAULT_FIELDS.len(),
        fields: &VAULT_FIELDS,
    }];

    static INIT_IX: InstructionDescriptor = InstructionDescriptor {
        name: "initialize",
        tag: IX_INIT,
        discriminator: &[IX_INIT],
        args: &[],
        accounts: &[],
        remaining_accounts: None,
        capabilities: &["CreatesAccount", "MutatesState"],
        policy_pack: "COMPACT_VAULT_WRITE",
        receipt_expected: false,
        strict_writes: false,
        write_ranges: &[],
        parametric_write_ranges: &[],
        mutation_complete: false,
        lamport_accounts: &[],
        cu_estimate: 0,
    };

    static DEPOSIT_IX: InstructionDescriptor = InstructionDescriptor {
        name: "deposit",
        tag: IX_DEPOSIT,
        discriminator: &[IX_DEPOSIT],
        args: &[],
        accounts: &[],
        remaining_accounts: None,
        capabilities: &["MutatesState"],
        policy_pack: "COMPACT_VAULT_WRITE",
        receipt_expected: false,
        strict_writes: false,
        write_ranges: &[],
        parametric_write_ranges: &[],
        mutation_complete: false,
        lamport_accounts: &[],
        cu_estimate: 0,
    };

    static INSTRUCTIONS: [InstructionDescriptor; 2] = [INIT_IX, DEPOSIT_IX];

    fn manifest() -> ProgramManifest {
        ProgramManifest {
            name: "hopper_compact_vault",
            version: "0.2.1",
            description: "Devnet-ready proof for a 1-byte compact Hopper account.",
            layouts: &VAULT_LAYOUTS,
            layout_metadata: &[],
            instructions: &INSTRUCTIONS,
            events: &[],
            policies: &[],
            compatibility_pairs: &[],
            tooling_hints: &[
                "account_encoding=compact",
                "compact_body_offset=1",
                "layout_fingerprint_source=manifest_or_idl",
            ],
            contexts: &[],
        }
    }

    #[test]
    fn compact_vault_is_one_byte_header_plus_body() {
        // disc(1) + authority(32) + balance(8) = 41, NOT 16 + 40.
        assert_eq!(Vault::BODY_SIZE, 40);
        assert_eq!(Vault::COMPACT_LEN, 41);
        assert_eq!(Vault::MIN_SIZE, 41);
        assert_eq!(Vault::DISC, VAULT_DISC);
        assert_eq!(Vault::LAYOUT_ID, [67, 113, 65, 144, 124, 9, 52, 79]);
        // The macro-generated absolute offset folds in the single disc byte.
        assert_eq!(Vault::AUTHORITY_ABS_OFFSET, 1);
        assert_eq!(Vault::BALANCE_ABS_OFFSET, 33);
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

    #[test]
    fn generated_metadata_carries_compact_fingerprint() {
        let manifest = manifest();
        let manifest_json = ManifestJson(&manifest).to_string();
        let idl_json = IdlJsonFromManifest(&manifest).to_string();
        let ts_accounts = TsAccounts(&manifest).to_string();
        let kt_accounts = KtAccounts(&manifest).to_string();

        assert!(manifest_json.contains("\"layoutId\": \"437141907c09344f\""));
        assert!(manifest_json.contains("\"totalSize\": 41"));
        assert!(manifest_json.contains("\"offset\": 1"));
        assert!(manifest_json.contains("\"offset\": 33"));
        assert!(manifest_json.contains("layout_fingerprint_source=manifest_or_idl"));

        assert!(idl_json.contains("\"layoutId\": \"437141907c09344f\""));
        assert!(idl_json.contains("\"totalSize\": 41"));

        assert!(ts_accounts
            .contains("export const VAULT_ACCOUNT_ENCODING: AccountEncoding = \"compact\";"));
        assert!(ts_accounts.contains("export const VAULT_LAYOUT_ID = \"437141907c09344f\";"));
        assert!(ts_accounts.contains("assertCompactLayout(data, VAULT_LAYOUT);"));
        assert!(ts_accounts.contains("new PublicKey(data.slice(1, 33))"));
        assert!(ts_accounts.contains("view.getBigUint64(33, true)"));
        assert!(!ts_accounts.contains("assertLayoutId(data, VAULT_LAYOUT_ID);"));

        assert!(kt_accounts
            .contains("val VAULT_ACCOUNT_ENCODING: AccountEncoding = AccountEncoding.Compact"));
        assert!(kt_accounts.contains("const val VAULT_LAYOUT_ID: String = \"437141907c09344f\""));
        assert!(kt_accounts.contains("assertCompactLayout(data, VAULT_LAYOUT)"));
        assert!(kt_accounts.contains("PublicKey(data.copyOfRange(1, 33))"));
        assert!(kt_accounts.contains("ByteBuffer.wrap(data, 33, 8)"));
        assert!(!kt_accounts.contains("assertLayoutId(data, VAULT_LAYOUT_ID)"));
    }

    #[test]
    fn one_descriptor_feeds_loader_registry_and_offsets() {
        // The macro emits a single `LayoutDescriptor::DESCRIPTOR` that the
        // hot-path loader, the Tier-2 registry row, and the field offsets
        // all read from. Prove they agree.
        let d = <Vault as LayoutDescriptor>::DESCRIPTOR;
        assert_eq!(d.disc, VAULT_DISC);
        assert!(d.is_compact());
        assert_eq!(d.body_offset as u32, Vault::AUTHORITY_ABS_OFFSET);
        assert_eq!(d.min_size as usize, Vault::COMPACT_LEN);

        // The registry row is built from the same descriptor.
        assert_eq!(d.registry_entry(), Vault::registry_entry());

        // Hot-path validation reads no registry: just len + disc.
        let mut buf = [0u8; 41];
        buf[0] = VAULT_DISC;
        assert!(<Vault as LayoutDescriptor>::validate_hot(&buf).is_ok());
        buf[0] = VAULT_DISC ^ 0xFF;
        assert!(<Vault as LayoutDescriptor>::validate_hot(&buf).is_err());
    }

    #[test]
    fn governed_redeploy_against_onchain_registry() {
        // Publish the current registry on chain.
        let schema_hash = [7u8; 32];
        let mut buf = [0u8; 512];
        let n = program_registry(&mut buf, &schema_hash).unwrap();
        let onchain = read_registry(&buf[..n]).unwrap();

        // Redeploying the same descriptor set is Unchanged and always allowed.
        let descriptors = [<Vault as LayoutDescriptor>::DESCRIPTOR];
        let compat = diff_descriptors_vs_registry(&descriptors, &onchain);
        assert_eq!(compat, RegistryCompat::Unchanged);
        assert!(ManifestProfile::Governed.permits_upgrade(compat));
    }

    #[test]
    fn one_state_drives_loader_fingerprint_datasize_and_idl() {
        // The single `#[hopper::state(compact, disc = 1)]` declaration above
        // produces, with no extra glue: the hot-path loader, the Tier-2
        // registry row, a client decode fingerprint, a data-size budget, and
        // an IDL/Codama node -- all from one descriptor.

        // Client decode fingerprint: deterministic, embeddable as hex.
        let fp = <Vault as LayoutDescriptor>::fingerprint();
        assert_eq!(fp, <Vault as LayoutDescriptor>::fingerprint());
        assert_eq!(fp.to_hex().len(), 32);

        // Transaction builders size loaded account data from the descriptor.
        let descriptors = [<Vault as LayoutDescriptor>::DESCRIPTOR];
        assert_eq!(
            min_loaded_data_size(&descriptors),
            Vault::COMPACT_LEN as u64
        );
        // No dynamic tail here, so headroom is not applied; margin still adds.
        assert_eq!(
            recommend_loaded_data_limit(&descriptors, 1024, 256),
            Vault::COMPACT_LEN as u64 + 256
        );

        // Cost model: a 41-byte compact vault is cheap to copy-on-write.
        let profile = <Vault as LayoutDescriptor>::cost_profile();
        assert_eq!(profile.class, SizeClass::Small);
        assert!(!profile.growable);
        assert_eq!(
            <Vault as LayoutDescriptor>::DESCRIPTOR.cost_lint(),
            CostLint::Ok
        );

        // IDL/Codama node carries the same identity a client checks.
        let node = <Vault as LayoutDescriptor>::idl_node();
        assert_eq!(node.name, "Vault");
        assert_eq!(node.disc, VAULT_DISC);
        assert_eq!(node.kind, LayoutKind::Compact);
        assert_eq!(node.fingerprint, fp);
    }
}
