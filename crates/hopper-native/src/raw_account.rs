//! Raw account header for the Solana loader input buffer.
//!
//! `RuntimeAccount` is the substrate-level truth Hopper Native uses when it
//! reads the loader's account array. Duplicate-account tracking lives in
//! [`crate::raw_input`]; this type models only the canonical backing account
//! record that duplicates point at.

use crate::address::Address;

/// Raw C-layout struct matching the Solana BPF account input format.
///
/// Each non-duplicate account in the entrypoint input buffer begins with this
/// header, immediately followed by `data_len` bytes of account data.
///
/// The first byte occupies the loader's duplicate marker slot. Hopper Native
/// reuses that byte as `borrow_state`, which is valid because canonical
/// accounts enter the program with the `0xFF` marker and Hopper uses the same
/// value to mean `NOT_BORROWED`.
#[repr(C)]
#[cfg_attr(feature = "copy", derive(Copy))]
#[derive(Clone, Default)]
pub struct RuntimeAccount {
    /// Borrow tracking state (repurposed from the loader duplicate marker).
    ///
    /// Duplicate-account relationships are tracked in `raw_input`; once a
    /// canonical account is identified, this byte becomes Hopper's borrow
    /// state for that canonical record.
    pub borrow_state: u8,
    /// 1 if transaction signer, 0 otherwise.
    pub is_signer: u8,
    /// 1 if writable, 0 otherwise.
    pub is_writable: u8,
    /// 1 if executable, 0 otherwise.
    pub executable: u8,
    /// Delta between original and current data length (realloc tracking).
    pub resize_delta: i32,
    /// Account public key (32 bytes).
    pub address: Address,
    /// Owning program (32 bytes).
    pub owner: Address,
    /// Lamport balance.
    pub lamports: u64,
    /// Length of account data following this struct.
    pub data_len: u64,
    // Account data bytes follow immediately in memory.
}

impl RuntimeAccount {
    /// Size of the raw loader header before account data begins.
    pub const SIZE: usize = core::mem::size_of::<Self>();
}

const _: () = assert!(core::mem::size_of::<RuntimeAccount>() == 88);
