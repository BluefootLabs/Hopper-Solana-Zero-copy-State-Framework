#![cfg(feature = "proc-macros")]
//! DSL and modifier wrappers must agree with both supported Rust overlay shapes.
use hopper::hopper_core::{
    accounts::HopperAccount,
    check::modifier::{self, FromAccount, HopperLayout},
};
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

#[hopper::state(disc = 73)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BodyOnly {
    pub value: WireU64,
}

hopper::hopper_layout! {
    pub struct HeaderIncluded, disc = 74, version = 1 {
        value: WireU64 = 8,
    }
}

trait Value: HopperLayout {
    fn value(&self) -> u64;
    fn set(&mut self, value: u64);
}
impl Value for BodyOnly {
    fn value(&self) -> u64 {
        self.value.get()
    }
    fn set(&mut self, value: u64) {
        self.value.set(value);
    }
}
impl Value for HeaderIncluded {
    fn value(&self) -> u64 {
        self.value.get()
    }
    fn set(&mut self, value: u64) {
        self.value.set(value);
    }
}

fn access<T: Value>(pid: &Address, accounts: &[AccountView], _: &[u8]) -> ProgramResult {
    let account = HopperAccount::<T>::from_account_mut(&accounts[0], pid)?;
    let read = account.read()?;
    assert_eq!(read.get().value(), 42);
    assert!(
        account.write().is_err(),
        "body projection must retain the guard"
    );
    drop(read);
    account.write()?.get_mut().set(43);
    let modifier = modifier::Account::<T>::from_account(&accounts[0], pid)?;
    assert_eq!(modifier.get().value(), 43);
    drop(modifier);
    modifier::AccountMut::<T>::from_account(&accounts[0], pid)?
        .get_mut()
        .set(44);
    #[cfg(feature = "migrate")]
    {
        let migration = hopper::hopper_core::accounts::MigratingAccount::<T, T>::from_account(
            &accounts[0],
            pid,
        )?;
        assert_eq!(migration.old()?.get().value(), 44);
        migration.old_mut()?.get_mut().set(45);
        assert_eq!(migration.into_latest()?.get().value(), 45);
        migration.into_latest()?.get_mut().set(44);
    }
    Ok(())
}

fn exercise<T: Value>() {
    let pid = Address::new_from_array([99; 32]);
    let mut data = vec![0; T::LEN_WITH_HEADER];
    hopper::hopper_core::account::write_header(&mut data, T::DISC, T::VERSION, &T::LAYOUT_ID)
        .unwrap();
    data[16..24].copy_from_slice(&42u64.to_le_bytes());
    let account = AccountFixture::with_data(
        Address::new_from_array([98; 32]),
        pid,
        1_000_000,
        data.clone(),
    )
    .writable();
    let svm = HopperSvm::new();
    let result = svm.process_instruction(pid, &[], core::slice::from_ref(&account), access::<T>);
    assert_eq!(result.program_result, Ok(()));
    data[16..24].copy_from_slice(&44u64.to_le_bytes());
    assert_eq!(
        result.resulting_accounts[0].data, data,
        "writes must preserve the complete header"
    );
    let mut wrong = account;
    wrong.owner = Address::new_from_array([97; 32]);
    let result = svm.process_instruction(pid, &[], core::slice::from_ref(&wrong), access::<T>);
    assert!(result.program_result.is_err());
    assert_eq!(result.resulting_accounts[0].data, wrong.data);
}

#[test]
fn body_only_proc_layout_skips_header_and_keeps_borrow_guard() {
    exercise::<BodyOnly>();
}

#[test]
fn declarative_layout_still_includes_header() {
    exercise::<HeaderIncluded>();
}
