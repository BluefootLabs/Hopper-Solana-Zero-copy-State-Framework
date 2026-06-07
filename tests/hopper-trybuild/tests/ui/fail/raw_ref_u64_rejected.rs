//! Enforce alignment policy at the compiler boundary:
//!
//! `raw_ref::<u64>` must not compile because native `u64` is not
//! alignment-1 and therefore is not `Pod` under Hopper's zero-copy model.

use hopper::prelude::AccountView;

fn must_fail(account: &AccountView<'_>) {
    let _ = unsafe { account.raw_ref::<u64>() };
}

fn main() {}
