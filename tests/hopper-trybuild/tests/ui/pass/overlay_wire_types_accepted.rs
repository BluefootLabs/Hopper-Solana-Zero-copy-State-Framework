//! The positive companion to the `*_u64_rejected` fail fixtures.
//!
//! Alignment-1 wire types and byte arrays ARE valid overlays, so the
//! same `segment_ref` / `raw_ref` calls type-check when the element type
//! is `WireU64` (the canonical account field type) or `[u8; N]`. This
//! pins the happy path so the alignment guard cannot regress into
//! "everything is rejected".

use hopper::hopper_runtime::SegmentBorrowRegistry;
use hopper::prelude::{AccountView, WireU64};

fn ok(account: &AccountView<'_>, borrows: &mut SegmentBorrowRegistry) {
    let _ = account.segment_ref::<WireU64>(borrows, 0, 8);
    let _ = account.segment_ref::<[u8; 8]>(borrows, 0, 8);
    let _ = unsafe { account.raw_ref::<WireU64>() };
    let _ = unsafe { account.raw_ref::<[u8; 8]>() };
}

fn main() {}
