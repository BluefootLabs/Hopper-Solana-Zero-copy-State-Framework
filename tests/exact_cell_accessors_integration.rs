//! Bound cell accessors use the same invocation selector as the byte policy.
#![cfg(feature = "proc-macros")]

use hopper::layout::write_header;
use hopper::prelude::*;
use hopper_runtime::write_policy::write_policy_violation;
use hopper_svm::{AccountFixture, HopperSvm};

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 81, version = 1)]
pub struct CellLedger {
    pub authority: Address,
    pub balances: [WireU64; 4],
    pub recipients: [Address; 4],
    pub flags: [u8; 3],
}

#[derive(Accounts)]
#[accounts(strict_writes, lamports())]
#[instruction(balance_slot: u16, flag_slot: u8)]
pub struct SelectedCells<'info> {
    #[account(cells(balance_slot; balances, recipients), cells(flag_slot; flags))]
    pub ledger: Account<'info, CellLedger>,
}

// The attribute-context path and the full u32 selector domain must work too.
#[hopper::context(strict_writes, lamports())]
#[instruction(slot: u32)]
pub struct WideSelection {
    #[account(cells(slot; balances))]
    pub ledger: CellLedger,
}

fn fixture() -> (Address, AccountFixture) {
    let program = Address::new_from_array([31; 32]);
    let mut data = vec![0x5a; CellLedger::LEN];
    write_header(
        &mut data,
        CellLedger::DISC,
        CellLedger::VERSION,
        &CellLedger::LAYOUT_ID,
    )
    .unwrap();
    for (slot, value) in [10u64, 20, 30, 40].into_iter().enumerate() {
        let offset = CellLedger::BALANCES_ABS_OFFSET as usize + slot * 8;
        data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    (
        program,
        AccountFixture::with_data(Address::new_from_array([32; 32]), program, 123_456, data)
            .writable(),
    )
}

fn selected_handler<'a>(
    program: &'a Address,
    accounts: &'a [AccountView<'a>],
    data: &'a [u8],
) -> ProgramResult {
    let balance_slot = u16::from_le_bytes([data[0], data[1]]);
    let flag_slot = data[2];
    let mut raw = Context::new(program, accounts, data);
    let mut ctx = SelectedCells::bind_with_args(&mut raw, balance_slot, flag_slot)?;
    let before = ctx.ledger_balances_cell_ref()?.get();
    {
        let mut balance = ctx.ledger_balances_cell_mut()?;
        balance.checked_add_assign(7)?;
    }
    assert_eq!(ctx.ledger_balances_cell_ref()?.get(), before + 7);
    *ctx.ledger_recipients_cell_mut()? = Address::new_from_array([0xb2; 32]);
    *ctx.ledger_flags_cell_mut()? = 0xe1;

    // Generated accessors cannot weaken the ambient policy. Whole columns,
    // neighboring cells, protected fields, and lamports remain refused.
    assert!(matches!(
        ctx.ledger_balances_mut(),
        Err(error) if error == write_policy_violation(0)
    ));
    let neighbor = (balance_slot as u32 + 1) % 4;
    assert!(matches!(
        ctx.raw().segment_mut::<WireU64>(0, CellLedger::BALANCES_ABS_OFFSET + neighbor * 8),
        Err(error) if error == write_policy_violation(0)
    ));
    assert!(matches!(
        ctx.raw().segment_mut::<Address>(0, CellLedger::AUTHORITY_ABS_OFFSET),
        Err(error) if error == write_policy_violation(0)
    ));
    assert_eq!(
        accounts[0].try_set_lamports(0),
        Err(write_policy_violation(0))
    );
    Ok(())
}

#[test]
fn multiple_selectors_change_only_the_selected_typed_cells() {
    for balance_slot in 0..4u16 {
        for flag_slot in 0..3u8 {
            let (program, ledger) = fixture();
            let mut expected = ledger.clone();
            let offset = CellLedger::BALANCES_ABS_OFFSET as usize + balance_slot as usize * 8;
            let balance = (u64::from(balance_slot) + 1) * 10 + 7;
            expected.data[offset..offset + 8].copy_from_slice(&balance.to_le_bytes());
            let recipient = CellLedger::RECIPIENTS_ABS_OFFSET as usize + balance_slot as usize * 32;
            expected.data[recipient..recipient + 32].fill(0xb2);
            expected.data[CellLedger::FLAGS_ABS_OFFSET as usize + flag_slot as usize] = 0xe1;
            let encoded = balance_slot.to_le_bytes();
            let result = HopperSvm::new().process_instruction(
                program,
                &[encoded[0], encoded[1], flag_slot],
                &[ledger],
                selected_handler,
            );
            assert_eq!(result.program_result, Ok(()));
            assert_eq!(result.resulting_accounts, vec![expected]);
        }
    }
}

fn wide_handler<'a>(
    program: &'a Address,
    accounts: &'a [AccountView<'a>],
    data: &'a [u8],
) -> ProgramResult {
    let slot = u32::from_le_bytes(data[..4].try_into().unwrap());
    let mut raw = Context::new(program, accounts, data);
    let mut ctx = WideSelection::bind_with_args(&mut raw, slot)?;
    if data[4] == 0 {
        let _ = ctx.ledger_balances_cell_ref()?;
    } else {
        ctx.ledger_balances_cell_mut()?.checked_add_assign(1)?;
    }
    Ok(())
}

#[test]
fn out_of_range_reads_and_writes_refuse_without_truncating_the_selector() {
    for slot in [4u32, 255, 65_535, 65_536, u32::MAX] {
        for mode in [0, 1] {
            let (program, ledger) = fixture();
            let mut data = slot.to_le_bytes().to_vec();
            data.push(mode);
            let result = HopperSvm::new().process_instruction(
                program,
                &data,
                core::slice::from_ref(&ledger),
                wide_handler,
            );
            assert_eq!(
                result.program_result,
                Err(ProgramError::InvalidInstructionData)
            );
            assert_eq!(result.resulting_accounts, vec![ledger]);
        }
    }
}

#[test]
fn attribute_context_projects_the_last_valid_u32_selected_cell() {
    let (program, ledger) = fixture();
    let mut expected = ledger.clone();
    let offset = CellLedger::BALANCES_ABS_OFFSET as usize + 3 * 8;
    expected.data[offset..offset + 8].copy_from_slice(&41u64.to_le_bytes());
    let result =
        HopperSvm::new().process_instruction(program, &[3, 0, 0, 0, 1], &[ledger], wide_handler);
    assert_eq!(result.program_result, Ok(()));
    assert_eq!(result.resulting_accounts, vec![expected]);
}
