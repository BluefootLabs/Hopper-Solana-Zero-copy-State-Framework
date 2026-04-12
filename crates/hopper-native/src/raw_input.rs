//! Raw loader input parsing for Hopper Native.
//!
//! This is the single source of truth for Solana loader input decoding. It owns
//! duplicate-account resolution, canonical-account lookup, and original-index
//! tracking so higher layers operate on already-resolved account views.

use core::mem::MaybeUninit;

use crate::account_view::AccountView;
use crate::address::Address;
use crate::raw_account::RuntimeAccount;
use crate::MAX_PERMITTED_DATA_INCREASE;

const BPF_ALIGN_OF_U128: usize = 8;

/// Metadata for one parsed account slot in the loader input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawAccountIndex {
    /// Index of this slot in the original loader account array.
    pub original_index: usize,
    /// Canonical account index this slot resolves to, if duplicated.
    pub duplicate_of: Option<usize>,
}

impl RawAccountIndex {
    /// Whether this slot is a duplicate reference to an earlier account.
    #[inline(always)]
    pub const fn is_duplicate(&self) -> bool {
        self.duplicate_of.is_some()
    }
}

/// Instruction tail discovered after scanning the loader input buffer.
#[derive(Clone)]
pub struct RawInstructionFrame {
    pub accounts_start: *mut u8,
    pub account_count: usize,
    pub instruction_data: &'static [u8],
    pub program_id: Address,
}

/// Deserialize the loader input into `AccountView`s.
///
/// Duplicate-account resolution happens here. A duplicate slot reuses the
/// canonical `RuntimeAccount` pointer of the earlier slot it references, and
/// its `original_index` remains the loader slot where it appeared.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
pub unsafe fn deserialize_accounts<const MAX: usize>(
    input: *mut u8,
    accounts: &mut [MaybeUninit<AccountView>; MAX],
) -> (Address, usize, &'static [u8]) {
    let frame = unsafe { scan_instruction_frame(input) };

    let mut offset = 8usize;
    let count = frame.account_count.min(MAX);

    let mut slot = 0usize;
    while slot < count {
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            let raw = unsafe { input.add(offset) as *mut RuntimeAccount };
            accounts[slot] = MaybeUninit::new(unsafe { AccountView::new_unchecked(raw) });

            let data_len = unsafe { (*raw).data_len as usize };
            offset += RuntimeAccount::SIZE;
            offset += data_len + MAX_PERMITTED_DATA_INCREASE;
            offset += unsafe { input.add(offset).align_offset(BPF_ALIGN_OF_U128) };
            offset += 8;
        } else {
            let duplicate_of = marker as usize;
            let raw = if duplicate_of < slot {
                unsafe { accounts[duplicate_of].assume_init_ref().raw_ptr() }
            } else if slot > 0 {
                unsafe { accounts[0].assume_init_ref().raw_ptr() }
            } else {
                core::ptr::null_mut()
            };

            accounts[slot] = MaybeUninit::new(unsafe { AccountView::new_unchecked(raw) });
            offset += 8;
        }

        slot += 1;
    }

    while slot < frame.account_count {
        let marker = unsafe { *input.add(offset) };
        if marker == u8::MAX {
            let raw = unsafe { input.add(offset) as *const RuntimeAccount };
            let data_len = unsafe { (*raw).data_len as usize };
            offset += RuntimeAccount::SIZE;
            offset += data_len + MAX_PERMITTED_DATA_INCREASE;
            offset += unsafe { input.add(offset).align_offset(BPF_ALIGN_OF_U128) };
            offset += 8;
        } else {
            offset += 8;
        }
        slot += 1;
    }

    (frame.program_id, count, frame.instruction_data)
}

/// Parse just the instruction tail and account span from the loader input.
///
/// This supports both eager entrypoint parsing and lazy account iteration.
/// The returned frame carries the original account span start so duplicate and
/// canonical-account relationships remain defined at the loader level.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
pub unsafe fn scan_instruction_frame(input: *mut u8) -> RawInstructionFrame {
    let mut scan = input;

    let num_accounts = unsafe { *(scan as *const u64) as usize };
    scan = unsafe { scan.add(8) };
    let accounts_start = scan;

    let mut slot = 0usize;
    while slot < num_accounts {
        let marker = unsafe { *scan };
        if marker == u8::MAX {
            let raw = scan as *const RuntimeAccount;
            let data_len = unsafe { (*raw).data_len as usize };
            let mut step = RuntimeAccount::SIZE + data_len + MAX_PERMITTED_DATA_INCREASE;
            step += unsafe { scan.add(step).align_offset(BPF_ALIGN_OF_U128) };
            step += 8;
            scan = unsafe { scan.add(step) };
        } else {
            scan = unsafe { scan.add(8) };
        }
        slot += 1;
    }

    let data_len = unsafe { *(scan as *const u64) as usize };
    scan = unsafe { scan.add(8) };
    let instruction_data = unsafe { core::slice::from_raw_parts(scan as *const u8, data_len) };
    scan = unsafe { scan.add(data_len) };

    let program_id_ptr = scan as *const [u8; 32];
    let program_id = Address::new_from_array(unsafe { *program_id_ptr });

    RawInstructionFrame {
        accounts_start,
        account_count: num_accounts.min(254),
        instruction_data,
        program_id,
    }
}
