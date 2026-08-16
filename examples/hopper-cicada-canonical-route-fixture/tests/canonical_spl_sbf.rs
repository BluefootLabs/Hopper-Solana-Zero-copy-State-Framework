//! Direct SBF smoke proof for the route fixture.
//!
//! Cicada's lifecycle suite supplies the end-to-end policy and rollback proof.
//! This focused test pins the route side of that contract: the compiled route
//! invokes Mollusk's canonical SPL Token ELF twice for an honest swap and uses
//! that same processor for an ownership-valid hostile `SetAuthority` request.

use std::path::PathBuf;

use hopper_cicada_canonical_route_fixture::{
    ROUTE_CANONICAL_MUTATE_SOURCE_POLICY, ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN,
    ROUTE_CANONICAL_SWAP,
};
use hopper_test::LiteSvmHarness;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

fn route_elf_stem() -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("target");
    path.push("deploy");
    path.push("hopper_cicada_canonical_route_fixture");
    path.to_string_lossy().into_owned()
}

fn mint_account(
    token_program: &Pubkey,
    decimals: u8,
    supply: u64,
    authority: Option<&Pubkey>,
) -> Account {
    let mut data = vec![0u8; 82];
    if let Some(authority) = authority {
        data[..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..36].copy_from_slice(&authority.to_bytes());
    }
    data[36..44].copy_from_slice(&supply.to_le_bytes());
    data[44] = decimals;
    data[45] = 1;
    Account {
        lamports: 10_000_000,
        data,
        owner: *token_program,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_account(token_program: &Pubkey, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Account {
    let mut data = vec![0u8; 165];
    data[..32].copy_from_slice(&mint.to_bytes());
    data[32..64].copy_from_slice(&owner.to_bytes());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    Account {
        lamports: 10_000_000,
        data,
        owner: *token_program,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_amount(account: &Account) -> u64 {
    u64::from_le_bytes(account.data[64..72].try_into().unwrap())
}

fn token_authority(account: &Account) -> Pubkey {
    Pubkey::new_from_array(account.data[32..64].try_into().unwrap())
}

struct RouteFixture {
    svm: LiteSvmHarness,
    route_program: Pubkey,
    token_program: Pubkey,
    source: Pubkey,
    input_mint: Pubkey,
    input_sink: Pubkey,
    vault_authority: Pubkey,
    output_reserve: Pubkey,
    output_mint: Pubkey,
    destination: Pubkey,
    liquidity_authority: Pubkey,
    accounts: Vec<(Pubkey, Account)>,
}

impl RouteFixture {
    fn new() -> Option<Self> {
        let route_program = Pubkey::new_unique();
        let mut svm = LiteSvmHarness::load(&route_program, &route_elf_stem())?;
        let token_program = mollusk_svm_programs_token::token::ID;
        mollusk_svm_programs_token::token::add_program(svm.mollusk_mut());
        svm.capture_logs();

        let source = Pubkey::new_unique();
        let input_mint = Pubkey::new_unique();
        let input_sink = Pubkey::new_unique();
        let vault_authority = Pubkey::new_unique();
        let output_reserve = Pubkey::new_unique();
        let output_mint = Pubkey::new_unique();
        let destination = Pubkey::new_unique();
        let liquidity_authority = Pubkey::new_unique();
        let accounts = vec![
            (
                source,
                token_account(&token_program, &input_mint, &vault_authority, 100),
            ),
            (input_mint, mint_account(&token_program, 6, 100, None)),
            (
                input_sink,
                token_account(&token_program, &input_mint, &liquidity_authority, 0),
            ),
            (
                vault_authority,
                Account::new(1_000_000, 0, &Pubkey::default()),
            ),
            (
                output_reserve,
                token_account(&token_program, &output_mint, &liquidity_authority, 200),
            ),
            (
                output_mint,
                mint_account(&token_program, 6, 200, Some(&liquidity_authority)),
            ),
            (
                destination,
                token_account(&token_program, &output_mint, &Pubkey::new_unique(), 0),
            ),
            (
                liquidity_authority,
                Account::new(1_000_000, 0, &Pubkey::default()),
            ),
            (token_program, mollusk_svm_programs_token::token::account()),
        ];
        Some(Self {
            svm,
            route_program,
            token_program,
            source,
            input_mint,
            input_sink,
            vault_authority,
            output_reserve,
            output_mint,
            destination,
            liquidity_authority,
            accounts,
        })
    }

    fn instruction(&self, command: u8, input: u64, output: u64) -> Instruction {
        let mut data = Vec::with_capacity(19);
        data.push(command);
        data.extend_from_slice(&input.to_le_bytes());
        data.extend_from_slice(&output.to_le_bytes());
        data.push(6);
        data.push(6);
        Instruction::new_with_bytes(
            self.route_program,
            &data,
            vec![
                AccountMeta::new(self.source, false),
                AccountMeta::new_readonly(self.input_mint, false),
                AccountMeta::new(self.input_sink, false),
                AccountMeta::new_readonly(self.vault_authority, true),
                AccountMeta::new(self.output_reserve, false),
                if command == ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN {
                    AccountMeta::new(self.output_mint, false)
                } else {
                    AccountMeta::new_readonly(self.output_mint, false)
                },
                AccountMeta::new(self.destination, false),
                AccountMeta::new_readonly(self.liquidity_authority, true),
                AccountMeta::new_readonly(self.token_program, false),
            ],
        )
    }
}

#[test]
fn compiled_supply_neutral_mint_and_burn_use_canonical_authority() {
    let Some(fixture) = RouteFixture::new() else {
        eprintln!("SKIPPED: build the canonical route SBF artifact first");
        return;
    };
    let instruction = fixture.instruction(ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN, 60, 95);
    let result = fixture.svm.process(&instruction, &fixture.accounts);
    assert!(
        result.succeeded(),
        "the mint-authority round trip is legal; Cicada must deny writable mint delegation",
    );
    assert_eq!(
        token_amount(result.raw().get_account(&fixture.destination).unwrap()),
        95,
    );
    assert_eq!(
        token_amount(result.raw().get_account(&fixture.output_reserve).unwrap()),
        105,
    );
    assert_eq!(
        u64::from_le_bytes(
            result.raw().get_account(&fixture.output_mint).unwrap().data[36..44]
                .try_into()
                .unwrap(),
        ),
        200,
    );
    let mint_before = &fixture
        .accounts
        .iter()
        .find(|(address, _)| address == &fixture.output_mint)
        .expect("output mint fixture")
        .1
        .data;
    assert_eq!(
        &result.raw().get_account(&fixture.output_mint).unwrap().data,
        mint_before,
        "MintToChecked plus BurnChecked must restore every mint byte",
    );

    let canonical_invocation = format!("Program {} invoke [2]", fixture.token_program);
    assert_eq!(
        fixture
            .svm
            .logs()
            .iter()
            .filter(|line| *line == &canonical_invocation)
            .count(),
        4,
        "both transfers, MintToChecked, and BurnChecked must enter the canonical token ELF",
    );
}

#[test]
fn compiled_route_executes_both_legs_through_canonical_spl_token() {
    let Some(fixture) = RouteFixture::new() else {
        eprintln!("SKIPPED: build the canonical route SBF artifact first");
        return;
    };
    let instruction = fixture.instruction(ROUTE_CANONICAL_SWAP, 60, 95);
    let result = fixture.svm.process(&instruction, &fixture.accounts);
    assert!(result.succeeded(), "canonical swap failed");

    assert_eq!(
        token_amount(result.raw().get_account(&fixture.source).unwrap()),
        40
    );
    assert_eq!(
        token_amount(result.raw().get_account(&fixture.input_sink).unwrap()),
        60,
    );
    assert_eq!(
        token_amount(result.raw().get_account(&fixture.output_reserve).unwrap()),
        105,
    );
    assert_eq!(
        token_amount(result.raw().get_account(&fixture.destination).unwrap()),
        95,
    );

    let canonical_invocation = format!("Program {} invoke [2]", fixture.token_program);
    assert_eq!(
        fixture
            .svm
            .logs()
            .iter()
            .filter(|line| *line == &canonical_invocation)
            .count(),
        2,
        "each swap leg must enter Mollusk's canonical SPL Token ELF",
    );
}

#[test]
fn compiled_hostile_policy_change_is_canonical_and_owner_authorized() {
    let Some(fixture) = RouteFixture::new() else {
        eprintln!("SKIPPED: build the canonical route SBF artifact first");
        return;
    };
    let instruction = fixture.instruction(ROUTE_CANONICAL_MUTATE_SOURCE_POLICY, 60, 95);
    let result = fixture.svm.process(&instruction, &fixture.accounts);
    assert!(
        result.succeeded(),
        "the route-level request is legal; Cicada must be the layer that rejects its policy effect",
    );
    assert_eq!(
        token_authority(result.raw().get_account(&fixture.source).unwrap()),
        fixture.liquidity_authority,
    );

    let canonical_invocation = format!("Program {} invoke [2]", fixture.token_program);
    assert_eq!(
        fixture
            .svm
            .logs()
            .iter()
            .filter(|line| *line == &canonical_invocation)
            .count(),
        3,
        "two transfers and SetAuthority must all enter the canonical token ELF",
    );
}
