//! Alignment policy at the compiler boundary.
//!
//! `raw_ref::<u64>` must NOT compile: native `u64` has alignment 8, so
//! forming `&u64` at an arbitrary account offset is undefined behaviour.
//! Hopper's `Pod` marker is alignment-1-only, so `u64` is not `Pod` and
//! this overlay is rejected. Use `WireU64` for account fields and
//! `Context::read_data::<u64>` for by-value scalar decoding.

use hopper::prelude::AccountView;

fn must_fail(account: &AccountView<'_>) {
    let _ = unsafe { account.raw_ref::<u64>() };
}

fn main() {}
