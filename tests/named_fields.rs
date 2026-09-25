//! Named inputs remain separate from the wire ABI and from account authority.
#![cfg(feature = "proc-macros")]
use hopper::prelude::*;

#[hopper::state(disc = 71, version = 2)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Named {
    pub authority: Address,
    pub amount: WireU64,
    pub signed: WireI32,
    pub enabled: WireBool,
    pub tag: [u8; 3],
}

#[hopper::state(disc = 72, compact)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CompactNamed {
    pub amount: WireU64,
    pub enabled: WireBool,
}

fn fields() -> NamedFields {
    NamedFields {
        tag: [3, 2, 1], // Deliberately authored in a different order.
        enabled: true,
        signed: -45,
        amount: u64::MAX - 1,
        authority: Address::new_from_array([8; 32]),
    }
}

#[test]
fn named_and_positional_inputs_have_identical_wire_bytes() {
    let positional = Named::new(
        Address::new_from_array([8; 32]),
        u64::MAX - 1,
        -45,
        true,
        [3, 2, 1],
    );
    let named = Named::from_fields(fields());
    let mut bytes = vec![0; Named::LEN];
    Named::write_init_header(&mut bytes).unwrap();
    let original_header = bytes[..16].to_vec();
    Named::overlay_mut(&mut bytes[16..])
        .unwrap()
        .set_fields(fields())
        .unwrap();
    let actual = Named::overlay(&bytes[16..]).unwrap();
    for state in [&positional, &named, actual] {
        assert_eq!(state.authority, positional.authority);
        assert_eq!(state.amount.get(), positional.amount.get());
        assert_eq!(state.signed.get(), -45);
        assert!(state.enabled.get());
        assert_eq!(state.tag, [3, 2, 1]);
    }
    let mut expected = original_header;
    expected.extend_from_slice(&[8; 32]);
    expected.extend_from_slice(&(u64::MAX - 1).to_le_bytes());
    expected.extend_from_slice(&(-45i32).to_le_bytes());
    expected.extend_from_slice(&[1, 3, 2, 1]);
    assert_eq!(bytes, expected);
}

#[test]
fn compact_named_inputs_keep_the_compact_wire_contract() {
    const VALUE: CompactNamed = CompactNamed::from_fields(CompactNamedFields {
        amount: 123,
        enabled: false,
    });
    let mut value = VALUE;
    CompactNamedFields {
        amount: 456,
        enabled: true,
    }
    .write(&mut value)
    .unwrap();
    assert_eq!(value.amount.get(), 456);
    assert!(value.enabled.get());
    assert_eq!(CompactNamed::COMPACT_LEN, 10);
}

// This integration test is a separate consumer crate. Custom inputs do not
// require modifying the derive or implementing another trait on the layout.
struct CheckedDeposit(u64);
impl AccountFields for CheckedDeposit {
    type Layout = Named;

    fn write(self, state: &mut Named) -> ProgramResult {
        let next = state
            .amount
            .get()
            .checked_add(self.0)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        state.amount.set(next);
        Ok(())
    }
}

#[test]
fn consumer_defined_inputs_can_validate_before_writing() {
    let mut state = Named::from_fields(fields());
    assert_eq!(
        CheckedDeposit(2).write(&mut state),
        Err(ProgramError::ArithmeticOverflow)
    );
    assert_eq!(state.amount.get(), u64::MAX - 1);
    CheckedDeposit(1).write(&mut state).unwrap();
    assert_eq!(state.amount.get(), u64::MAX);
}

#[derive(Accounts)]
pub struct CreateNamed<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(init, payer = payer, space = Named::INIT_SPACE)]
    pub state: InitAccount<'info, Named>,
    pub system_program: Program<'info, System>,
}

struct RejectAfterWrite;
impl AccountFields for RejectAfterWrite {
    type Layout = Named;
    fn write(self, layout: &mut Named) -> ProgramResult {
        layout.amount.set(999);
        Err(ProgramError::Custom(7123))
    }
}

#[program]
mod initializer_program {
    use super::*;

    #[instruction(0)]
    pub fn create(ctx: Ctx<CreateNamed>) -> ProgramResult {
        ctx.init_state_with(fields())
    }

    #[instruction(1)]
    pub fn reject(ctx: Ctx<CreateNamed>) -> ProgramResult {
        ctx.init_state_with(RejectAfterWrite)
    }
}

#[test]
fn composed_initialization_propagates_failure_without_local_undo() {
    use hopper_svm::{AccountFixture, HopperSvm};
    let program = Address::new_from_array([7; 32]);
    let system = Address::new_from_array([0; 32]);
    let accounts = [
        AccountFixture::new(Address::new_from_array([1; 32]), system, 1_000_000_000, 0)
            .signer()
            .writable(),
        AccountFixture::new(Address::new_from_array([2; 32]), system, 0, 0)
            .signer()
            .writable(),
        AccountFixture::new(system, system, 1, 0).executable(),
    ];
    let svm = HopperSvm::new();
    let created = svm.process_instruction(
        program,
        &[0],
        &accounts,
        __hopper_process_instruction_initializer_program,
    );
    assert_eq!(created.program_result, Ok(()));
    assert_eq!(created.resulting_accounts[1].owner, program);
    assert_eq!(
        Named::overlay(&created.resulting_accounts[1].data[16..])
            .unwrap()
            .amount
            .get(),
        u64::MAX - 1
    );
    let refused = svm.process_instruction(
        program,
        &[1],
        &accounts,
        __hopper_process_instruction_initializer_program,
    );
    assert_eq!(refused.program_result, Err(ProgramError::Custom(7123)));
    // HopperSvm is a direct host call, not a transactional SVM. This locks in
    // the documented distinction: the helper propagates failure, but creation
    // and the initializer's earlier writes are not locally undone. Actual SVM
    // rollback is checked separately against the compiled vault ELF.
    assert_eq!(refused.resulting_accounts[1].owner, program);
    assert_eq!(
        Named::overlay(&refused.resulting_accounts[1].data[16..])
            .unwrap()
            .amount
            .get(),
        999
    );
    assert!(refused.resulting_accounts[0].lamports < accounts[0].lamports);
}
