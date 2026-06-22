//! `segment_ref::<u64>` must NOT compile (the safe overlay path).
//!
//! Even though `segment_ref` is a *safe* method, it returns a typed
//! reference into account bytes, so its type parameter must be
//! alignment-1. `u64` is not `Pod`, so this is a compile error rather
//! than latent alignment UB at an unaligned offset.

use hopper::hopper_runtime::SegmentBorrowRegistry;
use hopper::prelude::AccountView;

fn must_fail(account: &AccountView<'_>, borrows: &mut SegmentBorrowRegistry) {
    let _ = account.segment_ref::<u64>(borrows, 0, 8);
}

fn main() {}
