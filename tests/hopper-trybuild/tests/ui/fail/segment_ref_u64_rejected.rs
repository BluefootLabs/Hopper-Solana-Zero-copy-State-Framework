//! Enforce alignment policy at the compiler boundary:
//!
//! `segment_ref::<u64>` must not compile because native `u64` is not
//! alignment-1 and therefore is not `Pod` under Hopper's zero-copy model.

use hopper::prelude::AccountView;

fn must_fail(account: &AccountView<'_>, borrows: &mut hopper::hopper_runtime::SegmentBorrowRegistry) {
    let _ = account.segment_ref::<u64>(borrows, 0, 8);
}

fn main() {}
