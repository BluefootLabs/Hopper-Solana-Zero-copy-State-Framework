//! Differential tests: every token builder's instruction data and account
//! metas, captured through its `TokenInstruction::emit`, against the
//! canonical `spl-token-2022-interface` constructors. The same `emit` feeds
//! the CPI and the `TokenBatch`, so this covers both paths.

use crate::account::AccountView;
use crate::address::Address;
use crate::instruction::InstructionAccount;
use crate::token::*;
use crate::token_2022_ix::*;
use crate::ProgramResult;
use hopper_native::{
    AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
};
use solana_pubkey::Pubkey;
use spl_token_2022_interface::extension::ExtensionType;
use spl_token_2022_interface::instruction as ix;
use spl_token_2022_interface::state::AccountState;
use spl_token_2022_interface::{extension as spl, id as token_2022_id};
use std::boxed::Box;
use std::vec::Vec;

/// One captured meta: address bytes, writable, signer.
type Meta = ([u8; 32], bool, bool);

#[derive(Default)]
struct Capture {
    data: Vec<u8>,
    metas: Vec<Meta>,
}

impl<'a> TokenSink<'a> for Capture {
    fn emit<const N: usize>(
        &mut self,
        data: &[u8],
        accounts: [InstructionAccount<'a>; N],
        views: [&'a AccountView<'a>; N],
        trailing: &[Trailing<'_, 'a>],
    ) -> ProgramResult {
        self.data = data.to_vec();
        self.metas = accounts
            .iter()
            .map(|m| (*m.address.as_array(), m.is_writable, m.is_signer))
            .collect();
        for (meta, view) in accounts.iter().zip(views.iter()) {
            assert_eq!(meta.address, view.address(), "meta and view disagree");
        }
        for run in trailing {
            for view in run.views {
                self.metas
                    .push((*view.address().as_array(), run.writable, run.signer));
            }
        }
        Ok(())
    }
}

fn leak_account(address: [u8; 32], signer: bool) -> &'static AccountView<'static> {
    let backing: &'static mut [u64] = Vec::leak(std::vec![0u64; RuntimeAccount::SIZE.div_ceil(8)]);
    let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: test helper writes a valid RuntimeAccount header into leaked
    // backing memory that lives for the rest of the process.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: u8::from(signer),
            is_writable: 1,
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array(address),
            owner: NativeAddress::new_from_array(TOKEN_2022_PROGRAM_ID.to_bytes()),
            lamports: 1,
            data_len: 0,
        });
    }
    // SAFETY: `raw` points at the initialized RuntimeAccount header.
    let backend = unsafe { NativeAccountView::new_unchecked(raw) };
    Box::leak(Box::new(AccountView::from_backend(backend)))
}

fn capture<'a>(
    instruction: &impl TokenInstruction<'a>,
    multisig_signers: &[&'a AccountView<'a>],
) -> Capture {
    let mut sink = Capture::default();
    instruction.emit(multisig_signers, &mut sink).unwrap();
    sink
}

fn canonical_metas(instruction: &solana_instruction::Instruction) -> Vec<Meta> {
    instruction
        .accounts
        .iter()
        .map(|m| (m.pubkey.to_bytes(), m.is_writable, m.is_signer))
        .collect()
}

fn same(captured: Capture, canonical: solana_instruction::Instruction, label: &str) {
    assert_eq!(captured.data, canonical.data, "{label}: data");
    assert_eq!(
        captured.metas,
        canonical_metas(&canonical),
        "{label}: metas"
    );
}

struct Fixture {
    mint: &'static AccountView<'static>,
    account: &'static AccountView<'static>,
    destination: &'static AccountView<'static>,
    authority: &'static AccountView<'static>,
    payer: &'static AccountView<'static>,
    system: &'static AccountView<'static>,
    signers: [&'static AccountView<'static>; 2],
}

impl Fixture {
    fn new() -> Self {
        Self {
            mint: leak_account([1; 32], false),
            account: leak_account([2; 32], false),
            destination: leak_account([3; 32], false),
            authority: leak_account([4; 32], true),
            payer: leak_account([5; 32], true),
            system: leak_account([0; 32], false),
            signers: [leak_account([6; 32], true), leak_account([7; 32], true)],
        }
    }
}

fn pk(view: &AccountView<'_>) -> Pubkey {
    Pubkey::new_from_array(*view.address().as_array())
}

fn addr(byte: u8) -> Address {
    Address::new_from_array([byte; 32])
}

fn pubkey(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

#[test]
fn administrative_builders_match_the_canonical_constructors() {
    let f = Fixture::new();
    let program = token_2022_id();
    let mint_authority = addr(9);
    let freeze = addr(10);
    let rent = leak_account(
        crate::__decode_base58_32("SysvarRent111111111111111111111111111111111"),
        false,
    );
    let freeze_pk = pubkey(10);

    for freeze_authority in [None, Some(&freeze)] {
        same(
            capture(
                &InitializeMint {
                    mint: f.mint,
                    rent_sysvar: rent,
                    decimals: 6,
                    mint_authority: &mint_authority,
                    freeze_authority,
                },
                &[],
            ),
            ix::initialize_mint(
                &program,
                &pk(f.mint),
                &pubkey(9),
                freeze_authority.map(|_| &freeze_pk),
                6,
            )
            .unwrap(),
            "initialize_mint",
        );
    }

    let members: Vec<Pubkey> = f.signers.iter().map(|v| pk(v)).collect();
    let member_refs: Vec<&Pubkey> = members.iter().collect();
    same(
        capture(
            &InitializeMultisig {
                multisig: f.account,
                rent_sysvar: rent,
                signers: &f.signers,
                m: 2,
            },
            &[],
        ),
        ix::initialize_multisig(&program, &pk(f.account), &member_refs, 2).unwrap(),
        "initialize_multisig",
    );
    same(
        capture(
            &InitializeMultisig2 {
                multisig: f.account,
                signers: &f.signers,
                m: 1,
            },
            &[],
        ),
        ix::initialize_multisig2(&program, &pk(f.account), &member_refs, 1).unwrap(),
        "initialize_multisig2",
    );
    same(
        capture(&InitializeImmutableOwner { account: f.account }, &[]),
        ix::initialize_immutable_owner(&program, &pk(f.account)).unwrap(),
        "initialize_immutable_owner",
    );
    same(
        capture(
            &GetAccountDataSize {
                mint: f.mint,
                extension_types: &[7, 11],
            },
            &[],
        ),
        ix::get_account_data_size(
            &program,
            &pk(f.mint),
            &[ExtensionType::ImmutableOwner, ExtensionType::CpiGuard],
        )
        .unwrap(),
        "get_account_data_size",
    );
    same(
        capture(
            &AmountToUiAmount {
                mint: f.mint,
                amount: 1_234_567,
            },
            &[],
        ),
        ix::amount_to_ui_amount(&program, &pk(f.mint), 1_234_567).unwrap(),
        "amount_to_ui_amount",
    );
    same(
        capture(
            &UiAmountToAmount {
                mint: f.mint,
                ui_amount: "1.234567",
            },
            &[],
        ),
        ix::ui_amount_to_amount(&program, &pk(f.mint), "1.234567").unwrap(),
        "ui_amount_to_amount",
    );

    let signer_pks: Vec<Pubkey> = f.signers.iter().map(|v| pk(v)).collect();
    let signer_refs: Vec<&Pubkey> = signer_pks.iter().collect();
    for (multisig, canonical_signers) in [(&[][..], &[][..]), (&f.signers[..], &signer_refs[..])] {
        same(
            capture(
                &WithdrawExcessLamports {
                    source: f.account,
                    destination: f.destination,
                    authority: f.authority,
                },
                multisig,
            ),
            ix::withdraw_excess_lamports(
                &program,
                &pk(f.account),
                &pk(f.destination),
                &pk(f.authority),
                canonical_signers,
            )
            .unwrap(),
            "withdraw_excess_lamports",
        );
        for amount in [None, Some(42)] {
            same(
                capture(
                    &UnwrapLamports {
                        source: f.account,
                        destination: f.destination,
                        authority: f.authority,
                        amount,
                    },
                    multisig,
                ),
                ix::unwrap_lamports(
                    &program,
                    &pk(f.account),
                    &pk(f.destination),
                    &pk(f.authority),
                    canonical_signers,
                    amount,
                )
                .unwrap(),
                "unwrap_lamports",
            );
        }
        same(
            capture(
                &Reallocate {
                    account: f.account,
                    payer: f.payer,
                    system_program: f.system,
                    owner: f.authority,
                    extension_types: &[8],
                },
                multisig,
            ),
            ix::reallocate(
                &program,
                &pk(f.account),
                &pk(f.payer),
                &pk(f.authority),
                canonical_signers,
                &[ExtensionType::MemoTransfer],
            )
            .unwrap(),
            "reallocate",
        );
    }

    let native_mint = leak_account(
        spl_token_2022_interface::native_mint::id().to_bytes(),
        false,
    );
    same(
        capture(
            &CreateNativeMint {
                payer: f.payer,
                native_mint,
                system_program: f.system,
            },
            &[],
        ),
        ix::create_native_mint(&program, &pk(f.payer)).unwrap(),
        "create_native_mint",
    );
    same(
        capture(&InitializeNonTransferableMint { mint: f.mint }, &[]),
        ix::initialize_non_transferable_mint(&program, &pk(f.mint)).unwrap(),
        "initialize_non_transferable_mint",
    );
}

#[test]
fn shared_builders_match_the_canonical_constructors_on_both_programs() {
    let f = Fixture::new();
    let program = token_2022_id();
    let signer_pks: Vec<Pubkey> = f.signers.iter().map(|v| pk(v)).collect();
    let signer_refs: Vec<&Pubkey> = signer_pks.iter().collect();
    for (multisig, canonical_signers) in [(&[][..], &[][..]), (&f.signers[..], &signer_refs[..])] {
        same(
            capture(
                &TransferChecked {
                    from: f.account,
                    mint: f.mint,
                    to: f.destination,
                    authority: f.authority,
                    amount: 7,
                    decimals: 2,
                },
                multisig,
            ),
            ix::transfer_checked(
                &program,
                &pk(f.account),
                &pk(f.mint),
                &pk(f.destination),
                &pk(f.authority),
                canonical_signers,
                7,
                2,
            )
            .unwrap(),
            "transfer_checked",
        );
        same(
            capture(
                &MintToChecked {
                    mint: f.mint,
                    account: f.account,
                    mint_authority: f.authority,
                    amount: 7,
                    decimals: 2,
                },
                multisig,
            ),
            ix::mint_to_checked(
                &program,
                &pk(f.mint),
                &pk(f.account),
                &pk(f.authority),
                canonical_signers,
                7,
                2,
            )
            .unwrap(),
            "mint_to_checked",
        );
        same(
            capture(
                &BurnChecked {
                    account: f.account,
                    mint: f.mint,
                    authority: f.authority,
                    amount: 7,
                    decimals: 2,
                },
                multisig,
            ),
            ix::burn_checked(
                &program,
                &pk(f.account),
                &pk(f.mint),
                &pk(f.authority),
                canonical_signers,
                7,
                2,
            )
            .unwrap(),
            "burn_checked",
        );
        same(
            capture(
                &ApproveChecked {
                    source: f.account,
                    mint: f.mint,
                    delegate: f.destination,
                    authority: f.authority,
                    amount: 7,
                    decimals: 2,
                },
                multisig,
            ),
            ix::approve_checked(
                &program,
                &pk(f.account),
                &pk(f.mint),
                &pk(f.destination),
                &pk(f.authority),
                canonical_signers,
                7,
                2,
            )
            .unwrap(),
            "approve_checked",
        );
        same(
            capture(
                &CloseAccount {
                    account: f.account,
                    destination: f.destination,
                    authority: f.authority,
                },
                multisig,
            ),
            ix::close_account(
                &program,
                &pk(f.account),
                &pk(f.destination),
                &pk(f.authority),
                canonical_signers,
            )
            .unwrap(),
            "close_account",
        );
        same(
            capture(
                &Revoke {
                    source: f.account,
                    authority: f.authority,
                },
                multisig,
            ),
            ix::revoke(
                &program,
                &pk(f.account),
                &pk(f.authority),
                canonical_signers,
            )
            .unwrap(),
            "revoke",
        );
        same(
            capture(
                &FreezeAccount {
                    account: f.account,
                    mint: f.mint,
                    freeze_authority: f.authority,
                },
                multisig,
            ),
            ix::freeze_account(
                &program,
                &pk(f.account),
                &pk(f.mint),
                &pk(f.authority),
                canonical_signers,
            )
            .unwrap(),
            "freeze_account",
        );
        same(
            capture(
                &ThawAccount {
                    account: f.account,
                    mint: f.mint,
                    freeze_authority: f.authority,
                },
                multisig,
            ),
            ix::thaw_account(
                &program,
                &pk(f.account),
                &pk(f.mint),
                &pk(f.authority),
                canonical_signers,
            )
            .unwrap(),
            "thaw_account",
        );
        let new_authority = addr(12);
        for new in [None, Some(&new_authority)] {
            same(
                capture(
                    &SetAuthority {
                        account: f.account,
                        current_authority: f.authority,
                        authority_type: TokenAuthorityType::CloseAccount,
                        new_authority: new,
                    },
                    multisig,
                ),
                ix::set_authority(
                    &program,
                    &pk(f.account),
                    new.map(|_| pubkey(12)).as_ref(),
                    ix::AuthorityType::CloseAccount,
                    &pk(f.authority),
                    canonical_signers,
                )
                .unwrap(),
                "set_authority",
            );
        }
    }
    let owner = addr(13);
    same(
        capture(
            &InitializeAccount3 {
                account: f.account,
                mint: f.mint,
                owner: &owner,
            },
            &[],
        ),
        ix::initialize_account3(&program, &pk(f.account), &pk(f.mint), &pubkey(13)).unwrap(),
        "initialize_account3",
    );
    same(
        capture(&SyncNative { account: f.account }, &[]),
        ix::sync_native(&program, &pk(f.account)).unwrap(),
        "sync_native",
    );
}

#[test]
fn extension_builders_match_the_canonical_constructors() {
    let f = Fixture::new();
    let program = token_2022_id();
    let a = addr(20);
    let b = addr(21);
    let pa = pubkey(20);
    let pb = pubkey(21);
    let signer_pks: Vec<Pubkey> = f.signers.iter().map(|v| pk(v)).collect();
    let signer_refs: Vec<&Pubkey> = signer_pks.iter().collect();
    let sources = [f.account, f.destination];
    let source_pks: Vec<Pubkey> = sources.iter().map(|v| pk(v)).collect();
    let source_refs: Vec<&Pubkey> = source_pks.iter().collect();

    // Initializers: one writable mint, fixed data.
    for (has_a, has_b) in [(false, false), (true, false), (false, true), (true, true)] {
        let oa = has_a.then_some(&a);
        let ob = has_b.then_some(&b);
        let ca = has_a.then_some(pa);
        let cb = has_b.then_some(pb);
        same(
            capture(
                &InitializeTransferFeeConfig {
                    mint: f.mint,
                    transfer_fee_config_authority: oa,
                    withdraw_withheld_authority: ob,
                    transfer_fee_basis_points: 250,
                    maximum_fee: 99,
                },
                &[],
            ),
            spl::transfer_fee::instruction::initialize_transfer_fee_config(
                &program,
                &pk(f.mint),
                ca.as_ref(),
                cb.as_ref(),
                250,
                99,
            )
            .unwrap(),
            "initialize_transfer_fee_config",
        );
        same(
            capture(
                &InitializeMintCloseAuthority {
                    mint: f.mint,
                    close_authority: oa,
                },
                &[],
            ),
            ix::initialize_mint_close_authority(&program, &pk(f.mint), ca.as_ref()).unwrap(),
            "initialize_mint_close_authority",
        );
        same(
            capture(
                &InitializeInterestBearingMint {
                    mint: f.mint,
                    rate_authority: oa,
                    rate: -50,
                },
                &[],
            ),
            spl::interest_bearing_mint::instruction::initialize(&program, &pk(f.mint), ca, -50)
                .unwrap(),
            "initialize_interest_bearing_mint",
        );
        same(
            capture(
                &InitializeTransferHook {
                    mint: f.mint,
                    authority: oa,
                    program_id: ob,
                },
                &[],
            ),
            spl::transfer_hook::instruction::initialize(&program, &pk(f.mint), ca, cb).unwrap(),
            "initialize_transfer_hook",
        );
        same(
            capture(
                &InitializeMetadataPointer {
                    mint: f.mint,
                    authority: oa,
                    metadata_address: ob,
                },
                &[],
            ),
            spl::metadata_pointer::instruction::initialize(&program, &pk(f.mint), ca, cb).unwrap(),
            "initialize_metadata_pointer",
        );
        same(
            capture(
                &InitializeGroupPointer {
                    mint: f.mint,
                    authority: oa,
                    group_address: ob,
                },
                &[],
            ),
            spl::group_pointer::instruction::initialize(&program, &pk(f.mint), ca, cb).unwrap(),
            "initialize_group_pointer",
        );
        same(
            capture(
                &InitializeGroupMemberPointer {
                    mint: f.mint,
                    authority: oa,
                    member_address: ob,
                },
                &[],
            ),
            spl::group_member_pointer::instruction::initialize(&program, &pk(f.mint), ca, cb)
                .unwrap(),
            "initialize_group_member_pointer",
        );
        same(
            capture(
                &InitializeScaledUiAmount {
                    mint: f.mint,
                    authority: oa,
                    multiplier: 2.5,
                },
                &[],
            ),
            spl::scaled_ui_amount::instruction::initialize(&program, &pk(f.mint), ca, 2.5).unwrap(),
            "initialize_scaled_ui_amount",
        );
    }
    same(
        capture(
            &InitializeDefaultAccountState {
                mint: f.mint,
                state: 2,
            },
            &[],
        ),
        spl::default_account_state::instruction::initialize_default_account_state(
            &program,
            &pk(f.mint),
            &AccountState::Frozen,
        )
        .unwrap(),
        "initialize_default_account_state",
    );
    same(
        capture(
            &InitializePermanentDelegate {
                mint: f.mint,
                delegate: &a,
            },
            &[],
        ),
        ix::initialize_permanent_delegate(&program, &pk(f.mint), &pa).unwrap(),
        "initialize_permanent_delegate",
    );
    same(
        capture(
            &InitializePausable {
                mint: f.mint,
                authority: &a,
            },
            &[],
        ),
        spl::pausable::instruction::initialize(&program, &pk(f.mint), &pa).unwrap(),
        "initialize_pausable",
    );
    same(
        capture(
            &InitializePermissionedBurn {
                mint: f.mint,
                authority: &a,
            },
            &[],
        ),
        spl::permissioned_burn::instruction::initialize(&program, &pk(f.mint), &pa).unwrap(),
        "initialize_permissioned_burn",
    );

    // Updates and toggles: target, authority, optional multisig signers.
    for (multisig, cs) in [(&[][..], &[][..]), (&f.signers[..], &signer_refs[..])] {
        let mint = pk(f.mint);
        let account = pk(f.account);
        let auth = pk(f.authority);
        same(
            capture(
                &TransferCheckedWithFee {
                    source: f.account,
                    mint: f.mint,
                    destination: f.destination,
                    authority: f.authority,
                    amount: 100,
                    decimals: 2,
                    fee: 3,
                },
                multisig,
            ),
            spl::transfer_fee::instruction::transfer_checked_with_fee(
                &program,
                &account,
                &mint,
                &pk(f.destination),
                &auth,
                cs,
                100,
                2,
                3,
            )
            .unwrap(),
            "transfer_checked_with_fee",
        );
        same(
            capture(
                &WithdrawWithheldTokensFromMint {
                    mint: f.mint,
                    destination: f.destination,
                    withdraw_withheld_authority: f.authority,
                },
                multisig,
            ),
            spl::transfer_fee::instruction::withdraw_withheld_tokens_from_mint(
                &program,
                &mint,
                &pk(f.destination),
                &auth,
                cs,
            )
            .unwrap(),
            "withdraw_withheld_tokens_from_mint",
        );
        same(
            capture(
                &WithdrawWithheldTokensFromAccounts {
                    mint: f.mint,
                    destination: f.destination,
                    withdraw_withheld_authority: f.authority,
                    sources: &sources,
                },
                multisig,
            ),
            spl::transfer_fee::instruction::withdraw_withheld_tokens_from_accounts(
                &program,
                &mint,
                &pk(f.destination),
                &auth,
                cs,
                &source_refs,
            )
            .unwrap(),
            "withdraw_withheld_tokens_from_accounts",
        );
        same(
            capture(
                &SetTransferFee {
                    mint: f.mint,
                    transfer_fee_config_authority: f.authority,
                    transfer_fee_basis_points: 10,
                    maximum_fee: 20,
                },
                multisig,
            ),
            spl::transfer_fee::instruction::set_transfer_fee(&program, &mint, &auth, cs, 10, 20)
                .unwrap(),
            "set_transfer_fee",
        );
        same(
            capture(
                &UpdateDefaultAccountState {
                    mint: f.mint,
                    freeze_authority: f.authority,
                    state: 1,
                },
                multisig,
            ),
            spl::default_account_state::instruction::update_default_account_state(
                &program,
                &mint,
                &auth,
                cs,
                &AccountState::Initialized,
            )
            .unwrap(),
            "update_default_account_state",
        );
        same(
            capture(
                &EnableMemoTransfer {
                    account: f.account,
                    owner: f.authority,
                },
                multisig,
            ),
            spl::memo_transfer::instruction::enable_required_transfer_memos(
                &program, &account, &auth, cs,
            )
            .unwrap(),
            "enable_memo_transfer",
        );
        same(
            capture(
                &DisableMemoTransfer {
                    account: f.account,
                    owner: f.authority,
                },
                multisig,
            ),
            spl::memo_transfer::instruction::disable_required_transfer_memos(
                &program, &account, &auth, cs,
            )
            .unwrap(),
            "disable_memo_transfer",
        );
        same(
            capture(
                &UpdateInterestRate {
                    mint: f.mint,
                    rate_authority: f.authority,
                    rate: 77,
                },
                multisig,
            ),
            spl::interest_bearing_mint::instruction::update_rate(&program, &mint, &auth, cs, 77)
                .unwrap(),
            "update_interest_rate",
        );
        same(
            capture(
                &EnableCpiGuard {
                    account: f.account,
                    owner: f.authority,
                },
                multisig,
            ),
            spl::cpi_guard::instruction::enable_cpi_guard(&program, &account, &auth, cs).unwrap(),
            "enable_cpi_guard",
        );
        same(
            capture(
                &DisableCpiGuard {
                    account: f.account,
                    owner: f.authority,
                },
                multisig,
            ),
            spl::cpi_guard::instruction::disable_cpi_guard(&program, &account, &auth, cs).unwrap(),
            "disable_cpi_guard",
        );
        for target in [None, Some(&b)] {
            let ct = target.map(|_| pb);
            same(
                capture(
                    &UpdateTransferHook {
                        mint: f.mint,
                        authority: f.authority,
                        program_id: target,
                    },
                    multisig,
                ),
                spl::transfer_hook::instruction::update(&program, &mint, &auth, cs, ct).unwrap(),
                "update_transfer_hook",
            );
            same(
                capture(
                    &UpdateMetadataPointer {
                        mint: f.mint,
                        authority: f.authority,
                        metadata_address: target,
                    },
                    multisig,
                ),
                spl::metadata_pointer::instruction::update(&program, &mint, &auth, cs, ct).unwrap(),
                "update_metadata_pointer",
            );
            same(
                capture(
                    &UpdateGroupPointer {
                        mint: f.mint,
                        authority: f.authority,
                        group_address: target,
                    },
                    multisig,
                ),
                spl::group_pointer::instruction::update(&program, &mint, &auth, cs, ct).unwrap(),
                "update_group_pointer",
            );
            same(
                capture(
                    &UpdateGroupMemberPointer {
                        mint: f.mint,
                        authority: f.authority,
                        member_address: target,
                    },
                    multisig,
                ),
                spl::group_member_pointer::instruction::update(&program, &mint, &auth, cs, ct)
                    .unwrap(),
                "update_group_member_pointer",
            );
        }
        same(
            capture(
                &UpdateScaledUiAmountMultiplier {
                    mint: f.mint,
                    authority: f.authority,
                    multiplier: 3.0,
                    effective_timestamp: 1_700_000_000,
                },
                multisig,
            ),
            spl::scaled_ui_amount::instruction::update_multiplier(
                &program,
                &mint,
                &auth,
                cs,
                3.0,
                1_700_000_000,
            )
            .unwrap(),
            "update_scaled_ui_amount_multiplier",
        );
        same(
            capture(
                &Pause {
                    mint: f.mint,
                    authority: f.authority,
                },
                multisig,
            ),
            spl::pausable::instruction::pause(&program, &mint, &auth, cs).unwrap(),
            "pause",
        );
        same(
            capture(
                &Resume {
                    mint: f.mint,
                    authority: f.authority,
                },
                multisig,
            ),
            spl::pausable::instruction::resume(&program, &mint, &auth, cs).unwrap(),
            "resume",
        );
        same(
            capture(
                &PermissionedBurn {
                    account: f.account,
                    mint: f.mint,
                    permissioned_burn_authority: f.payer,
                    authority: f.authority,
                    amount: 5,
                },
                multisig,
            ),
            spl::permissioned_burn::instruction::burn(
                &program,
                &account,
                &mint,
                &pk(f.payer),
                &auth,
                cs,
                5,
            )
            .unwrap(),
            "permissioned_burn",
        );
        same(
            capture(
                &PermissionedBurnChecked {
                    account: f.account,
                    mint: f.mint,
                    permissioned_burn_authority: f.payer,
                    authority: f.authority,
                    amount: 5,
                    decimals: 1,
                },
                multisig,
            ),
            spl::permissioned_burn::instruction::burn_checked(
                &program,
                &account,
                &mint,
                &pk(f.payer),
                &auth,
                cs,
                5,
                1,
            )
            .unwrap(),
            "permissioned_burn_checked",
        );
    }
    same(
        capture(
            &HarvestWithheldTokensToMint {
                mint: f.mint,
                sources: &sources,
            },
            &[],
        ),
        spl::transfer_fee::instruction::harvest_withheld_tokens_to_mint(
            &program,
            &pk(f.mint),
            &source_refs,
        )
        .unwrap(),
        "harvest_withheld_tokens_to_mint",
    );
}

#[test]
fn mint_plan_extensions_match_canonical_sizes_and_bytes() {
    use crate::token_mint::{MintConfig, MintExtension as E, MintPlan};
    let a = addr(20);
    let b = addr(21);
    let pa = pubkey(20);
    let pb = pubkey(21);
    let program = token_2022_id();
    let mint = pubkey(1);
    let extensions = [
        E::DefaultAccountState(1),
        E::InterestBearing {
            rate_authority: Some(&a),
            rate: 300,
        },
        E::ScaledUiAmount {
            authority: Some(&a),
            multiplier: 2.0,
        },
        E::Pausable(&a),
        E::GroupPointer {
            authority: Some(&a),
            group_address: Some(&b),
        },
        E::GroupMemberPointer {
            authority: Some(&a),
            member_address: None,
        },
        E::PermissionedBurn(&a),
    ];
    let kinds = [
        ExtensionType::DefaultAccountState,
        ExtensionType::InterestBearingConfig,
        ExtensionType::ScaledUiAmount,
        ExtensionType::Pausable,
        ExtensionType::GroupPointer,
        ExtensionType::GroupMemberPointer,
        ExtensionType::PermissionedBurn,
    ];
    let canonical = [
        spl::default_account_state::instruction::initialize_default_account_state(
            &program,
            &mint,
            &AccountState::Initialized,
        )
        .unwrap(),
        spl::interest_bearing_mint::instruction::initialize(&program, &mint, Some(pa), 300)
            .unwrap(),
        spl::scaled_ui_amount::instruction::initialize(&program, &mint, Some(pa), 2.0).unwrap(),
        spl::pausable::instruction::initialize(&program, &mint, &pa).unwrap(),
        spl::group_pointer::instruction::initialize(&program, &mint, Some(pa), Some(pb)).unwrap(),
        spl::group_member_pointer::instruction::initialize(&program, &mint, Some(pa), None)
            .unwrap(),
        spl::permissioned_burn::instruction::initialize(&program, &mint, &pa).unwrap(),
    ];
    for ((extension, kind), canonical) in extensions.iter().zip(kinds).zip(canonical.iter()) {
        assert_eq!(
            extension.instruction_data().unwrap().as_bytes(),
            canonical.data,
            "{kind:?} bytes"
        );
        assert_eq!(
            extension.value_len(),
            ExtensionType::try_calculate_account_len::<spl_token_2022_interface::state::Mint>(&[
                kind
            ])
            .unwrap()
                - 166
                - 4,
            "{kind:?} value length"
        );
    }
    let config = MintConfig {
        decimals: 6,
        mint_authority: &a,
        freeze_authority: None,
    };
    let plan = MintPlan::new(TokenProgram::Token2022, config, &extensions).unwrap();
    assert_eq!(
        plan.space(),
        ExtensionType::try_calculate_account_len::<spl_token_2022_interface::state::Mint>(&kinds)
            .unwrap()
    );
    let zero = addr(0);
    for bad in [
        E::DefaultAccountState(0),
        E::ScaledUiAmount {
            authority: None,
            multiplier: -1.0,
        },
        E::Pausable(&zero),
        E::PermissionedBurn(&zero),
        E::GroupPointer {
            authority: Some(&zero),
            group_address: None,
        },
        E::InterestBearing {
            rate_authority: Some(&zero),
            rate: 0,
        },
    ] {
        assert!(MintPlan::new(TokenProgram::Token2022, config, &[bad]).is_err());
    }
}

#[test]
fn token_program_owning_selects_the_program_and_refuses_others() {
    let token_2022 = leak_account([30; 32], false);
    assert_eq!(
        TokenProgram::owning(token_2022),
        Ok(TokenProgram::Token2022)
    );
    let legacy_backing: &'static mut [u64] =
        Vec::leak(std::vec![0u64; RuntimeAccount::SIZE.div_ceil(8)]);
    let raw = legacy_backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: as in `leak_account`, a valid header in leaked memory.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 0,
            is_writable: 1,
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array([31; 32]),
            owner: NativeAddress::new_from_array(TOKEN_PROGRAM_ID.to_bytes()),
            lamports: 1,
            data_len: 0,
        });
    }
    // SAFETY: `raw` points at the initialized header.
    let legacy = AccountView::from_backend(unsafe { NativeAccountView::new_unchecked(raw) });
    assert_eq!(TokenProgram::owning(&legacy), Ok(TokenProgram::Legacy));
    let other_backing: &'static mut [u64] =
        Vec::leak(std::vec![0u64; RuntimeAccount::SIZE.div_ceil(8)]);
    let raw = other_backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: as above.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 0,
            is_writable: 1,
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array([32; 32]),
            owner: NativeAddress::new_from_array([99; 32]),
            lamports: 1,
            data_len: 0,
        });
    }
    // SAFETY: `raw` points at the initialized header.
    let other = AccountView::from_backend(unsafe { NativeAccountView::new_unchecked(raw) });
    assert_eq!(
        TokenProgram::owning(&other),
        Err(crate::error::ProgramError::IncorrectProgramId)
    );
    assert_eq!(
        TokenProgram::from_address(&TOKEN_2022_PROGRAM_ID),
        Some(TokenProgram::Token2022)
    );
    assert_eq!(TokenProgram::from_address(&addr(1)), None);
    assert_eq!(TokenProgram::Legacy.address(), &TOKEN_PROGRAM_ID);
}
