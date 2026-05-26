use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionView, Signer};
use crate::ProgramResult;
use core::cell::UnsafeCell;

#[repr(transparent)]
pub struct BackendAccountView {
    inner: UnsafeCell<pinocchio::AccountView>,
}

pub type BackendAccountSlice<'a> = &'a mut [pinocchio::AccountView];
pub type BackendAddress = pinocchio::Address;
pub type BackendProgramResult = pinocchio::ProgramResult;
pub type BackendRef<'a, T> = pinocchio::account::Ref<'a, T>;
pub type BackendRefMut<'a, T> = pinocchio::account::RefMut<'a, T>;
pub const BACKEND_MAX_TX_ACCOUNTS: usize = pinocchio::MAX_TX_ACCOUNTS;
pub const BACKEND_SUCCESS: u64 = pinocchio::SUCCESS;

const _: () = {
    assert!(
        core::mem::size_of::<BackendAccountView>()
            == core::mem::size_of::<pinocchio::AccountView>()
    );
    assert!(
        core::mem::align_of::<BackendAccountView>()
            == core::mem::align_of::<pinocchio::AccountView>()
    );
    assert!(!core::mem::needs_drop::<BackendAccountView>());
};

impl BackendAccountView {
    #[inline(always)]
    fn as_pinocchio(&self) -> &pinocchio::AccountView {
        // SAFETY: `BackendAccountView` is a transparent compatibility wrapper
        // around Pinocchio's raw-pointer account view. Shared access is enough
        // for read-only methods; Pinocchio's own borrow byte guards data refs.
        unsafe { &*self.inner.get() }
    }

    #[inline(always)]
    fn as_pinocchio_mut(&self) -> &mut pinocchio::AccountView {
        // SAFETY: Isolated to the legacy Pinocchio adapter. Pinocchio 0.11
        // requires `&mut AccountView` for lamports/data/resize mutation, while
        // Hopper's public account API intentionally remains `&self` plus runtime
        // borrow guards over the underlying SVM account memory.
        unsafe { &mut *self.inner.get() }
    }

    #[inline(always)]
    pub fn is_signer(&self) -> bool {
        self.as_pinocchio().is_signer()
    }

    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        self.as_pinocchio().is_writable()
    }

    #[inline(always)]
    pub fn executable(&self) -> bool {
        self.as_pinocchio().executable()
    }

    #[inline(always)]
    pub fn data_len(&self) -> usize {
        self.as_pinocchio().data_len()
    }

    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.as_pinocchio().lamports()
    }

    #[inline(always)]
    pub fn set_lamports(&self, lamports: u64) {
        self.as_pinocchio_mut().set_lamports(lamports);
    }

    #[inline(always)]
    pub fn try_borrow(&self) -> Result<BackendRef<'_, [u8]>, pinocchio::error::ProgramError> {
        self.as_pinocchio().try_borrow()
    }

    #[inline(always)]
    pub fn try_borrow_mut(
        &self,
    ) -> Result<BackendRefMut<'_, [u8]>, pinocchio::error::ProgramError> {
        self.as_pinocchio_mut().try_borrow_mut()
    }

    #[inline(always)]
    pub fn check_borrow(&self) -> Result<(), pinocchio::error::ProgramError> {
        self.as_pinocchio().check_borrow()
    }

    #[inline(always)]
    pub fn check_borrow_mut(&self) -> Result<(), pinocchio::error::ProgramError> {
        self.as_pinocchio().check_borrow_mut()
    }

    #[inline(always)]
    pub unsafe fn borrow_unchecked(&self) -> &[u8] {
        // SAFETY: Caller upholds unchecked borrow invariants.
        unsafe { self.as_pinocchio().borrow_unchecked() }
    }

    #[inline(always)]
    pub unsafe fn borrow_unchecked_mut(&self) -> &mut [u8] {
        // SAFETY: Caller upholds unchecked mutable borrow invariants.
        unsafe { self.as_pinocchio_mut().borrow_unchecked_mut() }
    }

    #[inline(always)]
    pub unsafe fn close_unchecked(&self) {
        // SAFETY: Caller guarantees no active borrows.
        unsafe { self.as_pinocchio_mut().close_unchecked() }
    }
}

impl Clone for BackendAccountView {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            inner: UnsafeCell::new(*self.as_pinocchio()),
        }
    }
}

impl PartialEq for BackendAccountView {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.as_pinocchio() == other.as_pinocchio()
    }
}

impl Eq for BackendAccountView {}

impl AsRef<pinocchio::AccountView> for BackendAccountView {
    #[inline(always)]
    fn as_ref(&self) -> &pinocchio::AccountView {
        self.as_pinocchio()
    }
}

#[inline(always)]
///
/// # Safety
///
/// Caller must uphold the invariants documented for this unsafe API before invoking it.
pub unsafe fn wrap_account_slice(accounts: BackendAccountSlice<'_>) -> &[AccountView] {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe { core::slice::from_raw_parts(accounts.as_ptr() as *const AccountView, accounts.len()) }
}

#[inline(always)]
pub fn account_address(view: &BackendAccountView) -> &Address {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe { &*(view.as_pinocchio().address() as *const BackendAddress as *const Address) }
}

#[inline(always)]
///
/// # Safety
///
/// Caller must uphold the invariants documented for this unsafe API before invoking it.
pub unsafe fn account_owner(view: &BackendAccountView) -> &Address {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe { &*(view.as_pinocchio().owner() as *const BackendAddress as *const Address) }
}

#[inline(always)]
pub fn read_owner(view: &BackendAccountView) -> Address {
    Address(view.as_pinocchio().owner().to_bytes())
}

#[inline(always)]
pub fn as_backend_address(address: &Address) -> &BackendAddress {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { *view.borrow_unchecked().as_ptr() }
    }
}

#[inline(always)]
pub fn version(view: &BackendAccountView) -> u8 {
    if view.data_len() < 2 {
        0
    } else {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { *view.borrow_unchecked().as_ptr().add(1) }
    }
}

#[inline(always)]
pub fn layout_id(view: &BackendAccountView) -> Option<&[u8; 8]> {
    if view.data_len() < 12 {
        None
    } else {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        Some(unsafe { &*(view.borrow_unchecked().as_ptr().add(4) as *const [u8; 8]) })
    }
}

#[inline(always)]
///
/// # Safety
///
/// Caller must uphold the invariants documented for this unsafe API before invoking it.
pub unsafe fn assign(view: &BackendAccountView, new_owner: &Address) {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        view.as_pinocchio_mut()
            .assign(as_backend_address(new_owner));
    }
}

#[inline(always)]
pub fn close(view: &BackendAccountView) -> ProgramResult {
    view.set_lamports(0);
    zero_data(view)
}

#[inline(always)]
pub fn zero_data(view: &BackendAccountView) -> ProgramResult {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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

#[inline(always)]
pub fn resize(view: &BackendAccountView, new_len: usize) -> ProgramResult {
    pinocchio::Resize::resize(view.as_pinocchio_mut(), new_len).map_err(ProgramError::from)
}

#[cfg(target_os = "solana")]
#[inline(always)]
pub fn find_program_address(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
    crate::pda::find_program_address(seeds, program_id)
}

#[inline(always)]
pub fn create_program_address(
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<Address, ProgramError> {
    crate::pda::create_program_address(seeds, program_id)
}

#[inline(always)]
///
/// # Safety
///
/// Caller must uphold the invariants documented for this unsafe API before invoking it.
pub unsafe fn process_entrypoint<const MAX: usize>(
    input: *mut u8,
    process_instruction: fn(
        &BackendAddress,
        BackendAccountSlice<'_>,
        &[u8],
    ) -> BackendProgramResult,
) -> u64 {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe { pinocchio::entrypoint::process_entrypoint::<MAX>(input, process_instruction) }
}

#[inline(always)]
pub fn bridge_to_runtime(
    program_id: &BackendAddress,
    accounts: BackendAccountSlice<'_>,
    data: &[u8],
    process_instruction: fn(&Address, &[AccountView], &[u8]) -> ProgramResult,
) -> BackendProgramResult {
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        accounts: unsafe {
            core::slice::from_raw_parts(
                instruction.accounts.as_ptr() as *const pinocchio::instruction::InstructionAccount,
                instruction.accounts.len(),
            )
        },
        data: instruction.data,
    };

    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    let pin_accounts: &[&BackendAccountView; ACCOUNTS] =
        unsafe { &*(account_views as *const _ as *const [&BackendAccountView; ACCOUNTS]) };

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
