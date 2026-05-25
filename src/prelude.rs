//! Framework-mode prelude for authored Hopper programs.
//!
//! This import is intentionally small: account wrappers, instruction context,
//! common wire types, result/error types, guard macros, CPI/token facade modules,
//! and the canonical proc macros. Protocol-grade state machinery lives behind
//! `hopper::systems::*` or explicit modules such as `hopper::segment`.

pub use crate::account::{
    Account, InitAccount, Interface, InterfaceAccount, InterfaceAccountLayout,
    InterfaceAccountResolve, InterfaceSpec, Program, ProgramId, Signer, System, SystemAccount,
    SystemId, UncheckedAccount,
};
pub use crate::context::Context;
pub use crate::context::Context as Ctx;
pub use hopper_runtime::{
    AccountView, Address, HopperString, HopperVec, Pod, ProgramError, ProgramResult, TailBytes,
    TailCodec, TailElement, TailStr,
};

/// Lifetime-shaped bounded UTF-8 authoring value.
///
/// Hopper account macros accept `String<'a, N>` as the canonical pretty
/// dynamic-field syntax and lower it into the compact tail model. This wrapper
/// makes the same spelling available to type checkers without changing the
/// owned runtime representation, which remains [`HopperString<N>`].
#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct HopperAuthoringString<'a, const N: usize> {
    inner: HopperString<N>,
    _lifetime: core::marker::PhantomData<&'a ()>,
}

impl<'a, const N: usize> HopperAuthoringString<'a, N> {
    #[inline]
    pub const fn empty() -> Self {
        Self {
            inner: HopperString::empty(),
            _lifetime: core::marker::PhantomData,
        }
    }

    #[inline]
    pub fn from_str(value: &str) -> Result<Self> {
        Ok(Self::from_hopper(HopperString::from_str(value)?))
    }

    #[inline]
    pub const fn from_hopper(inner: HopperString<N>) -> Self {
        Self {
            inner,
            _lifetime: core::marker::PhantomData,
        }
    }

    #[inline(always)]
    pub const fn as_hopper(&self) -> &HopperString<N> {
        &self.inner
    }

    #[inline(always)]
    pub fn into_hopper(self) -> HopperString<N> {
        self.inner
    }

    #[inline]
    pub fn set_str(&mut self, value: &str) -> Result<()> {
        self.inner.set_str(value)
    }

    #[inline]
    pub fn as_str(&self) -> Result<&str> {
        self.inner.as_str()
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }
}

impl<'a, const N: usize> Default for HopperAuthoringString<'a, N> {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<'a, const N: usize> TailCodec for HopperAuthoringString<'a, N> {
    const MAX_ENCODED_LEN: usize = HopperString::<N>::MAX_ENCODED_LEN;

    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize> {
        self.inner.encode(out)
    }

    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize)> {
        let (inner, used) = HopperString::<N>::decode(input)?;
        Ok((Self::from_hopper(inner), used))
    }
}

/// Lifetime-shaped bounded list authoring value.
#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct HopperAuthoringVec<'a, T, const N: usize>
where
    T: TailCodec + Copy + Default,
{
    inner: HopperVec<T, N>,
    _lifetime: core::marker::PhantomData<&'a ()>,
}

impl<'a, T, const N: usize> HopperAuthoringVec<'a, T, N>
where
    T: TailCodec + Copy + Default,
{
    #[inline]
    pub fn empty() -> Self {
        Self::from_hopper(HopperVec::empty())
    }

    #[inline]
    pub const fn from_hopper(inner: HopperVec<T, N>) -> Self {
        Self {
            inner,
            _lifetime: core::marker::PhantomData,
        }
    }

    #[inline(always)]
    pub const fn as_hopper(&self) -> &HopperVec<T, N> {
        &self.inner
    }

    #[inline(always)]
    pub fn into_hopper(self) -> HopperVec<T, N> {
        self.inner
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        self.inner.as_slice()
    }

    #[inline]
    pub fn push(&mut self, value: T) -> Result<()> {
        self.inner.push(value)
    }
}

impl<'a, T, const N: usize> HopperAuthoringVec<'a, T, N>
where
    T: TailCodec + Copy + Default + PartialEq,
{
    #[inline]
    pub fn push_unique(&mut self, value: T) -> Result<bool> {
        self.inner.push_unique(value)
    }

    #[inline]
    pub fn remove_first(&mut self, value: &T) -> bool {
        self.inner.remove_first(value)
    }
}

impl<'a, T, const N: usize> Default for HopperAuthoringVec<'a, T, N>
where
    T: TailCodec + Copy + Default,
{
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<'a, T, const N: usize> TailCodec for HopperAuthoringVec<'a, T, N>
where
    T: TailCodec + Copy + Default,
{
    const MAX_ENCODED_LEN: usize = HopperVec::<T, N>::MAX_ENCODED_LEN;

    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize> {
        self.inner.encode(out)
    }

    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize)> {
        let (inner, used) = HopperVec::<T, N>::decode(input)?;
        Ok((Self::from_hopper(inner), used))
    }
}

/// Short alias for bounded dynamic UTF-8 fields outside account authoring.
pub type Text<const N: usize> = HopperString<N>;

/// Short alias for bounded dynamic list fields outside account authoring.
pub type List<T, const N: usize> = HopperVec<T, N>;

/// Option-A Quasar-shaped alias for bounded dynamic UTF-8 authoring values.
///
/// In `#[account]` structs, `String<'a, N>` is the canonical pretty form that
/// the macro lowers into Hopper's compact dynamic tail. In ordinary Rust code,
/// use [`HopperString<N>`] or [`Text<N>`] for the owned bounded value.
pub type String<'a, const N: usize> = HopperAuthoringString<'a, N>;

/// Option-A Quasar-shaped alias for bounded dynamic list authoring values.
///
/// In `#[account]` structs, `Vec<'a, T, N>` is the canonical pretty form that
/// the macro lowers into Hopper's compact dynamic tail. In ordinary Rust code,
/// use [`HopperVec<T, N>`] or [`List<T, N>`] for the owned bounded value.
pub type Vec<'a, T, const N: usize> = HopperAuthoringVec<'a, T, N>;

/// Solana-familiar alias for Hopper's 32-byte address type.
pub type Pubkey = Address;

/// Handler result alias for examples that prefer `Result<()>` spelling.
pub type Result<T = (), E = ProgramError> = core::result::Result<T, E>;

pub use hopper_core::abi::{WireBool, WireU16, WireU32, WireU64};

pub use crate::{associated_token, cpi, crypto, events, memo, pda, system, token, token_2022};
pub use hopper_associated_token::ATA_PROGRAM_ID;
pub use hopper_memo::{Memo, MAX_MEMO_SIGNERS, MEMO_PROGRAM_ID};
pub use hopper_solana::interface::{
    interface_transfer_checked, interface_transfer_checked_signed, InterfaceMint,
    InterfaceTokenAccount, TokenProgramKind,
};
pub use hopper_system::SYSTEM_PROGRAM_ID;
pub use hopper_token::TOKEN_PROGRAM_ID;
pub use hopper_token_2022::TOKEN_2022_PROGRAM_ID;

#[cfg(feature = "metaplex")]
pub use hopper_metaplex;
#[cfg(feature = "metaplex")]
pub use hopper_metaplex::{
    master_edition_pda, master_edition_pda_with_bump, metadata_pda, metadata_pda_with_bump,
    CreateMasterEditionV3, CreateMetadataAccountV3, DataV2, UpdateMetadataAccountV2,
    MPL_TOKEN_METADATA_PROGRAM_ID,
};

pub use crate::const_assert_pod;
pub use hopper_runtime::{
    address, err, error, hopper_emit_cpi, hopper_log, msg, require, require_eq, require_gt,
    require_gte, require_keys_eq, require_keys_neq, require_lt, require_lte, require_neq,
};

#[cfg(feature = "proc-macros")]
pub use crate::{account, accounts, args, error_code, event, program, Accounts, HopperInitSpace};
