use alloc::vec::Vec;
use core::cell::{Ref, RefMut};

use crate::account::AccountView;
use crate::address::{address_eq, Address};
use crate::error::ProgramError;
use crate::instruction::{InstructionView, Signer};
use crate::ProgramResult;

pub type BackendAddress = ::solana_program::pubkey::Pubkey;
pub type BackendProgramResult = ::solana_program::entrypoint::ProgramResult;
pub type BackendRef<'a, T> = Ref<'a, T>;
pub type BackendRefMut<'a, T> = RefMut<'a, T>;
pub const BACKEND_MAX_TX_ACCOUNTS: usize = 254;
pub const BACKEND_SUCCESS: u64 = ::solana_program::entrypoint::SUCCESS;

#[repr(transparent)]
#[derive(Clone)]
pub struct BackendAccountView {
    inner: ::solana_program::account_info::AccountInfo<'static>,
}

impl PartialEq for BackendAccountView {
    fn eq(&self, other: &Self) -> bool {
        self.inner.key == other.inner.key
            && self.inner.owner == other.inner.owner
            && self.inner.is_signer == other.inner.is_signer
            && self.inner.is_writable == other.inner.is_writable
            && self.inner.executable == other.inner.executable
    }
}

impl Eq for BackendAccountView {}

impl BackendAccountView {
    #[inline(always)]
    pub fn new(inner: ::solana_program::account_info::AccountInfo<'static>) -> Self {
        Self { inner }
    }

    #[inline(always)]
    pub fn is_signer(&self) -> bool {
        self.inner.is_signer
    }

    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        self.inner.is_writable
    }

    #[inline(always)]
    pub fn executable(&self) -> bool {
        self.inner.executable
    }

    #[inline(always)]
    pub fn data_len(&self) -> usize {
        self.inner.data_len()
    }

    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.inner.lamports()
    }

    #[inline(always)]
    pub fn set_lamports(&self, lamports: u64) {
        let mut current = self.inner.try_borrow_mut_lamports().expect("lamports borrow conflict");
        **current = lamports;
    }

    #[inline(always)]
    pub fn try_borrow(
        &self,
    ) -> Result<BackendRef<'_, [u8]>, ::solana_program::program_error::ProgramError> {
        self.inner
            .try_borrow_data()
            .map(|data| Ref::map(data, |slice| &**slice))
    }

    #[inline(always)]
    pub fn try_borrow_mut(
        &self,
    ) -> Result<BackendRefMut<'_, [u8]>, ::solana_program::program_error::ProgramError> {
        self.inner
            .try_borrow_mut_data()
            .map(|data| RefMut::map(data, |slice| &mut **slice))
    }

    #[inline(always)]
    pub fn resize(
        &self,
        new_len: usize,
    ) -> Result<(), ::solana_program::program_error::ProgramError> {
        #[allow(deprecated)]
        self.inner.realloc(new_len, false)
    }

    #[inline(always)]
    pub fn check_borrow(&self) -> Result<(), ::solana_program::program_error::ProgramError> {
        self.inner.try_borrow_data().map(|_| ())
    }

    #[inline(always)]
    pub fn check_borrow_mut(&self) -> Result<(), ::solana_program::program_error::ProgramError> {
        self.inner.try_borrow_mut_data().map(|_| ())
    }

    #[inline(always)]
    pub unsafe fn borrow_unchecked(&self) -> &[u8] {
        let data_ptr = self.inner.data.as_ptr();
        unsafe { &**data_ptr }
    }

    #[inline(always)]
    pub unsafe fn borrow_unchecked_mut(&self) -> &mut [u8] {
        let data_ptr = self.inner.data.as_ptr();
        unsafe { &mut **data_ptr }
    }

    #[inline(always)]
    pub unsafe fn close_unchecked(&self) {
        self.set_lamports(0);
        let data = unsafe { self.borrow_unchecked_mut() };
        let mut i = 0;
        while i < data.len() {
            data[i] = 0;
            i += 1;
        }
    }

    #[inline(always)]
    pub(crate) fn as_account_info(&self) -> &::solana_program::account_info::AccountInfo<'static> {
        &self.inner
    }
}

#[inline(always)]
pub unsafe fn wrap_account_slice(accounts: &[BackendAccountView]) -> &[AccountView] {
    unsafe { core::slice::from_raw_parts(accounts.as_ptr() as *const AccountView, accounts.len()) }
}

#[inline(always)]
unsafe fn wrap_deserialized_accounts<'a>(
    accounts: &'a [::solana_program::account_info::AccountInfo<'a>],
) -> &'a [BackendAccountView] {
    unsafe {
        core::slice::from_raw_parts(accounts.as_ptr() as *const BackendAccountView, accounts.len())
    }
}

#[inline(always)]
pub fn account_address(view: &BackendAccountView) -> &Address {
    unsafe { &*(view.inner.key as *const BackendAddress as *const Address) }
}

#[inline(always)]
pub unsafe fn account_owner(view: &BackendAccountView) -> &Address {
    unsafe { &*(view.inner.owner as *const BackendAddress as *const Address) }
}

#[inline(always)]
pub fn read_owner(view: &BackendAccountView) -> Address {
    Address::from(view.inner.owner.to_bytes())
}

#[inline(always)]
pub fn as_backend_address(address: &Address) -> BackendAddress {
    BackendAddress::new_from_array(address.to_bytes())
}

#[inline(always)]
pub fn owned_by(view: &BackendAccountView, program: &Address) -> bool {
    address_eq(&read_owner(view), program)
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
    let owner = as_backend_address(new_owner);
    view.inner.assign(&owner);
}

#[inline(always)]
pub fn close(view: &BackendAccountView) -> ProgramResult {
    view.set_lamports(0);
    zero_data(view)
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
    let program_id = as_backend_address(program_id);
    let (address, bump) = BackendAddress::find_program_address(seeds, &program_id);
    (Address::from(address), bump)
}

#[inline(always)]
pub fn create_program_address(seeds: &[&[u8]], program_id: &Address) -> Result<Address, ProgramError> {
    let program_id = as_backend_address(program_id);
    BackendAddress::create_program_address(seeds, &program_id)
        .map(Address::from)
        .map_err(|_| ProgramError::InvalidSeeds)
}

#[inline(always)]
pub unsafe fn process_entrypoint<const MAX: usize>(
    input: *mut u8,
    process_instruction: fn(&BackendAddress, &[BackendAccountView], &[u8]) -> BackendProgramResult,
) -> u64 {
    let (program_id, accounts, data) = unsafe { ::solana_program::entrypoint::deserialize(input) };

    let count = accounts.len().min(MAX);
    let wrapped = unsafe { wrap_deserialized_accounts(&accounts[..count]) };

    let program_id: &'static BackendAddress = unsafe { core::mem::transmute(program_id) };

    match process_instruction(program_id, wrapped, data) {
        Ok(()) => ::solana_program::entrypoint::SUCCESS,
        Err(error) => error.into(),
    }
}

#[inline(always)]
pub fn bridge_to_runtime(
    program_id: &BackendAddress,
    accounts: &[BackendAccountView],
    data: &[u8],
    process_instruction: fn(&Address, &[AccountView], &[u8]) -> ProgramResult,
) -> BackendProgramResult {
    let hopper_id = Address::from(*program_id);
    let hopper_accounts = unsafe { wrap_account_slice(accounts) };
    match process_instruction(&hopper_id, hopper_accounts, data) {
        Ok(()) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[inline(always)]
fn build_instruction(instruction: &InstructionView) -> ::solana_program::instruction::Instruction {
    let mut accounts = Vec::with_capacity(instruction.accounts.len());
    let mut i = 0;
    while i < instruction.accounts.len() {
        let account = &instruction.accounts[i];
        let pubkey = as_backend_address(account.address);
        let meta = match (account.is_writable, account.is_signer) {
            (true, true) => ::solana_program::instruction::AccountMeta::new(pubkey, true),
            (true, false) => ::solana_program::instruction::AccountMeta::new(pubkey, false),
            (false, true) => ::solana_program::instruction::AccountMeta::new_readonly(pubkey, true),
            (false, false) => ::solana_program::instruction::AccountMeta::new_readonly(pubkey, false),
        };
        accounts.push(meta);
        i += 1;
    }

    ::solana_program::instruction::Instruction {
        program_id: as_backend_address(instruction.program_id),
        accounts,
        data: instruction.data.to_vec(),
    }
}

#[inline(always)]
fn clone_account_infos<const ACCOUNTS: usize>(
    account_views: &[&AccountView; ACCOUNTS],
) -> Vec<::solana_program::account_info::AccountInfo<'static>> {
    let mut infos = Vec::with_capacity(ACCOUNTS);
    let mut i = 0;
    while i < ACCOUNTS {
        infos.push(account_views[i].as_backend().as_account_info().clone());
        i += 1;
    }
    infos
}

#[inline(always)]
fn signer_seed_groups<'a, 'b>(signers: &[Signer<'a, 'b>]) -> Vec<Vec<&'a [u8]>> {
    let mut groups = Vec::with_capacity(signers.len());
    let mut i = 0;
    while i < signers.len() {
        let signer = &signers[i];
        let seeds = unsafe { core::slice::from_raw_parts(signer.seeds, signer.len as usize) };
        let mut group = Vec::with_capacity(seeds.len());

        let mut j = 0;
        while j < seeds.len() {
            group.push(unsafe {
                core::slice::from_raw_parts(seeds[j].seed, seeds[j].len as usize)
            });
            j += 1;
        }

        groups.push(group);
        i += 1;
    }

    groups
}

#[inline(always)]
pub fn invoke_signed<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
    signers_seeds: &[Signer],
) -> ProgramResult {
    let backend_instruction = build_instruction(instruction);
    let backend_accounts = clone_account_infos(account_views);
    let signer_groups = signer_seed_groups(signers_seeds);
    let mut signer_refs = Vec::with_capacity(signer_groups.len());

    let mut i = 0;
    while i < signer_groups.len() {
        signer_refs.push(signer_groups[i].as_slice());
        i += 1;
    }

    ::solana_program::program::invoke_signed(
        &backend_instruction,
        &backend_accounts,
        signer_refs.as_slice(),
    )
    .map_err(ProgramError::from)
}

#[inline(always)]
pub fn set_return_data(data: &[u8]) {
    ::solana_program::program::set_return_data(data)
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

impl From<::solana_program::program_error::ProgramError> for ProgramError {
    #[inline(always)]
    fn from(error: ::solana_program::program_error::ProgramError) -> Self {
        ProgramError::from(u64::from(error))
    }
}

impl From<ProgramError> for ::solana_program::program_error::ProgramError {
    #[inline(always)]
    fn from(error: ProgramError) -> Self {
        ::solana_program::program_error::ProgramError::from(u64::from(error))
    }
}