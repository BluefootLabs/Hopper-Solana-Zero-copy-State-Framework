//! Solana address type -- 32-byte public key.

/// Number of bytes in an address.
pub const ADDRESS_BYTES: usize = 32;

/// Maximum length of a single PDA seed.
pub const MAX_SEED_LEN: usize = 32;

/// Maximum number of seeds for PDA derivation.
pub const MAX_SEEDS: usize = 16;

/// Marker appended to PDA hash inputs: `"ProgramDerivedAddress"`.
pub const PDA_MARKER: &[u8; 21] = b"ProgramDerivedAddress";

/// A Solana address (public key): 32 bytes, transparent layout.
#[repr(transparent)]
#[cfg_attr(feature = "copy", derive(Copy))]
#[derive(Clone, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Address(pub(crate) [u8; 32]);

impl Address {
    /// Construct from a raw byte array.
    #[inline(always)]
    pub const fn new_from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the underlying bytes by value.
    #[inline(always)]
    pub const fn to_bytes(&self) -> [u8; 32] {
        self.0
    }

    /// Borrow the underlying byte array.
    #[inline(always)]
    pub const fn as_array(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for Address {
    #[inline(always)]
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<Address> for [u8; 32] {
    #[inline(always)]
    fn from(addr: Address) -> [u8; 32] {
        addr.0
    }
}

impl TryFrom<&[u8]> for Address {
    type Error = core::array::TryFromSliceError;

    #[inline]
    fn try_from(slice: &[u8]) -> Result<Self, Self::Error> {
        let arr: [u8; 32] = slice.try_into()?;
        Ok(Self(arr))
    }
}

impl AsRef<[u8]> for Address {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsMut<[u8]> for Address {
    #[inline(always)]
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl AsRef<[u8; 32]> for Address {
    #[inline(always)]
    fn as_ref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl core::hash::Hash for Address {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl core::fmt::Debug for Address {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Address({:?})", &self.0[..4])
    }
}

/// Fast address equality using 4 x u64 comparison.
#[inline(always)]
pub fn address_eq(a: &Address, b: &Address) -> bool {
    let a_ptr = a.0.as_ptr() as *const u64;
    let b_ptr = b.0.as_ptr() as *const u64;
    // SAFETY: Address is 32 bytes = 4 x u64. The #[repr(transparent)]
    // layout guarantees the bytes are contiguous. We compare as u64
    // for fewer instructions.
    unsafe {
        *a_ptr == *b_ptr
            && *a_ptr.add(1) == *b_ptr.add(1)
            && *a_ptr.add(2) == *b_ptr.add(2)
            && *a_ptr.add(3) == *b_ptr.add(3)
    }
}

/// Compile-time base58 address literal.
///
/// Usage: `const MY_ADDR: Address = address!("11111111111111111111111111111111");`
#[macro_export]
macro_rules! address {
    ( $literal:expr ) => {
        $crate::address::Address::new_from_array(five8_const::decode_32_const($literal))
    };
}
