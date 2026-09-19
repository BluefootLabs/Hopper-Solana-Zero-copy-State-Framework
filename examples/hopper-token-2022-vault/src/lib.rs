//! # Hopper Token-2022 Vault Example
//!
//! A Hopper-authored Token-2022 vault flow built entirely on Hopper-owned
//! companion crates and the canonical whole-layout access path.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::hopper_schema::{
    AccountEntry, ArgDescriptor, ArgEncoding, InstructionDescriptor, ProgramManifest,
};
use hopper::prelude::*;
use hopper::systems::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

hopper_layout! {
    /// Minimal Token-2022 treasury state.
    pub struct RewardVault, disc = 41, version = 1 {
        authority:    TypedAddress<Authority> = 32,
        mint:         TypedAddress<Mint>      = 32,
        vault_ata:    TypedAddress<Token>     = 32,
        minted_total: WireU64                 = 8,
        swept_total:  WireU64                 = 8,
        bump:         u8                      = 1,
    }
}

hopper::hopper_manifest! {
    REWARD_VAULT_MANIFEST = RewardVault {
        authority:    TypedAddress<Authority> = 32,
        mint:         TypedAddress<Mint>      = 32,
        vault_ata:    TypedAddress<Token>     = 32,
        minted_total: WireU64                 = 8,
        swept_total:  WireU64                 = 8,
        bump:         u8                      = 1,
    }
}

hopper_error! {
    base = 6400;
    ZeroAmount,
    WrongTokenProgram,
    WrongSystemProgram,
    WrongAssociatedTokenProgram,
    Unauthorized,
    VaultBindingMismatch,
}

const INIT_TAG: u8 = 0;
const PREPARE_TAG: u8 = 1;
const MINT_TAG: u8 = 2;
const SWEEP_TAG: u8 = 3;
const AMOUNT_ARG_LEN: usize = 8;

struct InitAccounts;
impl InitAccounts {
    const PAYER: usize = 0;
    const VAULT_STATE: usize = 1;
    const AUTHORITY: usize = 2;
    const SYSTEM_PROGRAM: usize = 3;
    const LEN: usize = 4;
}

struct PrepareAccounts;
impl PrepareAccounts {
    const PAYER: usize = 0;
    const AUTHORITY: usize = 1;
    const VAULT_STATE: usize = 2;
    const VAULT_ATA: usize = 3;
    const MINT: usize = 4;
    const SYSTEM_PROGRAM: usize = 5;
    const TOKEN_PROGRAM_2022: usize = 6;
    const ASSOCIATED_TOKEN_PROGRAM: usize = 7;
    const LEN: usize = 8;
}

struct MintAccounts;
impl MintAccounts {
    const AUTHORITY: usize = 0;
    const VAULT_STATE: usize = 1;
    const VAULT_ATA: usize = 2;
    const MINT: usize = 3;
    const TOKEN_PROGRAM_2022: usize = 4;
    const LEN: usize = 5;
}

struct SweepAccounts;
impl SweepAccounts {
    const AUTHORITY: usize = 0;
    const VAULT_STATE: usize = 1;
    const VAULT_ATA: usize = 2;
    const DESTINATION_ATA: usize = 3;
    const MINT: usize = 4;
    const TOKEN_PROGRAM_2022: usize = 5;
    const LEN: usize = 6;
}

static INIT_MANIFEST_ACCOUNTS: [AccountEntry; InitAccounts::LEN] = [
    manifest_account("payer", true, true, ""),
    manifest_account("vault_state", true, true, "RewardVault"),
    manifest_account("authority", false, true, ""),
    manifest_account("system_program", false, false, ""),
];

static PREPARE_MANIFEST_ACCOUNTS: [AccountEntry; PrepareAccounts::LEN] = [
    manifest_account("payer", true, true, ""),
    manifest_account("authority", false, true, ""),
    manifest_account("vault_state", true, false, "RewardVault"),
    manifest_account("vault_ata", true, false, ""),
    manifest_account("mint", false, false, ""),
    manifest_account("system_program", false, false, ""),
    manifest_account("token_program_2022", false, false, ""),
    manifest_account("associated_token_program", false, false, ""),
];

static MINT_MANIFEST_ACCOUNTS: [AccountEntry; MintAccounts::LEN] = [
    manifest_account("authority", false, true, ""),
    manifest_account("vault_state", true, false, "RewardVault"),
    manifest_account("vault_ata", true, false, ""),
    manifest_account("mint", true, false, ""),
    manifest_account("token_program_2022", false, false, ""),
];

static SWEEP_MANIFEST_ACCOUNTS: [AccountEntry; SweepAccounts::LEN] = [
    manifest_account("authority", false, true, ""),
    manifest_account("vault_state", true, false, "RewardVault"),
    manifest_account("vault_ata", true, false, ""),
    manifest_account("destination_ata", true, false, ""),
    manifest_account("mint", false, false, ""),
    manifest_account("token_program_2022", false, false, ""),
];

static AMOUNT_ARGS: [ArgDescriptor; 1] = [ArgDescriptor {
    name: "amount",
    canonical_type: "u64",
    size: AMOUNT_ARG_LEN as u16,
    encoding: ArgEncoding::Fixed,
}];

static INSTRUCTION_MANIFESTS: [InstructionDescriptor; 4] = [
    manifest_instruction("init_vault", INIT_TAG, &INIT_MANIFEST_ACCOUNTS, &[]),
    manifest_instruction(
        "prepare_vault_ata",
        PREPARE_TAG,
        &PREPARE_MANIFEST_ACCOUNTS,
        &[],
    ),
    manifest_instruction(
        "mint_rewards",
        MINT_TAG,
        &MINT_MANIFEST_ACCOUNTS,
        &AMOUNT_ARGS,
    ),
    manifest_instruction(
        "sweep_rewards",
        SWEEP_TAG,
        &SWEEP_MANIFEST_ACCOUNTS,
        &AMOUNT_ARGS,
    ),
];

const fn manifest_account(
    name: &'static str,
    writable: bool,
    signer: bool,
    layout_ref: &'static str,
) -> AccountEntry {
    AccountEntry {
        name,
        writable,
        signer,
        layout_ref,
        seeds: &[],
    }
}

const fn manifest_instruction(
    name: &'static str,
    tag: u8,
    accounts: &'static [AccountEntry],
    args: &'static [ArgDescriptor],
) -> InstructionDescriptor {
    InstructionDescriptor {
        name,
        tag,
        discriminator: match tag {
            INIT_TAG => &[INIT_TAG],
            PREPARE_TAG => &[PREPARE_TAG],
            MINT_TAG => &[MINT_TAG],
            SWEEP_TAG => &[SWEEP_TAG],
            _ => &[],
        },
        args,
        accounts,
        remaining_accounts: None,
        capabilities: &[],
        policy_pack: "",
        receipt_expected: false,
        strict_writes: false,
        write_ranges: &[],
        parametric_write_ranges: &[],
        mutation_complete: false,
        lamport_accounts: &[],
        cu_estimate: 0,
    }
}

/// Source-owned schema for this raw-dispatch program.
///
/// It intentionally publishes no policy, receipt, strict-write, or CU claim:
/// those contracts are not enforced by a typed Hopper context in this example.
/// Account order and privilege bits are the exact raw ABI consumed below.
pub static PROGRAM_MANIFEST: ProgramManifest = ProgramManifest {
    name: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    description: env!("CARGO_PKG_DESCRIPTION"),
    layouts: &[REWARD_VAULT_MANIFEST],
    layout_metadata: &[],
    instructions: &INSTRUCTION_MANIFESTS,
    events: &[],
    policies: &[],
    compatibility_pairs: &[],
    tooling_hints: &[],
    contexts: &[],
};

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (tag, remaining) = hopper::hopper_core::dispatch::dispatch_instruction(instruction_data)?;
    match tag {
        INIT_TAG => process_init_vault(program_id, accounts, remaining),
        PREPARE_TAG => process_prepare_vault_ata(program_id, accounts, remaining),
        MINT_TAG => process_mint_rewards(program_id, accounts, remaining),
        SWEEP_TAG => process_sweep_rewards(program_id, accounts, remaining),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn process_init_vault(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < InitAccounts::LEN {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let payer = &accounts[InitAccounts::PAYER];
    let vault_state = &accounts[InitAccounts::VAULT_STATE];
    let authority = &accounts[InitAccounts::AUTHORITY];
    let system_program = &accounts[InitAccounts::SYSTEM_PROGRAM];

    require_payer(payer)?;
    authority.check_signer()?;
    if *system_program.address() != SYSTEM_PROGRAM_ID {
        return Err(WrongSystemProgram.into());
    }
    system_program.check_executable()?;

    hopper_init!(payer, vault_state, system_program, program_id, RewardVault)?;

    let mut vault = RewardVault::load_mut(vault_state, program_id)?;
    let vault = vault.get_mut();
    vault.authority = TypedAddress::from_account(authority);
    vault.mint = TypedAddress::zeroed();
    vault.vault_ata = TypedAddress::zeroed();
    vault.minted_total = WireU64::new(0);
    vault.swept_total = WireU64::new(0);
    vault.bump = 0;

    Ok(())
}

fn process_prepare_vault_ata(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < PrepareAccounts::LEN {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let payer = &accounts[PrepareAccounts::PAYER];
    let authority = &accounts[PrepareAccounts::AUTHORITY];
    let vault_state = &accounts[PrepareAccounts::VAULT_STATE];
    let vault_ata = &accounts[PrepareAccounts::VAULT_ATA];
    let mint = &accounts[PrepareAccounts::MINT];
    let system_program = &accounts[PrepareAccounts::SYSTEM_PROGRAM];
    let token_program_2022 = &accounts[PrepareAccounts::TOKEN_PROGRAM_2022];
    let associated_token_program = &accounts[PrepareAccounts::ASSOCIATED_TOKEN_PROGRAM];

    require_payer(payer)?;
    authority.check_signer()?;
    vault_state.check_writable()?;
    vault_ata.check_writable()?;
    if *system_program.address() != SYSTEM_PROGRAM_ID {
        return Err(WrongSystemProgram.into());
    }
    system_program.check_executable()?;
    if *token_program_2022.address() != TOKEN_2022_PROGRAM_ID {
        return Err(WrongTokenProgram.into());
    }
    token_program_2022.check_executable()?;
    if *associated_token_program.address() != hopper::hopper_associated_token::ATA_PROGRAM_ID {
        return Err(WrongAssociatedTokenProgram.into());
    }
    associated_token_program.check_executable()?;

    // The initializer fixes the authority. Preparing the ATA may bind the
    // mint/account pair exactly once, but can never replace either side of an
    // existing binding. All state authorization is checked before entering an
    // external program.
    let first_binding = {
        let vault = RewardVault::load(vault_state, program_id)?;
        let vault = vault.get();
        if !vault.authority.eq_account(authority) {
            return Err(Unauthorized.into());
        }
        match (vault.mint.is_zero(), vault.vault_ata.is_zero()) {
            (true, true) => true,
            (false, false) => {
                if !vault.mint.eq_account(mint) || !vault.vault_ata.eq_account(vault_ata) {
                    return Err(VaultBindingMismatch.into());
                }
                false
            }
            _ => return Err(ProgramError::InvalidAccountData),
        }
    };
    validate_token_2022_mint(mint, Some(authority))?;

    hopper::hopper_associated_token::CreateIdempotent {
        payer,
        associated_account: vault_ata,
        wallet: authority,
        mint,
        system_program,
        token_program: token_program_2022,
    }
    .invoke()?;

    validate_token_2022_account(vault_ata, mint.address(), Some(authority.address()))?;

    if first_binding {
        let mut vault = RewardVault::load_mut(vault_state, program_id)?;
        let vault = vault.get_mut();
        if !vault.authority.eq_account(authority)
            || !vault.mint.is_zero()
            || !vault.vault_ata.is_zero()
        {
            return Err(VaultBindingMismatch.into());
        }
        vault.mint = TypedAddress::from_account(mint);
        vault.vault_ata = TypedAddress::from_account(vault_ata);
    }

    Ok(())
}

fn process_mint_rewards(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < MintAccounts::LEN || data.len() < AMOUNT_ARG_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }

    let authority = &accounts[MintAccounts::AUTHORITY];
    let vault_state = &accounts[MintAccounts::VAULT_STATE];
    let vault_ata = &accounts[MintAccounts::VAULT_ATA];
    let mint = &accounts[MintAccounts::MINT];
    let token_program_2022 = &accounts[MintAccounts::TOKEN_PROGRAM_2022];
    let amount = read_amount(data)?;

    authority.check_signer()?;
    vault_state.check_writable()?;
    vault_ata.check_writable()?;
    mint.check_writable()?;
    if *token_program_2022.address() != TOKEN_2022_PROGRAM_ID {
        return Err(WrongTokenProgram.into());
    }
    token_program_2022.check_executable()?;

    let next_total = {
        let vault = RewardVault::load(vault_state, program_id)?;
        let vault = vault.get();
        if !vault.authority.eq_account(authority) {
            return Err(Unauthorized.into());
        }
        if !vault.vault_ata.eq_account(vault_ata) || !vault.mint.eq_account(mint) {
            return Err(VaultBindingMismatch.into());
        }
        vault
            .minted_total
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?
    };
    validate_token_2022_mint(mint, Some(authority))?;
    validate_token_2022_account(vault_ata, mint.address(), Some(authority.address()))?;

    hopper::hopper_token_2022::MintTo {
        mint,
        account: vault_ata,
        mint_authority: authority,
        amount,
    }
    .invoke()?;

    let mut vault = RewardVault::load_mut(vault_state, program_id)?;
    let vault = vault.get_mut();
    if !vault.authority.eq_account(authority)
        || !vault.vault_ata.eq_account(vault_ata)
        || !vault.mint.eq_account(mint)
    {
        return Err(VaultBindingMismatch.into());
    }
    vault.minted_total = WireU64::new(next_total);

    Ok(())
}

fn process_sweep_rewards(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < SweepAccounts::LEN || data.len() < AMOUNT_ARG_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }

    let authority = &accounts[SweepAccounts::AUTHORITY];
    let vault_state = &accounts[SweepAccounts::VAULT_STATE];
    let vault_ata = &accounts[SweepAccounts::VAULT_ATA];
    let destination_ata = &accounts[SweepAccounts::DESTINATION_ATA];
    let mint = &accounts[SweepAccounts::MINT];
    let token_program_2022 = &accounts[SweepAccounts::TOKEN_PROGRAM_2022];
    let amount = read_amount(data)?;

    authority.check_signer()?;
    vault_state.check_writable()?;
    vault_ata.check_writable()?;
    destination_ata.check_writable()?;
    if *token_program_2022.address() != TOKEN_2022_PROGRAM_ID {
        return Err(WrongTokenProgram.into());
    }
    token_program_2022.check_executable()?;

    let next_total = {
        let vault = RewardVault::load(vault_state, program_id)?;
        let vault = vault.get();
        if !vault.authority.eq_account(authority) {
            return Err(Unauthorized.into());
        }
        if !vault.vault_ata.eq_account(vault_ata) || !vault.mint.eq_account(mint) {
            return Err(VaultBindingMismatch.into());
        }
        vault
            .swept_total
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?
    };
    let decimals = validate_token_2022_mint(mint, None)?;
    validate_token_2022_account(vault_ata, mint.address(), Some(authority.address()))?;
    validate_token_2022_account(destination_ata, mint.address(), None)?;

    interface_transfer_checked_with_program(
        vault_ata,
        mint,
        destination_ata,
        authority,
        token_program_2022,
        amount,
        decimals,
    )?;

    let mut vault = RewardVault::load_mut(vault_state, program_id)?;
    let vault = vault.get_mut();
    if !vault.authority.eq_account(authority) || !vault.vault_ata.eq_account(vault_ata) {
        return Err(VaultBindingMismatch.into());
    }
    vault.swept_total = WireU64::new(next_total);

    Ok(())
}

fn read_amount(data: &[u8]) -> Result<u64, ProgramError> {
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);
    Ok(amount)
}

fn validate_token_2022_mint(
    mint: &AccountView,
    expected_authority: Option<&AccountView>,
) -> Result<u8, ProgramError> {
    if !mint.owned_by(&TOKEN_2022_PROGRAM_ID) {
        return Err(WrongTokenProgram.into());
    }
    let data = mint.try_borrow()?;
    let mint_view = InterfaceMint::from_data(&data, TokenProgramKind::Token2022)?;
    mint_view.assert_initialized()?;
    hopper::hopper_token_2022::check_safe_token_2022_mint(&data)?;
    if let Some(expected) = expected_authority {
        match mint_view.authority()? {
            Some(actual) if actual == expected.address() => {}
            _ => return Err(Unauthorized.into()),
        }
    }
    mint_view.decimals()
}

fn validate_token_2022_account(
    account: &AccountView,
    expected_mint: &Address,
    expected_owner: Option<&Address>,
) -> ProgramResult {
    if !account.owned_by(&TOKEN_2022_PROGRAM_ID) {
        return Err(WrongTokenProgram.into());
    }
    let data = account.try_borrow()?;
    let token = InterfaceTokenAccount::from_data(&data, TokenProgramKind::Token2022)?;
    token.assert_initialized()?;
    token.assert_mint(expected_mint)?;
    if let Some(owner) = expected_owner {
        token.assert_owner(owner)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper::hopper_schema::codama::ManifestJson;

    type ExpectedAccount = (&'static str, bool, bool, &'static str);

    fn assert_accounts(actual: &[AccountEntry], expected: &[ExpectedAccount]) {
        assert_eq!(actual.len(), expected.len());
        for (account, &(name, writable, signer, layout_ref)) in actual.iter().zip(expected) {
            assert_eq!(account.name, name);
            assert_eq!(account.writable, writable, "{name} writable bit");
            assert_eq!(account.signer, signer, "{name} signer bit");
            assert_eq!(account.layout_ref, layout_ref, "{name} layout ref");
            assert!(
                account.seeds.is_empty(),
                "raw ABI has no declared PDA seeds"
            );
        }
    }

    #[test]
    fn raw_dispatch_manifest_matches_runtime_tags_args_and_account_indices() {
        assert_eq!(PROGRAM_MANIFEST.instructions.len(), 4);
        let init = &PROGRAM_MANIFEST.instructions[INIT_TAG as usize];
        let prepare = &PROGRAM_MANIFEST.instructions[PREPARE_TAG as usize];
        let mint = &PROGRAM_MANIFEST.instructions[MINT_TAG as usize];
        let sweep = &PROGRAM_MANIFEST.instructions[SWEEP_TAG as usize];

        for (instruction, tag) in [
            (init, INIT_TAG),
            (prepare, PREPARE_TAG),
            (mint, MINT_TAG),
            (sweep, SWEEP_TAG),
        ] {
            assert_eq!(instruction.tag, tag);
            assert_eq!(instruction.discriminator, &[tag]);
            assert!(instruction.capabilities.is_empty());
            assert_eq!(instruction.policy_pack, "");
            assert!(!instruction.receipt_expected);
            assert!(!instruction.strict_writes);
            assert!(!instruction.mutation_complete);
            assert!(instruction.write_ranges.is_empty());
            assert!(instruction.parametric_write_ranges.is_empty());
            assert!(instruction.lamport_accounts.is_empty());
            assert_eq!(instruction.cu_estimate, 0);
        }

        assert!(init.args.is_empty());
        assert!(prepare.args.is_empty());
        for instruction in [mint, sweep] {
            assert_eq!(instruction.args.len(), 1);
            assert_eq!(instruction.args[0].name, "amount");
            assert_eq!(instruction.args[0].canonical_type, "u64");
            assert_eq!(
                instruction.args[0].fixed_size(),
                Some(AMOUNT_ARG_LEN as u16)
            );
        }

        assert_accounts(
            init.accounts,
            &[
                ("payer", true, true, ""),
                ("vault_state", true, true, "RewardVault"),
                ("authority", false, true, ""),
                ("system_program", false, false, ""),
            ],
        );
        assert_accounts(
            prepare.accounts,
            &[
                ("payer", true, true, ""),
                ("authority", false, true, ""),
                ("vault_state", true, false, "RewardVault"),
                ("vault_ata", true, false, ""),
                ("mint", false, false, ""),
                ("system_program", false, false, ""),
                ("token_program_2022", false, false, ""),
                ("associated_token_program", false, false, ""),
            ],
        );
        assert_accounts(
            mint.accounts,
            &[
                ("authority", false, true, ""),
                ("vault_state", true, false, "RewardVault"),
                ("vault_ata", true, false, ""),
                ("mint", true, false, ""),
                ("token_program_2022", false, false, ""),
            ],
        );
        assert_accounts(
            sweep.accounts,
            &[
                ("authority", false, true, ""),
                ("vault_state", true, false, "RewardVault"),
                ("vault_ata", true, false, ""),
                ("destination_ata", true, false, ""),
                ("mint", false, false, ""),
                ("token_program_2022", false, false, ""),
            ],
        );

        assert_eq!(InitAccounts::LEN, init.accounts.len());
        assert_eq!(PrepareAccounts::LEN, prepare.accounts.len());
        assert_eq!(MintAccounts::LEN, mint.accounts.len());
        assert_eq!(SweepAccounts::LEN, sweep.accounts.len());
        assert_eq!(
            prepare.accounts[PrepareAccounts::ASSOCIATED_TOKEN_PROGRAM].name,
            "associated_token_program"
        );
        assert_eq!(sweep.accounts[SweepAccounts::MINT].name, "mint");
    }

    #[test]
    fn program_manifest_uses_the_runtime_layout_and_no_unenforced_contracts() {
        assert_eq!(PROGRAM_MANIFEST.layouts.len(), 1);
        let layout = &PROGRAM_MANIFEST.layouts[0];
        assert_eq!(layout.name, "RewardVault");
        assert_eq!(layout.disc, RewardVault::DISC);
        assert_eq!(layout.version, RewardVault::VERSION);
        assert_eq!(layout.layout_id, RewardVault::LAYOUT_ID);
        assert_eq!(layout.total_size, RewardVault::LEN);
        assert_eq!(layout.fields.len(), 6);
        let runtime_layout =
            <RewardVault as hopper::hopper_schema::SchemaExport>::layout_manifest();
        assert_eq!(layout.field_count, runtime_layout.field_count);
        for (published, runtime) in layout.fields.iter().zip(runtime_layout.fields) {
            assert_eq!(published.name, runtime.name);
            assert_eq!(published.canonical_type, runtime.canonical_type);
            assert_eq!(published.size, runtime.size);
            assert_eq!(published.offset, runtime.offset);
            assert_eq!(published.intent, runtime.intent);
        }
        assert!(PROGRAM_MANIFEST.policies.is_empty());
        assert!(PROGRAM_MANIFEST.contexts.is_empty());
        assert!(PROGRAM_MANIFEST.events.is_empty());
    }

    #[test]
    fn checked_in_manifest_is_the_source_manifest_rendering() {
        let rendered = ManifestJson(&PROGRAM_MANIFEST).to_string();
        let checked_in = include_str!("../hopper.manifest.json").replace("\r\n", "\n");
        assert_eq!(checked_in.trim_end(), rendered.trim_end());
    }
}
