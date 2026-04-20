#![no_main]
//! Fuzz target for `hopper_core::account::pod_from_bytes::<T>`.
//!
//! Contract: given any byte slice, `pod_from_bytes::<T>(buf)` returns
//! `Ok(&T)` exactly when `buf.len() == size_of::<T>()` AND `buf` is
//! properly aligned for `T`, otherwise returns an error. It must
//! never panic, never UB, never expose uninitialized data.
//!
//! Also exercises `pod_read::<T>` (value-read) for differential
//! consistency: when both succeed, the bytes the reference sees must
//! match the value.

use libfuzzer_sys::fuzz_target;

use hopper_core::abi::WireU64;
use hopper_core::account::{pod_from_bytes, pod_read};

fuzz_target!(|data: &[u8]| {
    // Reference overlay.
    if let Ok(by_ref) = pod_from_bytes::<WireU64>(data) {
        // If the reference succeeded, the value read must also succeed
        // and produce byte-identical output.
        let by_val: WireU64 = pod_read::<WireU64>(data).expect("value path must agree");
        assert_eq!(by_ref.as_bytes(), by_val.as_bytes());
    }
});
