use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::ProgramResult;

pub type BackendAccountView = hopper_native::AccountView;
pub type BackendAddress = hopper_native::Address;
pub type BackendProgramResult = hopper_native::ProgramResult;
pub type BackendRef<'a, T> = hopper_native::borrow::Ref<'a, T>;
pub type BackendRefMut<'a, T> = hopper_native::borrow::RefMut<'a, T>;

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
    Address::from(view.read_owner())
}

#[inline(always)]
pub fn as_backend_address(address: &Address) -> &BackendAddress {
    unsafe { &*(address as *const Address as *const BackendAddress) }
}

#[inline(always)]
pub fn owned_by(view: &BackendAccountView, program: &Address) -> bool {
    view.owned_by(as_backend_address(program))
}

#[inline(always)]
pub fn disc(view: &BackendAccountView) -> u8 {
    view.disc()
}

#[inline(always)]
pub fn version(view: &BackendAccountView) -> u8 {
    view.version()
}

#[inline(always)]
pub fn layout_id(view: &BackendAccountView) -> Option<&[u8; 8]> {
    view.layout_id()
}

#[inline(always)]
pub unsafe fn assign(view: &BackendAccountView, new_owner: &Address) {
    unsafe { view.assign(as_backend_address(new_owner)); }
}

#[inline(always)]
pub fn close(view: &BackendAccountView) -> ProgramResult {
    view.close().map_err(ProgramError::from)
}

#[inline(always)]
pub fn zero_data(view: &BackendAccountView) -> ProgramResult {
    let mut data = view.try_borrow_mut().map_err(ProgramError::from)?;
    let mut i = 0;
    while i < data.len() {
        data[i] = 0;
        i += 1;
    }
    Ok(())
}

#[cfg(target_os = "solana")]
#[inline(always)]
pub fn find_program_address(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
    let (address, bump) = hopper_native::pda::find_program_address(seeds, as_backend_address(program_id));
    (Address::from(address), bump)
}

#[inline(always)]
pub fn create_program_address(seeds: &[&[u8]], program_id: &Address) -> Result<Address, ProgramError> {
    #[cfg(target_os = "solana")]
    {
        hopper_native::pda::create_program_address(seeds, as_backend_address(program_id))
            .map(Address::from)
            .map_err(|_| ProgramError::InvalidSeeds)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id);
        Err(ProgramError::InvalidSeeds)
    }
}

#[inline(always)]
pub unsafe fn process_entrypoint<const MAX: usize>(
    input: *mut u8,
    process_instruction: fn(&BackendAddress, &[BackendAccountView], &[u8]) -> BackendProgramResult,
) -> u64 {
    unsafe { hopper_native::entrypoint::process_entrypoint::<MAX>(input, process_instruction) }
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
pub fn set_return_data(data: &[u8]) {
    #[cfg(target_os = "solana")]
    unsafe {
        hopper_native::syscalls::sol_set_return_data(data.as_ptr(), data.len() as u64);
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
