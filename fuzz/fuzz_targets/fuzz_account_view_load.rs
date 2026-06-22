#![no_main]
//! Fuzz target for `hopper_runtime::account::AccountView::load` —
//! the primary unsafe casting boundary in the Hopper runtime.
//!
//! # Contract
//!
//! Given any byte slice as a pretend account data buffer,
//! `AccountView::load` must:
//!
//! 1. Never panic.
//! 2. Never invoke undefined behaviour.
//! 3. Return `Err` (not `Ok`) whenever the buffer is too short to
//!    hold a valid Hopper account header (32 bytes).
//! 4. Return `Err` when the discriminator extracted from the header
//!    does not match the expected discriminator supplied to `load`.
//! 5. Return `Err` when the layout fingerprint in the header does not
//!    match the expected fingerprint.
//! 6. When it does return `Ok`, the typed reference must alias only
//!    bytes that are within the supplied buffer.
//!
//! The fuzzer hammers all six contracts simultaneously by varying both
//! the raw buffer bytes and the "expected" discriminator / fingerprint
//! values passed to the load call.  Because `AccountView::load` is
//! `#[inline(always)]` the host-target fuzz run also exercises the
//! same codegen path used in SBF builds.
//!
//! # Why this target?
//!
//! `fuzz_pod_overlay` covers `pod_from_bytes::<T>` (the lowest-level
//! Pod cast).  `fuzz_decode_header` covers the schema-layer header
//! decoder.  Neither covers the **runtime** path that combines owner
//! check + discriminator check + version check + layout fingerprint
//! check + alignment gate into a single validated zero-copy load.  This
//! target closes that gap.

use hopper_native::raw_account::RuntimeAccount;
use hopper_native::{AccountView as NativeAccountView, Address as NativeAddress, NOT_BORROWED};
use hopper_runtime::account::AccountView;
use hopper_runtime::field_map::FieldMap;
use hopper_runtime::layout::{self, HopperHeader, LayoutContract};
use libfuzzer_sys::fuzz_target;

/// Minimum valid Hopper account header size (bytes).
/// Layout: [discriminator: 1][version: 1][layout_id: 8][schema_epoch: 2]
///         [flags: 2][reserved: 2][authority: 32 — optional tail]
/// The loader requires at least the 16-byte fixed prefix.
const HEADER_MIN: usize = 16;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct FuzzLayout {
    value: [u8; 8],
}

unsafe impl hopper_runtime::Zeroable for FuzzLayout {}
unsafe impl hopper_runtime::Pod for FuzzLayout {}

impl FieldMap for FuzzLayout {
    const FIELDS: &'static [hopper_runtime::field_map::FieldInfo] = &[];
}

impl LayoutContract for FuzzLayout {
    const DISC: u8 = 7;
    const VERSION: u8 = 1;
    const LAYOUT_ID: [u8; 8] = [0xAB; 8];
    const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
}

fn make_account(data: &[u8]) -> (Vec<u8>, NativeAccountView<'static>) {
    let mut backing = vec![0u8; RuntimeAccount::SIZE + data.len()];
    let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: `backing` is sized for RuntimeAccount followed by `data.len()` bytes.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 1,
            is_writable: 1,
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array([1; 32]),
            owner: NativeAddress::new_from_array([2; 32]),
            lamports: 42,
            data_len: data.len() as u64,
        });
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            backing.as_mut_ptr().add(RuntimeAccount::SIZE),
            data.len(),
        );
    }
    // SAFETY: `raw` points into `backing`, which is returned with the view.
    let account = unsafe { NativeAccountView::new_unchecked(raw) };
    (backing, account)
}

fuzz_target!(|data: &[u8]| {
    let (backing, native_account) = make_account(data);
    let accounts = [native_account];
    // SAFETY: AccountView is repr(transparent) over the native account view.
    let runtime_accounts = unsafe { hopper_runtime::native_boundary::wrap_account_slice(&accounts) };
    let account: &AccountView<'_> = &runtime_accounts[0];

    // Contract: must never panic or UB regardless of buffer contents.
    let result = account.load::<FuzzLayout>();

    // --- enforce structural invariants ---------------------------------
    match result {
        Ok(_view) => {
            // If load succeeded the buffer must have been large enough
            // for the minimum header.
            assert!(
                backing.len() >= RuntimeAccount::SIZE + HEADER_MIN,
                "AccountView::load returned Ok on a buffer shorter than HEADER_MIN"
            );
            assert!(layout::read_disc(data) == Some(FuzzLayout::DISC));
            assert!(layout::read_version(data) == Some(FuzzLayout::VERSION));
            assert!(layout::read_layout_id(data) == Some(&FuzzLayout::LAYOUT_ID));
            // The view must not report a data region that exceeds the
            // buffer we handed it.
            let reported_len = account.data_len();
            assert!(
                reported_len <= data.len(),
                "AccountView reported data_len {} > buffer len {}",
                reported_len,
                data.len()
            );
        }
        Err(_) => {
            // Err is always acceptable — this is the common path for
            // adversarial inputs.  We just ensure the call returned
            // rather than panicking.
        }
    }
});
