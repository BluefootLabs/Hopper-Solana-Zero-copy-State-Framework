//! The durable-nonce reader: which bytes each accessor returns, and which
//! accounts it refuses to read.

use hopper_native::system::{
    NonceState, NONCE_ACCOUNT_LEN, NONCE_STATE_INITIALIZED, NONCE_VERSION_CURRENT,
};
use hopper_native::ProgramError;

/// An initialized nonce account where byte `i` of the body holds `i`.
fn nonce_account() -> [u8; NONCE_ACCOUNT_LEN] {
    let mut data = [0u8; NONCE_ACCOUNT_LEN];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = i as u8;
    }
    data[0..4].copy_from_slice(&NONCE_VERSION_CURRENT.to_le_bytes());
    data[4..8].copy_from_slice(&NONCE_STATE_INITIALIZED.to_le_bytes());
    data
}

#[test]
fn each_field_is_read_from_its_own_bytes() {
    let data = nonce_account();
    let nonce = NonceState::from_account_data(&data).unwrap();

    let authority = nonce.authority().as_array();
    assert_eq!((authority[0], authority[31]), (8, 39));
    let stored = nonce.durable_nonce();
    assert_eq!((stored[0], stored[31]), (40, 71));
    assert!(core::ptr::eq(stored.as_ptr(), data[40..].as_ptr()));
    assert_eq!(
        nonce.lamports_per_signature(),
        u64::from_le_bytes([72, 73, 74, 75, 76, 77, 78, 79])
    );
}

#[test]
fn a_longer_account_reads_the_same_fields() {
    let mut data = [0xEEu8; NONCE_ACCOUNT_LEN + 40];
    data[..NONCE_ACCOUNT_LEN].copy_from_slice(&nonce_account());
    let nonce = NonceState::from_account_data(&data).unwrap();
    assert_eq!(nonce.durable_nonce()[31], 71);
}

#[test]
fn what_is_not_an_initialized_nonce_is_refused() {
    let data = nonce_account();
    for len in [0, 8, 40, 72, NONCE_ACCOUNT_LEN - 1] {
        assert_eq!(
            NonceState::from_account_data(&data[..len]).unwrap_err(),
            ProgramError::AccountDataTooSmall,
            "{len} bytes"
        );
    }
    // Legacy version, uninitialized state, and a tag with a high byte set.
    for (version, state) in [(0u32, 1u32), (1, 0), (2, 1), (1, 2), (1, 0x0100_0001)] {
        let mut data = nonce_account();
        data[0..4].copy_from_slice(&version.to_le_bytes());
        data[4..8].copy_from_slice(&state.to_le_bytes());
        assert_eq!(
            NonceState::from_account_data(&data).unwrap_err(),
            ProgramError::InvalidAccountData,
            "version {version}, state {state}"
        );
    }
}
