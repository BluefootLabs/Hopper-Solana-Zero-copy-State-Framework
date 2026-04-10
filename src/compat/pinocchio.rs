use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionView, Signer};
use crate::ProgramResult;

pub type BackendAccountView = pinocchio::AccountView;
pub type BackendAddress = pinocchio::Address;
pub type BackendProgramResult = pinocchio::ProgramResult;
pub type BackendRef<'a, T> = pinocchio::account::Ref<'a, T>;
pub type BackendRefMut<'a, T> = pinocchio::account::RefMut<'a, T>;

#[inline(always)]
pub unsafe fn wrap_account_slice(accounts: &[BackendAccountView]) -> &[AccountView] {
    unsafe { core::slice::from_raw_parts(accounts.as_ptr() as *const AccountView, accounts.len()) }
}

#[inline(always)]
pub fn account_address(view: &BackendAccountView) -> &Address {
    unsafe { &*(view.address() as *const BackendAddress as *const Address) }
}

#[inline(always)]
pub unsafe fn account_owner(view: &BackendAccountView) -> &Address {
    unsafe { &*(view.owner() as *const BackendAddress as *const Address) }
}

#[inline(always)]
pub fn read_owner(view: &BackendAccountView) -> Address {
    Address(unsafe { view.owner() }.to_bytes())
}

#[inline(always)]
pub fn as_backend_address(address: &Address) -> &BackendAddress {
    unsafe { &*(address as *const Address as *const BackendAddress) }
}

#[inline(always)]
pub fn owned_by(view: &BackendAccountView, program: &Address) -> bool {
    crate::address::address_eq(&read_owner(view), program)
}

#[inline(always)]
pub fn disc(view: &BackendAccountView) -> u8 {
    if view.data_len() == 0 {
        0
    } else {
        unsafe { *view.borrow_unchecked().as_ptr() }
    }
}

#[inline(always)]
pub fn version(view: &BackendAccountView) -> u8 {
    if view.data_len() < 2 {
        0
    } else {
        unsafe { *view.borrow_unchecked().as_ptr().add(1) }
    }
}

#[inline(always)]
pub fn layout_id(view: &BackendAccountView) -> Option<&[u8; 8]> {
    if view.data_len() < 12 {
        None
    } else {
        Some(unsafe { &*(view.borrow_unchecked().as_ptr().add(4) as *const [u8; 8]) })
    }
}

#[inline(always)]
pub unsafe fn assign(view: &BackendAccountView, new_owner: &Address) {
    unsafe { view.assign(as_backend_address(new_owner)); }
}

#[inline(always)]
pub fn close(view: &BackendAccountView) -> ProgramResult {
    view.set_lamports(0);
    zero_data(view)
}

#[inline(always)]
pub fn zero_data(view: &BackendAccountView) -> ProgramResult {
    unsafe {
        let data = view.borrow_unchecked_mut();
        let mut i = 0;
        while i < data.len() {
            data[i] = 0;
            i += 1;
        }
    }
    Ok(())
}

#[cfg(target_os = "solana")]
#[inline(always)]
pub fn find_program_address(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
    crate::pda::find_program_address(seeds, program_id)
}

#[inline(always)]
pub fn create_program_address(seeds: &[&[u8]], program_id: &Address) -> Result<Address, ProgramError> {
    crate::pda::create_program_address(seeds, program_id)
}

#[inline(always)]
pub unsafe fn process_entrypoint<const MAX: usize>(
    input: *mut u8,
    process_instruction: fn(&BackendAddress, &[BackendAccountView], &[u8]) -> BackendProgramResult,
) -> u64 {
    unsafe { pinocchio::entrypoint::process_entrypoint::<MAX>(input, process_instruction) }
}

#[inline(always)]
pub fn bridge_to_runtime(
    program_id: &BackendAddress,
    accounts: &[BackendAccountView],
    data: &[u8],
    process_instruction: fn(&Address, &[AccountView], &[u8]) -> ProgramResult,
) -> BackendProgramResult {
    let hopper_id = unsafe { &*(program_id as *const BackendAddress as *const Address) };
    let hopper_accounts = unsafe { wrap_account_slice(accounts) };
    match process_instruction(hopper_id, hopper_accounts, data) {
        Ok(()) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[inline(always)]
pub fn signers_as_backend<'a, 'b>(
    signers: &'b [Signer<'a, 'b>],
) -> &'b [pinocchio::cpi::Signer<'a, 'b>] {
    unsafe {
        core::slice::from_raw_parts(
            signers.as_ptr() as *const pinocchio::cpi::Signer,
            signers.len(),
        )
    }
}

#[inline(always)]
pub fn invoke_signed<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
    signers_seeds: &[Signer],
) -> ProgramResult {
    let pin_instruction = pinocchio::instruction::InstructionView {
        program_id: as_backend_address(instruction.program_id),
        accounts: unsafe {
            core::slice::from_raw_parts(
                instruction.accounts.as_ptr() as *const pinocchio::instruction::InstructionAccount,
                instruction.accounts.len(),
            )
        },
        data: instruction.data,
    };

    let pin_accounts: &[&BackendAccountView; ACCOUNTS] = unsafe {
        &*(account_views as *const _ as *const [&BackendAccountView; ACCOUNTS])
    };

    if signers_seeds.is_empty() {
        pinocchio::cpi::invoke(&pin_instruction, pin_accounts).map_err(ProgramError::from)
    } else {
        pinocchio::cpi::invoke_signed(
            &pin_instruction,
            pin_accounts,
            signers_as_backend(signers_seeds),
        )
        .map_err(ProgramError::from)
    }
}

#[inline(always)]
pub fn set_return_data(data: &[u8]) {
    #[cfg(target_os = "solana")]
    unsafe {
        pinocchio::syscalls::sol_set_return_data(data.as_ptr(), data.len() as u64);
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = data;
    }
}

impl From<BackendAddress> for Address {
    #[inline(always)]
    fn from(address: BackendAddress) -> Self {
        Self(address.to_bytes())
    }
}

impl From<Address> for BackendAddress {
    #[inline(always)]
    fn from(address: Address) -> Self {
        BackendAddress::new_from_array(address.to_bytes())
    }
}

impl From<pinocchio::error::ProgramError> for ProgramError {
    #[inline(always)]
    fn from(error: pinocchio::error::ProgramError) -> Self {
        ProgramError::from(u64::from(error))
    }
}

impl From<ProgramError> for pinocchio::error::ProgramError {
    #[inline(always)]
    fn from(error: ProgramError) -> Self {
        pinocchio::error::ProgramError::from(u64::from(error))
    }
}
