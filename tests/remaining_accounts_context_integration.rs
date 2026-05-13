#![cfg(feature = "proc-macros")]

use hopper::prelude::*;

#[hopper::context]
pub struct UsesRemainingAccounts {
    pub authority: AccountView,
}

fn accessors_typecheck(
    ctx: &UsesRemainingAccountsCtx<'_, '_>,
) -> hopper::hopper_runtime::RemainingMode {
    let strict = ctx.remaining_accounts();
    let passthrough = ctx.remaining_accounts_passthrough();
    let raw = ctx.remaining_accounts_raw();

    let _ = strict.as_slice();
    let _ = passthrough.as_slice();
    let _ = raw.len();
    let _ = strict.account_views::<4>();
    let _ = ctx.remaining_accounts().signers::<4>();

    strict.mode()
}

#[test]
fn context_generates_remaining_account_modes() {
    let _ = accessors_typecheck;
}
