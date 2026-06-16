use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::ProgramResult;

pub type BackendAccountView<'info> = hopper_native::AccountView<'info>;
pub type BackendAccountSlice<'info> = &'info [BackendAccountView<'info>];
pub type BackendAddress = hopper_native::Address;
pub type BackendProgramResult = hopper_native::ProgramResult;
pub type BackendRef<'a, T> = hopper_native::borrow::Ref<'a, T>;
pub type BackendRefMut<'a, T> = hopper_native::borrow::RefMut<'a, T>;
pub const BACKEND_MAX_TX_ACCOUNTS: usize = hopper_native::MAX_TX_ACCOUNTS;
pub const BACKEND_SUCCESS: u64 = hopper_native::SUCCESS;

/// # Safety
///
/// Caller must provide the account slice handed to Hopper by the native Solana
/// entrypoint boundary. `AccountView` is layout-checked as transparent over the
/// native account view before this cast is used.
#[inline(always)]
pub unsafe fn wrap_account_slice<'info>(
    accounts: &'info [BackendAccountView<'info>],
) -> &'info [AccountView<'info>] {
    // SAFETY: AccountView is repr(transparent) over BackendAccountView and
    // compile-time layout assertions in account.rs enforce size/alignment.
    unsafe {
        core::slice::from_raw_parts(
            accounts.as_ptr() as *const AccountView<'info>,
            accounts.len(),
        )
    }
}

#[inline(always)]
pub fn account_address<'a>(view: &'a BackendAccountView<'a>) -> &'a Address {
    // SAFETY: Hopper Address and BackendAddress are both 32-byte transparent
    // address wrappers; returned reference is tied to the backend view.
    unsafe { &*(view.address() as *const BackendAddress as *const Address) }
}

/// # Safety
///
/// The returned owner reference is invalidated if the native account owner is
/// reassigned. Callers that need stable ownership should use `read_owner`.
#[inline(always)]
pub unsafe fn account_owner<'a>(view: &'a BackendAccountView<'a>) -> &'a Address {
    // SAFETY: Same address-layout cast as account_address; caller upholds the
    // owner-reference invalidation contract documented above.
    unsafe { &*(view.owner() as *const BackendAddress as *const Address) }
}

#[inline(always)]
pub fn read_owner(view: &BackendAccountView<'_>) -> Address {
    Address::from(view.read_owner())
}

#[inline(always)]
pub fn as_backend_address(address: &Address) -> &BackendAddress {
    // SAFETY: Address and BackendAddress share the exact 32-byte wire layout.
    unsafe { &*(address as *const Address as *const BackendAddress) }
}

#[inline(always)]
pub fn owned_by(view: &BackendAccountView<'_>, program: &Address) -> bool {
    view.owned_by(as_backend_address(program))
}

#[inline(always)]
pub fn disc(view: &BackendAccountView<'_>) -> u8 {
    view.disc()
}

#[inline(always)]
pub fn version(view: &BackendAccountView<'_>) -> u8 {
    view.version()
}

#[inline(always)]
pub fn layout_id<'a>(view: &'a BackendAccountView<'a>) -> Option<&'a [u8; 8]> {
    view.layout_id()
}

/// # Safety
///
/// Caller must ensure the account is writable and that owner reassignment is
/// authorized by the active instruction.
#[inline(always)]
pub unsafe fn assign(view: &BackendAccountView<'_>, new_owner: &Address) {
    // SAFETY: Caller guarantees owner reassignment is authorized; the address
    // cast preserves the 32-byte owner value exactly.
    unsafe {
        view.assign(as_backend_address(new_owner));
    }
}

#[inline(always)]
pub fn try_set_lamports(view: &BackendAccountView<'_>, lamports: u64) -> ProgramResult {
    view.set_lamports(lamports);
    Ok(())
}

#[inline(always)]
pub fn close(view: &BackendAccountView<'_>) -> ProgramResult {
    view.close().map_err(ProgramError::from)
}

#[inline(always)]
pub fn zero_data(view: &BackendAccountView<'_>) -> ProgramResult {
    let mut data = view.try_borrow_mut().map_err(ProgramError::from)?;
    let mut i = 0;
    while i < data.len() {
        data[i] = 0;
        i += 1;
    }
    Ok(())
}

#[inline(always)]
pub fn resize(view: &BackendAccountView<'_>, new_len: usize) -> ProgramResult {
    view.resize(new_len).map_err(ProgramError::from)
}

#[inline(always)]
pub fn resize_raw(view: &BackendAccountView<'_>, new_len: usize) -> ProgramResult {
    view.resize_raw(new_len).map_err(ProgramError::from)
}

#[cfg(target_os = "solana")]
#[inline(always)]
pub fn find_program_address(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
    let (address, bump) =
        hopper_native::pda::find_program_address(seeds, as_backend_address(program_id));
    (Address::from(address), bump)
}

#[inline(always)]
pub fn create_program_address(
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<Address, ProgramError> {
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

/// # Safety
///
/// Called from the exported Solana entrypoint with the loader-provided input
/// buffer. The pointer must be the raw SVM input buffer for this invocation.
#[inline(always)]
pub unsafe fn process_entrypoint<const MAX: usize>(
    input: *mut u8,
    process_instruction: fn(
        &BackendAddress,
        BackendAccountSlice<'_>,
        &[u8],
    ) -> BackendProgramResult,
) -> u64 {
    // SAFETY: Entrypoint caller provides the SVM input pointer and bridge
    // function matching Hopper Native's expected ABI.
    unsafe { hopper_native::entrypoint::process_entrypoint::<MAX>(input, process_instruction) }
}

#[inline(always)]
pub fn bridge_to_runtime(
    program_id: &BackendAddress,
    accounts: BackendAccountSlice<'_>,
    data: &[u8],
    process_instruction: for<'info> fn(
        &'info Address,
        &'info [AccountView<'info>],
        &'info [u8],
    ) -> ProgramResult,
) -> BackendProgramResult {
    // SAFETY: BackendAddress and Address have identical 32-byte layouts;
    // lifetime is inherited from the entrypoint-provided program id.
    let hopper_id = unsafe { &*(program_id as *const BackendAddress as *const Address) };
    // SAFETY: Backend account slice comes directly from Hopper Native and is
    // repr-compatible with runtime AccountView.
    let hopper_accounts = unsafe { wrap_account_slice(accounts) };
    match process_instruction(hopper_id, hopper_accounts, data) {
        Ok(()) => Ok(()),
        Err(error) => Err(error.into()),
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

#[doc(hidden)]
#[macro_export]
macro_rules! __hopper_native_entrypoint {
    ( $process_instruction:expr, $maximum:expr ) => {
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8) -> u64 {
            #[inline(always)]
            fn __hopper_bridge(
                program_id: &$crate::native_boundary::BackendAddress,
                accounts: $crate::native_boundary::BackendAccountSlice<'_>,
                data: &[u8],
            ) -> $crate::native_boundary::BackendProgramResult {
                $crate::native_boundary::bridge_to_runtime(
                    program_id,
                    accounts,
                    data,
                    $process_instruction,
                )
            }
            // SAFETY: Solana calls this entrypoint with a valid BPF input
            // buffer; process_entrypoint performs the loader-frame parse.
            unsafe {
                $crate::native_boundary::process_entrypoint::<$maximum>(input, __hopper_bridge)
            }
        }
    };
}
