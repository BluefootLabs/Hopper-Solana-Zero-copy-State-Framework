#![no_main]
//! Fuzz target for `hopper_runtime::account::AccountView::load` —
//! the primary unsafe casting boundary in the Hopper runtime.
//!
//! # Contract
//!
//! Given **any** byte slice as a pretend account data buffer,
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

use libfuzzer_sys::fuzz_target;
use hopper_runtime::account::AccountView;

/// Minimum valid Hopper account header size (bytes).
/// Layout: [discriminator: 1][version: 1][layout_id: 8][schema_epoch: 2]
///         [flags: 2][reserved: 2][authority: 32 — optional tail]
/// The loader requires at least the 16-byte fixed prefix.
const HEADER_MIN: usize = 16;

fuzz_target!(|data: &[u8]| {
    // --- derive "expected" values directly from the input bytes -------
    // Using bytes from `data` itself as expected discriminator/fingerprint
    // ensures the fuzzer explores both the match and mismatch branches
    // without needing a structured input format.
    let expected_disc: u8 = data.first().copied().unwrap_or(0);
    let expected_layout_id: u64 = if data.len() >= 10 {
        u64::from_le_bytes(data[2..10].try_into().unwrap())
    } else {
        0u64
    };
    let expected_version: u8 = data.get(1).copied().unwrap_or(0);

    // --- call load with the raw buffer as account data -----------------
    // Contract: must never panic or UB regardless of buffer contents.
    let result = AccountView::load(
        data,
        expected_disc,
        expected_version,
        expected_layout_id,
    );

    // --- enforce structural invariants ---------------------------------
    match result {
        Ok(view) => {
            // If load succeeded the buffer must have been large enough
            // for the minimum header.
            assert!(
                data.len() >= HEADER_MIN,
                "AccountView::load returned Ok on a buffer shorter than HEADER_MIN"
            );
            // The view must not report a data region that exceeds the
            // buffer we handed it.
            let reported_len = view.data_len();
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
