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
///
/// `PartialEq`/`Eq` are implemented manually (see below) so every
/// `Address == Address` -- and every owner / program-id check that
/// funnels through [`address_eq`] -- compiles to a 4 x u64 word
/// compare instead of a bytewise loop. `PartialOrd`/`Ord` stay
/// derived: word-equality and byte-equality decide the same pairs
/// equal, so the derived ordering remains consistent with the manual
/// equality.
#[repr(transparent)]
#[cfg_attr(feature = "copy", derive(Copy))]
#[derive(Clone, Default, Ord, PartialOrd)]
pub struct Address(pub(crate) [u8; 32]);

impl PartialEq for Address {
    /// Word-compare equality: delegates to [`address_eq`] so the
    /// `==` operator is exactly as fast as the free function.
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        address_eq(self, other)
    }
}

// Word-equality is an equivalence relation: it decides equal exactly
// when all 32 bytes match, same as the previously-derived impl.
impl Eq for Address {}

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

/// Address equality over raw bytes: 4 x u64 word comparison.
///
/// Short-circuits on the first differing 8-byte chunk. This is the
/// single equality body behind every backend key check (owner checks,
/// CPI account validation, instruction introspection, PDA bump
/// search), so it must stay branch-light and inlinable.
#[inline(always)]
pub fn address_eq(a: &Address, b: &Address) -> bool {
    let a_ptr = a.0.as_ptr() as *const u64;
    let b_ptr = b.0.as_ptr() as *const u64;
    // SAFETY: Address is #[repr(transparent)] over [u8; 32] = 4 x u64,
    // so all four reads on each side are in bounds. Use unaligned reads
    // because Address is only byte-aligned.
    unsafe {
        core::ptr::read_unaligned(a_ptr) == core::ptr::read_unaligned(b_ptr)
            && core::ptr::read_unaligned(a_ptr.add(1)) == core::ptr::read_unaligned(b_ptr.add(1))
            && core::ptr::read_unaligned(a_ptr.add(2)) == core::ptr::read_unaligned(b_ptr.add(2))
            && core::ptr::read_unaligned(a_ptr.add(3)) == core::ptr::read_unaligned(b_ptr.add(3))
    }
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

    /// Edge patterns exercised by the equality tests below.
    fn edge_patterns() -> [[u8; 32]; 6] {
        let mut ramp = [0u8; 32];
        let mut i = 0;
        while i < 32 {
            ramp[i] = i as u8;
            i += 1;
        }
        let mut last_hi = [0u8; 32];
        last_hi[31] = 0xFF;
        let mut first_hi = [0u8; 32];
        first_hi[0] = 0xFF;
        [
            [0u8; 32],
            [0xFFu8; 32],
            ramp,
            last_hi,
            first_hi,
            [0xA5u8; 32],
        ]
    }

    #[test]
    fn address_eq_matches_bytewise_on_equal_arrays() {
        for pat in edge_patterns() {
            let a = Address::new_from_array(pat);
            let b = Address::new_from_array(pat);
            assert!(address_eq(&a, &b));
            assert_eq!(a, b);
        }
    }

    #[test]
    fn address_eq_detects_single_byte_difference_at_every_index() {
        for base in edge_patterns() {
            for idx in 0..32 {
                let mut other = base;
                other[idx] ^= 0x01;
                let a = Address::new_from_array(base);
                let b = Address::new_from_array(other);
                assert!(!address_eq(&a, &b), "missed diff at byte {idx}");
                assert_ne!(a, b);
                // Word compare must agree with bytewise compare.
                assert_eq!(address_eq(&a, &b), base == other);
            }
        }
    }

    #[test]
    fn address_eq_differs_only_in_last_byte() {
        let base = [7u8; 32];
        let mut other = base;
        other[31] = 8;
        let a = Address::new_from_array(base);
        let b = Address::new_from_array(other);
        assert!(!address_eq(&a, &b));
        assert_ne!(a, b);
    }

    #[test]
    fn eq_is_consistent_with_derived_ord() {
        use core::cmp::Ordering;
        let patterns = edge_patterns();
        for a in patterns {
            for b in patterns {
                let aa = Address::new_from_array(a);
                let ab = Address::new_from_array(b);
                // Manual PartialEq must agree with derived Ord.
                assert_eq!(aa == ab, aa.cmp(&ab) == Ordering::Equal);
                // ...and with bytewise equality on the raw arrays.
                assert_eq!(aa == ab, a == b);
                for idx in 0..32 {
                    let mut c = a;
                    c[idx] = c[idx].wrapping_add(1);
                    let ac = Address::new_from_array(c);
                    assert_eq!(aa == ac, aa.cmp(&ac) == Ordering::Equal);
                    assert_eq!(aa == ac, a == c);
                }
            }
        }
    }
}
