//! The load tiers that overlay a layout without the full header check:
//! `load_unverified` for tooling, and the deprecated `load_unchecked`.
//! Both must hand back the account's own bytes, and `load_unverified` must
//! say truthfully whether the header matched.

use hopper::prelude::*;

hopper::hopper_layout! {
    pub struct Ledger, disc = 81, version = 2 {
        owner:   [u8; 32] = 32,
        balance: WireU64  = 8,
        bump:    u8       = 1,
    }
}

hopper::hopper_interface! {
    /// The same fields, declared by a program that reads the ledger.
    pub struct LedgerView as Ledger, disc = 81, version = 2 {
        owner:   [u8; 32] = 32,
        balance: WireU64  = 8,
        bump:    u8       = 1,
    }
}

/// A reader that names its view like the layout, the form every earlier
/// release accepted.
mod reader {
    use hopper::prelude::*;

    hopper::hopper_interface! {
        pub struct Ledger, disc = 81, version = 2 {
            owner:   [u8; 32] = 32,
            balance: WireU64  = 8,
            bump:    u8       = 1,
        }
    }
}

fn ledger_bytes() -> std::vec::Vec<u8> {
    let mut data = vec![0u8; Ledger::LEN];
    Ledger::write_init_header(&mut data).unwrap();
    data[16..48].copy_from_slice(&[7; 32]);
    data[48..56].copy_from_slice(&1_234u64.to_le_bytes());
    data[56] = 254;
    data
}

#[test]
fn the_layout_is_header_then_fields() {
    assert_eq!(Ledger::LEN, 16 + 32 + 8 + 1);
    assert_eq!(LedgerView::LEN, Ledger::LEN);
    assert_eq!(core::mem::align_of::<Ledger>(), 1);
    // The view is fingerprinted as the layout it names, in either form.
    assert_eq!(LedgerView::ORIGIN, "Ledger");
    assert_eq!(reader::Ledger::LAYOUT_ID, Ledger::LAYOUT_ID);
    assert_eq!(reader::Ledger::ORIGIN, "Ledger");
    assert_eq!(Ledger::LAYOUT_ID, LedgerView::LAYOUT_ID);
    assert_ne!(Ledger::LAYOUT_ID, Renamed::LAYOUT_ID);
    assert_eq!(Renamed::ORIGIN, "Renamed");
}

hopper::hopper_interface! {
    /// The same fields under another layout name: a different layout.
    pub struct Renamed, disc = 81, version = 2 {
        owner:   [u8; 32] = 32,
        balance: WireU64  = 8,
        bump:    u8       = 1,
    }
}

#[test]
fn load_unverified_overlays_in_place_and_reports_the_header() {
    let data = ledger_bytes();
    let (ledger, valid) = Ledger::load_unverified(&data).unwrap();
    assert!(valid);
    assert!(core::ptr::eq(
        ledger as *const Ledger as *const u8,
        data.as_ptr()
    ));
    assert_eq!(ledger.owner, [7; 32]);
    assert_eq!(ledger.balance.get(), 1_234);
    assert_eq!(ledger.bump, 254);

    let (view, valid) = LedgerView::load_unverified(&data).unwrap();
    assert!(valid);
    assert_eq!(view.balance.get(), 1_234);
    assert_eq!(view.bump, 254);
}

#[test]
fn load_unverified_still_reads_when_the_header_is_wrong_and_says_so() {
    // An older version than the layout declares.
    let mut old = ledger_bytes();
    old[1] = 1;
    assert!(!Ledger::load_unverified(&old).unwrap().1);
    assert!(!LedgerView::load_unverified(&old).unwrap().1);
    // A view of another layout reads the bytes and reports the mismatch.
    let data = ledger_bytes();
    let (renamed, valid) = Renamed::load_unverified(&data).unwrap();
    assert!(!valid);
    assert_eq!(renamed.balance.get(), 1_234);

    // The discriminator, and the first, a middle, and the last byte of the
    // layout id.
    for at in [0usize, 4, 7, 11] {
        let mut data = ledger_bytes();
        data[at] ^= 0x40;
        let (ledger, valid) = Ledger::load_unverified(&data).unwrap();
        assert!(!valid, "header byte {at}");
        assert_eq!(ledger.balance.get(), 1_234);
        let (view, valid) = LedgerView::load_unverified(&data).unwrap();
        assert!(!valid, "header byte {at}");
        assert_eq!(view.balance.get(), 1_234);
    }
}

#[test]
fn load_unverified_refuses_a_buffer_shorter_than_the_layout() {
    let data = ledger_bytes();
    for len in [0, 15, 16, Ledger::LEN - 1] {
        assert!(Ledger::load_unverified(&data[..len]).is_none(), "{len}");
        assert!(LedgerView::load_unverified(&data[..len]).is_none(), "{len}");
    }
    // Longer is fine: the overlay covers the first `LEN` bytes.
    let mut longer = data.clone();
    longer.extend_from_slice(&[0xEE; 9]);
    let (ledger, valid) = Ledger::load_unverified(&longer).unwrap();
    assert!(valid);
    assert_eq!(ledger.bump, 254);
}

#[test]
#[allow(deprecated)]
fn load_unchecked_is_the_same_overlay_without_any_check() {
    let data = ledger_bytes();
    // SAFETY: `data` holds `Ledger::LEN` bytes, and the layout has
    // alignment 1 and accepts every bit pattern.
    let ledger = unsafe { Ledger::load_unchecked(&data) };
    assert!(core::ptr::eq(
        ledger as *const Ledger as *const u8,
        data.as_ptr()
    ));
    assert_eq!(ledger.balance.get(), 1_234);
    assert_eq!(ledger.bump, 254);
}

const OWNER_PROGRAM: [u8; 32] = [0xA1; 32];

fn read_through_the_view(_: &Address, accounts: &[AccountView], _: &[u8]) -> ProgramResult {
    let owner = Address::new_from_array(OWNER_PROGRAM);
    let view = LedgerView::load_cross_program(&accounts[0], &owner)?;
    assert_eq!(view.get().balance.get(), 1_234);
    assert_eq!(view.get().bump, 254);
    drop(view);
    // A view that names another layout is refused, as is another owner.
    assert!(Renamed::load_cross_program(&accounts[0], &owner).is_err());
    let stranger = Address::new_from_array([0xB2; 32]);
    assert!(LedgerView::load_cross_program(&accounts[0], &stranger).is_err());
    Ok(())
}

#[test]
fn a_renamed_view_loads_the_account_the_layout_wrote() {
    use hopper_svm::{AccountFixture, HopperSvm};
    let owner = Address::new_from_array(OWNER_PROGRAM);
    let reader = Address::new_from_array([0xC3; 32]);
    let account = AccountFixture::with_data(
        Address::new_from_array([1; 32]),
        owner,
        1_000_000,
        ledger_bytes(),
    );
    let result =
        HopperSvm::new().process_instruction(reader, &[], &[account], read_through_the_view);
    assert_eq!(result.program_result, Ok(()));
}
