//! Typed wrapper and hero-context integration tests.

#![cfg(feature = "proc-macros")]

use hopper::__runtime::{
    Account, HopperSigner, InitAccount, Interface, InterfaceAccount, InterfaceAccountLayout,
    InterfaceSpec, Program, ProgramId, SystemAccount, SystemId, UncheckedAccount,
};

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
fn interface_wrappers_are_zero_cost() {
    assert_eq!(
        core::mem::size_of::<Interface<'static, VaultInterface>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
    assert_eq!(
        core::mem::size_of::<InterfaceAccount<'static, TinyLayout>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
    assert!(VaultInterface::contains(&VAULT_PROGRAM_A));
    assert!(VaultInterface::contains(&VAULT_PROGRAM_B));
    assert!(!VaultInterface::contains(&SystemId::ID));
}

#[test]
fn unchecked_and_system_wrappers_are_zero_cost() {
    assert_eq!(
        core::mem::size_of::<UncheckedAccount<'static>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
    assert_eq!(
        core::mem::size_of::<SystemAccount<'static>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>()
    );
}

#[test]
fn prelude_exports_memo_and_token_interface_helpers() {
    use hopper::prelude::{
        interface_transfer_checked, HopperString, HopperVec, Interface as PreludeInterface,
        InterfaceAccount as PreludeInterfaceAccount, InterfaceAccountResolve as PreludeResolve,
        InterfaceMint, InterfaceTokenAccount, List, Memo, String as HopperPrettyString, TailCodec,
        TailElement, Text, TokenProgramKind, Vec as HopperPrettyVec, MAX_MEMO_SIGNERS,
        MEMO_PROGRAM_ID,
    };

    fn assert_prelude_interface_spec<T: hopper::prelude::InterfaceSpec>() {}
    fn assert_prelude_interface_layout<T: hopper::prelude::InterfaceAccountLayout>() {}
    fn assert_prelude_interface_resolve<T: PreludeResolve>() {}

    fn assert_tail_element<T: TailElement>() {}

    let _transfer = interface_transfer_checked;
    let _memo_program = MEMO_PROGRAM_ID;
    assert_eq!(MAX_MEMO_SIGNERS, 16);
    assert_tail_element::<u16>();
    assert_eq!(
        <HopperVec<u16, 4> as TailCodec>::MAX_ENCODED_LEN,
        2 + (4 * 2),
    );
    let label = HopperString::<8>::from_str("ops").unwrap();
    assert_eq!(label.as_str().unwrap(), "ops");
    let label_alias = Text::<8>::from_str("ops").unwrap();
    assert_eq!(label_alias.as_str().unwrap(), "ops");
    let string_alias = HopperPrettyString::<'static, 8>::from_str("ops").unwrap();
    assert_eq!(string_alias.as_str().unwrap(), "ops");
    assert_eq!(<List<u16, 4> as TailCodec>::MAX_ENCODED_LEN, 10);
    assert_eq!(
        <HopperPrettyVec<'static, u16, 4> as TailCodec>::MAX_ENCODED_LEN,
        10
    );
    assert_eq!(
        core::mem::size_of::<TokenProgramKind>(),
        core::mem::size_of::<u8>(),
    );
    assert_eq!(
        core::mem::size_of::<InterfaceTokenAccount<'static>>(),
        core::mem::size_of::<(&'static [u8], TokenProgramKind)>(),
    );
    assert_eq!(
        core::mem::size_of::<InterfaceMint<'static>>(),
        core::mem::size_of::<(&'static [u8], TokenProgramKind)>(),
    );
    assert_eq!(
        core::mem::size_of::<PreludeInterface<'static, VaultInterface>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>(),
    );
    assert_eq!(
        core::mem::size_of::<PreludeInterfaceAccount<'static, TinyLayout>>(),
        core::mem::size_of::<&'static hopper::__runtime::AccountView>(),
    );
    assert_prelude_interface_spec::<VaultInterface>();
    assert_prelude_interface_layout::<TinyLayout>();
    assert_prelude_interface_resolve::<AnyTinyLayout>();
    assert!(core::mem::size_of::<Memo<'static, 'static, 'static>>() > 0);
}

#[test]
fn substrate_exports_runtime_and_native_symbols() {
    use hopper::substrate::{AccountView, Address, NativeAccountView, NativeAddress};

    assert_eq!(
        core::mem::size_of::<Address>(),
        core::mem::size_of::<NativeAddress>()
    );
    assert_eq!(
        core::mem::size_of::<AccountView>(),
        core::mem::size_of::<NativeAccountView>()
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

#[test]
fn state_helpers_accept_native_wire_values() {
    let mut layout = TinyLayout::new(41);
    assert_eq!(layout.v.get(), 41);
    layout.set_inner(42).unwrap();
    assert_eq!(layout.v.get(), 42);
}

#[allow(dead_code)]
#[hopper::program]
mod hero_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: hopper::prelude::Ctx<Increment>) -> hopper::prelude::ProgramResult {
        let _bumps = ctx.bumps;
        let mut account = ctx.accounts.counter.get_mut()?;
        account.v.checked_add_assign(1)?;
        Ok(())
    }

    #[instruction(1)]
    pub fn read_interface(
        ctx: hopper::prelude::Ctx<ReadInterface>,
    ) -> hopper::prelude::ProgramResult {
        let _program = ctx.accounts.vault_program.key();
        let _layout = ctx.accounts.remote_vault.get()?;
        Ok(())
    }

    #[instruction(2)]
    pub fn read_any_interface(
        ctx: hopper::prelude::Ctx<ReadAnyInterface>,
    ) -> hopper::prelude::ProgramResult {
        match ctx.accounts.remote_vault.resolve()? {
            AnyTiny::Current(layout) => {
                let _ = layout.v.get();
            }
            AnyTiny::Next(layout) => {
                let _ = layout.v.get();
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[hopper::program(profile = "tiny", entrypoint = false)]
mod tiny_profile_program {
    #[instruction(0)]
    pub fn noop(_ctx: &mut hopper::prelude::Context<'_>) -> hopper::prelude::ProgramResult {
        Ok(())
    }
}

#[test]
fn program_profile_tiny_is_emitted() {
    assert_eq!(
        tiny_profile_program::HOPPER_PROGRAM_PROFILE,
        hopper::__runtime::HopperProgramProfile::TINY
    );
}

#[derive(hopper::Accounts)]
pub struct Increment<'info> {
    #[account(mut)]
    pub counter: hopper::prelude::Account<'info, TinyLayout>,
    pub authority: hopper::prelude::Signer<'info>,
}

#[derive(hopper::Accounts)]
pub struct ReadInterface<'info> {
    pub vault_program: hopper::prelude::Interface<'info, VaultInterface>,
    pub remote_vault: hopper::prelude::InterfaceAccount<'info, TinyLayout>,
}

#[derive(hopper::Accounts)]
pub struct Token2022NewExtensionSyntax<'info> {
    #[account(
        mint::token_program = hopper::token_2022::TOKEN_2022_PROGRAM_ID,
        extensions::confidential_transfer::mint,
        extensions::scaled_ui_amount::config,
    )]
    pub mint: hopper::prelude::UncheckedAccount<'info>,

    #[account(
        token::token_program = hopper::token_2022::TOKEN_2022_PROGRAM_ID,
        extensions::cpi_guard,
        extensions::confidential_transfer::account,
    )]
    pub token_account: hopper::prelude::UncheckedAccount<'info>,
}

#[derive(hopper::Accounts)]
pub struct ReadAnyInterface<'info> {
    pub remote_vault: hopper::prelude::InterfaceAccount<'info, AnyTinyLayout>,
}

// A minimal layout type used to satisfy the `T: LayoutContract`
// bound on `Account<'info, T>` / `InitAccount<'info, T>`.
#[derive(Copy, Clone)]
#[hopper::state(discriminator = 12, version = 1)]
#[repr(C)]
pub struct TinyLayout {
    pub v: hopper::prelude::WireU64,
}

#[derive(Copy, Clone)]
#[hopper::state(discriminator = 13, version = 2)]
#[repr(C)]
pub struct TinyLayoutV2 {
    pub v: hopper::prelude::WireU64,
}

hopper::interface_account_set! {
    pub struct AnyTinyLayout: VaultInterface;
    pub enum AnyTiny {
        Current(TinyLayout),
        Next(TinyLayoutV2),
    }
}

const VAULT_PROGRAM_A: hopper::__runtime::Address =
    hopper::__runtime::Address::new_from_array([0xA1; 32]);
const VAULT_PROGRAM_B: hopper::__runtime::Address =
    hopper::__runtime::Address::new_from_array([0xB2; 32]);

pub struct VaultInterface;

impl InterfaceSpec for VaultInterface {
    const IDS: &'static [hopper::__runtime::Address] = &[VAULT_PROGRAM_A, VAULT_PROGRAM_B];
}

impl InterfaceAccountLayout for TinyLayout {
    type Interface = VaultInterface;
}

impl InterfaceAccountLayout for TinyLayoutV2 {
    type Interface = VaultInterface;
}
