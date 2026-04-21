//! Typed pubkey addresses -- compile-time account-key safety.
//!
//! `TypedAddress<T>` is a transparent wrapper over `[u8; 32]` that carries
//! a phantom layout type. This prevents mixing up account keys of different
//! types at compile time (e.g., passing a vault address where a mint is expected).
//!
//! ## Wire representation
//!
//! Identical to `[u8; 32]` -- align 1, 32 bytes, all bit patterns valid.
//! Safe to overlay from any account data offset without padding.
//!
//! ## Usage
//!
//! ```ignore
//! hopper_layout! {
//!     pub struct Pool, disc = 5, version = 1 {
//!         authority: TypedAddress<Authority> = 32,  // or just [u8; 32]
//!         mint_a:    TypedAddress<Mint>      = 32,
//!         mint_b:    TypedAddress<Mint>      = 32,
//!         balance:   WireU64                 = 8,
//!     }
//! }
//!
//! // Type error: can't pass TypedAddress<Mint> where TypedAddress<Authority> is expected
//! fn check_authority(addr: &TypedAddress<Authority>, signer: &AccountView) -> ProgramResult { ... }
//! ```

use core::marker::PhantomData;

/// A 32-byte public key tagged with a phantom layout type.
///
/// Zero-cost: `#[repr(transparent)]` over `[u8; 32]`, align 1.
/// The type parameter `T` exists only at compile time for type safety --
/// it has no runtime representation.
#[repr(transparent)]
pub struct TypedAddress<T> {
    bytes: [u8; 32],
    _phantom: PhantomData<T>,
}

// Manual Copy/Clone to avoid requiring T: Copy/Clone (T is phantom-only).
impl<T> Clone for TypedAddress<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for TypedAddress<T> {}

impl<T> core::fmt::Debug for TypedAddress<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TypedAddress({:02x?})", &self.bytes[..4])
    }
}

// Compile-time guarantees
const _: () = assert!(core::mem::size_of::<TypedAddress<()>>() == 32);
const _: () = assert!(core::mem::align_of::<TypedAddress<()>>() == 1);

// Bytemuck proof (Hopper Safety Audit Must-Fix #5). Blanket over `T`
// is fine because `TypedAddress<T>` is `#[repr(transparent)]` over
// `[u8; 32]` and `T` only participates as `PhantomData` — the wire
// payload doesn't depend on `T` at all.
#[cfg(feature = "hopper-native-backend")]
unsafe impl<T: 'static> ::hopper_runtime::__hopper_native::bytemuck::Zeroable for TypedAddress<T> {}
#[cfg(feature = "hopper-native-backend")]
unsafe impl<T: Copy + 'static> ::hopper_runtime::__hopper_native::bytemuck::Pod for TypedAddress<T> {}

// SAFETY: #[repr(transparent)] over [u8; 32], all bit patterns valid, align 1.
unsafe impl<T: Copy + 'static> crate::account::Pod for TypedAddress<T> {}
// Audit Step 5 seal: Hopper-authored primitive.
unsafe impl<T: Copy + 'static> ::hopper_runtime::__sealed::HopperZeroCopySealed for TypedAddress<T> {}

impl<T> crate::account::FixedLayout for TypedAddress<T> {
    const SIZE: usize = 32;
}

impl<T> TypedAddress<T> {
    /// Create a typed address from raw bytes.
    #[inline(always)]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self {
            bytes,
            _phantom: PhantomData,
        }
    }

    /// Create from a byte slice reference (copies 32 bytes).
    #[inline(always)]
    pub fn from_slice(slice: &[u8; 32]) -> Self {
        Self::new(*slice)
    }

    /// Create from an `AccountView`'s address.
    #[inline(always)]
    pub fn from_account(account: &hopper_runtime::AccountView) -> Self {
        // SAFETY: Address is [u8; 32].
        let bytes = unsafe {
            *(account.address() as *const hopper_runtime::Address as *const [u8; 32])
        };
        Self::new(bytes)
    }

    /// The zero address (all bytes 0).
    #[inline(always)]
    pub const fn zeroed() -> Self {
        Self::new([0u8; 32])
    }

    /// Raw 32-byte key.
    #[inline(always)]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Check if this address equals a raw 32-byte key.
    #[inline(always)]
    pub fn eq_bytes(&self, other: &[u8; 32]) -> bool {
        crate::check::keys_eq_fast(&self.bytes, other)
    }

    /// Check if this address matches an `AccountView`'s address.
    #[inline(always)]
    pub fn eq_account(&self, account: &hopper_runtime::AccountView) -> bool {
        // SAFETY: Address is [u8; 32], same as our bytes.
        let addr = unsafe {
            &*(account.address() as *const hopper_runtime::Address as *const [u8; 32])
        };
        crate::check::keys_eq_fast(&self.bytes, addr)
    }

    /// Verify this address matches the given account, returning an error if not.
    #[inline(always)]
    pub fn require_eq_account(
        &self,
        account: &hopper_runtime::AccountView,
    ) -> Result<(), hopper_runtime::error::ProgramError> {
        if self.eq_account(account) {
            Ok(())
        } else {
            Err(hopper_runtime::error::ProgramError::InvalidAccountData)
        }
    }

    /// Check if this is the zero address.
    #[inline(always)]
    pub fn is_zero(&self) -> bool {
        crate::check::is_zero_address(&self.bytes)
    }

    /// Cast this typed address to a different type.
    ///
    /// Use sparingly -- this exists for interop with legacy/untyped code.
    #[inline(always)]
    pub const fn cast<U>(self) -> TypedAddress<U> {
        TypedAddress {
            bytes: self.bytes,
            _phantom: PhantomData,
        }
    }

    /// Erase the type tag, returning an untyped address.
    #[inline(always)]
    pub const fn untyped(self) -> UntypedAddress {
        UntypedAddress(self.bytes)
    }
}

impl<T> PartialEq for TypedAddress<T> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        crate::check::keys_eq_fast(&self.bytes, &other.bytes)
    }
}

impl<T> Eq for TypedAddress<T> {}

impl<T> PartialEq<[u8; 32]> for TypedAddress<T> {
    #[inline(always)]
    fn eq(&self, other: &[u8; 32]) -> bool {
        crate::check::keys_eq_fast(&self.bytes, other)
    }
}

impl<T> AsRef<[u8; 32]> for TypedAddress<T> {
    #[inline(always)]
    fn as_ref(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl<T> AsRef<[u8]> for TypedAddress<T> {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl<T> From<[u8; 32]> for TypedAddress<T> {
    #[inline(always)]
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

impl<T> From<TypedAddress<T>> for [u8; 32] {
    #[inline(always)]
    fn from(addr: TypedAddress<T>) -> [u8; 32] {
        addr.bytes
    }
}

/// An untyped 32-byte address (interop bridge).
///
/// Useful for storing addresses where the account type is not known
/// at compile time, or when interfacing with raw hopper-native APIs.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct UntypedAddress(pub [u8; 32]);

const _: () = assert!(core::mem::size_of::<UntypedAddress>() == 32);
const _: () = assert!(core::mem::align_of::<UntypedAddress>() == 1);

// Bytemuck proof (Hopper Safety Audit Must-Fix #5).
#[cfg(feature = "hopper-native-backend")]
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for UntypedAddress {}
#[cfg(feature = "hopper-native-backend")]
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for UntypedAddress {}

// SAFETY: Transparent over [u8; 32], align 1, all bits valid.
unsafe impl crate::account::Pod for UntypedAddress {}
// Audit Step 5 seal: Hopper-authored primitive.
unsafe impl ::hopper_runtime::__sealed::HopperZeroCopySealed for UntypedAddress {}

impl crate::account::FixedLayout for UntypedAddress {
    const SIZE: usize = 32;
}

impl UntypedAddress {
    /// Tag this address with a layout type.
    #[inline(always)]
    pub const fn typed<T>(self) -> TypedAddress<T> {
        TypedAddress::new(self.0)
    }

    /// Raw bytes.
    #[inline(always)]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl AsRef<[u8; 32]> for UntypedAddress {
    #[inline(always)]
    fn as_ref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for UntypedAddress {
    #[inline(always)]
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

// -- Marker types for common account roles --

/// Marker: an authority/signer address.
pub struct Authority;

/// Marker: a mint address.
pub struct Mint;

/// Marker: a token account address.
pub struct TokenAccount;

/// Alias for [`TokenAccount`].
pub type Token = TokenAccount;

/// Marker: a program address (executable).
pub struct Program;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_address_size_and_align() {
        assert_eq!(core::mem::size_of::<TypedAddress<Authority>>(), 32);
        assert_eq!(core::mem::align_of::<TypedAddress<Authority>>(), 1);
        assert_eq!(core::mem::size_of::<TypedAddress<Mint>>(), 32);
        assert_eq!(core::mem::size_of::<UntypedAddress>(), 32);
    }

    #[test]
    fn typed_address_equality() {
        let bytes = [42u8; 32];
        let a: TypedAddress<Authority> = TypedAddress::new(bytes);
        let b: TypedAddress<Authority> = TypedAddress::new(bytes);
        assert_eq!(a, b);
        assert!(a.eq_bytes(&bytes));
    }

    #[test]
    fn typed_address_zero_check() {
        let zero: TypedAddress<Mint> = TypedAddress::new([0u8; 32]);
        assert!(zero.is_zero());

        let nonzero: TypedAddress<Mint> = TypedAddress::new([1u8; 32]);
        assert!(!nonzero.is_zero());
    }

    #[test]
    fn typed_address_cast() {
        let mint_addr: TypedAddress<Mint> = TypedAddress::new([7u8; 32]);
        let _generic: TypedAddress<()> = mint_addr.cast();
    }

    #[test]
    fn typed_untyped_roundtrip() {
        let bytes = [99u8; 32];
        let typed: TypedAddress<TokenAccount> = TypedAddress::new(bytes);
        let untyped = typed.untyped();
        let retyped: TypedAddress<TokenAccount> = untyped.typed();
        assert_eq!(typed, retyped);
    }
}
