//! Differential checks against the canonical SPL interface, not copied encoders.
use hopper_runtime::{Address, ProgramError};
use hopper_token_2022::{MintConfig, MintExtension as E, MintPlan, MintProgram};
use solana_pubkey::Pubkey;
use spl_token_2022_interface::{extension as spl, instruction as ix, state::Mint};

const AUTHORITY: Address = Address::new_from_array([31; 32]);
const DESTINATION: Address = Address::new_from_array([42; 32]);

fn config() -> MintConfig<'static> {
    MintConfig {
        decimals: 9,
        mint_authority: &AUTHORITY,
        freeze_authority: Some(&DESTINATION),
    }
}

fn all_extensions() -> [E<'static>; 6] {
    [
        E::TransferFeeConfig {
            authority: Some(&AUTHORITY),
            withdraw_authority: Some(&DESTINATION),
            basis_points: 250,
            maximum_fee: u64::MAX,
        },
        E::MintCloseAuthority(Some(&AUTHORITY)),
        E::NonTransferable,
        E::PermanentDelegate(&DESTINATION),
        E::TransferHook {
            authority: Some(&AUTHORITY),
            program_id: Some(&DESTINATION),
        },
        E::MetadataPointer {
            authority: Some(&AUTHORITY),
            metadata_address: Some(&DESTINATION),
        },
    ]
}

#[test]
fn every_supported_extension_subset_has_canonical_space() {
    let all = all_extensions();
    let kinds = [
        spl::ExtensionType::TransferFeeConfig,
        spl::ExtensionType::MintCloseAuthority,
        spl::ExtensionType::NonTransferable,
        spl::ExtensionType::PermanentDelegate,
        spl::ExtensionType::TransferHook,
        spl::ExtensionType::MetadataPointer,
    ];
    for mask in 0..64 {
        let extensions: Vec<_> = all
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, e)| *e)
            .collect();
        let expected: Vec<_> = kinds
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, e)| *e)
            .collect();
        let plan = MintPlan::new(MintProgram::Token2022, config(), &extensions).unwrap();
        assert_eq!(
            plan.space(),
            spl::ExtensionType::try_calculate_account_len::<Mint>(&expected).unwrap(),
            "subset {mask}"
        );
        assert_eq!(plan.check_space(plan.space()), Ok(()));
        assert_eq!(
            plan.check_space(plan.space() - 1),
            Err(ProgramError::InvalidAccountData)
        );
        assert_eq!(
            plan.check_space(plan.space() + 1),
            Err(ProgramError::InvalidAccountData)
        );
        assert!(MintPlan::new(MintProgram::Legacy, config(), &extensions).is_ok() == (mask == 0));
    }
}

#[test]
fn wire_bytes_match_canonical_instruction_constructors() {
    let program = spl_token_2022_interface::id();
    let mint = Pubkey::new_from_array([17; 32]);
    let authority = Pubkey::new_from_array(*AUTHORITY.as_array());
    let destination = Pubkey::new_from_array(*DESTINATION.as_array());
    for has_authority in [false, true] {
        for has_destination in [false, true] {
            let a = has_authority.then_some(&AUTHORITY);
            let d = has_destination.then_some(&DESTINATION);
            let pa = has_authority.then_some(&authority);
            let pd = has_destination.then_some(&destination);
            for (bps, max_fee) in [(0, 0), (250, u64::MAX), (10_000, 1)] {
                let extension = E::TransferFeeConfig {
                    authority: a,
                    withdraw_authority: d,
                    basis_points: bps,
                    maximum_fee: max_fee,
                };
                let canonical = spl::transfer_fee::instruction::initialize_transfer_fee_config(
                    &program, &mint, pa, pd, bps, max_fee,
                )
                .unwrap();
                assert_eq!(
                    extension.instruction_data().unwrap().as_bytes(),
                    canonical.data
                );
            }
            for (extension, canonical) in [
                (
                    E::MintCloseAuthority(a),
                    ix::initialize_mint_close_authority(&program, &mint, pa).unwrap(),
                ),
                (
                    E::TransferHook {
                        authority: a,
                        program_id: d,
                    },
                    spl::transfer_hook::instruction::initialize(
                        &program,
                        &mint,
                        pa.copied(),
                        pd.copied(),
                    )
                    .unwrap(),
                ),
                (
                    E::MetadataPointer {
                        authority: a,
                        metadata_address: d,
                    },
                    spl::metadata_pointer::instruction::initialize(
                        &program,
                        &mint,
                        pa.copied(),
                        pd.copied(),
                    )
                    .unwrap(),
                ),
                (
                    E::NonTransferable,
                    ix::initialize_non_transferable_mint(&program, &mint).unwrap(),
                ),
                (
                    E::PermanentDelegate(&AUTHORITY),
                    ix::initialize_permanent_delegate(&program, &mint, &authority).unwrap(),
                ),
            ] {
                assert_eq!(
                    extension.instruction_data().unwrap().as_bytes(),
                    canonical.data
                );
                assert_eq!(canonical.accounts.len(), 1);
                assert!(!canonical.accounts[0].is_signer);
                assert!(canonical.accounts[0].is_writable);
            }
            for decimals in [0, 9, 255] {
                let config = MintConfig {
                    decimals,
                    mint_authority: &AUTHORITY,
                    freeze_authority: d,
                };
                let canonical =
                    ix::initialize_mint2(&program, &mint, &authority, pd, decimals).unwrap();
                assert_eq!(config.instruction_data().as_bytes(), canonical.data);
            }
        }
    }
}

#[test]
fn plans_reject_duplicates_invalid_fees_and_ambiguous_nullable_addresses() {
    for extension in all_extensions() {
        assert!(matches!(
            MintPlan::new(MintProgram::Token2022, config(), &[extension, extension]),
            Err(ProgramError::InvalidArgument)
        ));
    }
    let zero = Address::new_from_array([0; 32]);
    for extension in [
        E::TransferFeeConfig {
            authority: None,
            withdraw_authority: None,
            basis_points: 10_001,
            maximum_fee: 0,
        },
        E::TransferFeeConfig {
            authority: Some(&zero),
            withdraw_authority: None,
            basis_points: 0,
            maximum_fee: 0,
        },
        E::TransferFeeConfig {
            authority: None,
            withdraw_authority: Some(&zero),
            basis_points: 0,
            maximum_fee: 0,
        },
        E::MintCloseAuthority(Some(&zero)),
        E::PermanentDelegate(&zero),
        E::TransferHook {
            authority: Some(&zero),
            program_id: None,
        },
        E::TransferHook {
            authority: None,
            program_id: Some(&zero),
        },
        E::MetadataPointer {
            authority: Some(&zero),
            metadata_address: None,
        },
        E::MetadataPointer {
            authority: None,
            metadata_address: Some(&zero),
        },
    ] {
        assert!(matches!(
            MintPlan::new(MintProgram::Token2022, config(), &[extension]),
            Err(ProgramError::InvalidArgument)
        ));
        assert!(matches!(
            extension.instruction_data(),
            Err(ProgramError::InvalidArgument)
        ));
    }
}
