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

/// Decode a base58 Solana address literal into 32 bytes at compile time.
pub const fn decode_base58_32(input: &str) -> [u8; 32] {
    let bytes = input.as_bytes();
    let mut out = [0u8; ADDRESS_BYTES];
    let mut i = 0;

    while i < bytes.len() {
        let mut carry = base58_digit(bytes[i]) as u32;
        let mut j = ADDRESS_BYTES;

        while j > 0 {
            j -= 1;
            let value = (out[j] as u32) * 58 + carry;
            out[j] = value as u8;
            carry = value >> 8;
        }

        if carry != 0 {
            panic!("base58 address literal overflows 32 bytes");
        }

        i += 1;
    }

    out
}

const fn base58_digit(byte: u8) -> u8 {
    match byte {
        b'1'..=b'9' => byte - b'1',
        b'A'..=b'H' => byte - b'A' + 9,
        b'J'..=b'N' => byte - b'J' + 17,
        b'P'..=b'Z' => byte - b'P' + 22,
        b'a'..=b'k' => byte - b'a' + 33,
        b'm'..=b'z' => byte - b'm' + 44,
        _ => panic!("invalid base58 address literal"),
    }
}

/// Address equality over raw bytes.
#[inline(always)]
pub fn address_eq(a: &Address, b: &Address) -> bool {
    a.0 == b.0
}

/// Compile-time base58 address literal.
///
/// Usage: `const MY_ADDR: Address = address!("11111111111111111111111111111111");`
#[macro_export]
macro_rules! address {
    ( $literal:expr ) => {
        $crate::address::Address::new_from_array($crate::address::decode_base58_32($literal))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_system_program_literal() {
        const SYSTEM: [u8; 32] = decode_base58_32("11111111111111111111111111111111");
        assert_eq!(SYSTEM, [0u8; 32]);
    }

    #[test]
    fn address_macro_uses_local_decoder() {
        const SYSTEM: Address = crate::address!("11111111111111111111111111111111");
        assert_eq!(SYSTEM.to_bytes(), [0u8; 32]);
    }
}
