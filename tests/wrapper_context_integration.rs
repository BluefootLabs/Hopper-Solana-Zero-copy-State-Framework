//! Stage 2.3 shipping verification: typed wrappers are usable as
//! first-class types. Full macro-side integration
//! (`#[hopper::context]` auto-accessor emission through
//! `Account<'info, T>` generics) requires lifetime-aware context
//! codegen. the wrappers ship as standalone now, and programs
//! that want Anchor-style spelling can adopt them via the
//! attribute path today.

#![cfg(feature = "proc-macros")]

use hopper::__runtime::{Account, HopperSigner, InitAccount, Program, ProgramId, SystemId};

#[test]
fn signer_wrapper_is_repr_transparent_pointer_sized() {
    assert_eq!(
        core::mem::size_of::<HopperSigner<'static>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
}

#[test]
fn system_program_id_is_canonical_zero_pubkey() {
    assert_eq!(SystemId::ID.as_array(), &[0u8; 32]);
}

#[test]
fn account_wrapper_phantom_data_is_zero_cost() {
    // `Account<'info, T>` is `#[repr(transparent)]` over `&AccountView`
    // plus a `PhantomData<T>`. PhantomData compiles away.
    use hopper::prelude::WireU64;
    assert_eq!(
        core::mem::size_of::<Account<'static, TinyLayout>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
    // Use WireU64 to keep TinyLayout referenced from the test.
    let _: Option<WireU64> = None;
}

#[test]
fn init_account_wrapper_phantom_data_is_zero_cost() {
    assert_eq!(
        core::mem::size_of::<InitAccount<'static, TinyLayout>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
}

#[test]
fn program_wrapper_phantom_data_is_zero_cost() {
    assert_eq!(
        core::mem::size_of::<Program<'static, SystemId>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
}

#[test]
fn custom_program_id_impl_is_addressable_at_const_time() {
    struct MyProgram;
    impl ProgramId for MyProgram {
        const ID: hopper::__runtime::Address =
            hopper::__runtime::Address::new_from_array([0x42u8; 32]);
    }
    assert_eq!(MyProgram::ID.as_array(), &[0x42u8; 32]);
}

// A minimal layout type used to satisfy the `T: LayoutContract`
// bound on `Account<'info, T>` / `InitAccount<'info, T>`.
#[hopper::state(disc = 12, version = 1)]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct TinyLayout {
    pub v: hopper::prelude::WireU64,
}
