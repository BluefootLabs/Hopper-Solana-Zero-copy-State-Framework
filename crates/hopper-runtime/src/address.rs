//! Hopper-owned address type for Solana programs.
//!
//! `Address` is a 32-byte public key with `#[repr(transparent)]` layout
//! over `[u8; 32]`. Hopper owns the canonical public-key type across the
//! runtime.

// ── Constants ────────────────────────────────────────────────────────

/// Number of bytes in an address.
pub const ADDRESS_BYTES: usize = 32;

/// Maximum length of a single PDA seed.
pub const MAX_SEED_LEN: usize = 32;

/// Maximum number of seeds for PDA derivation.
pub const MAX_SEEDS: usize = 16;

/// Marker appended to PDA hash inputs: `"ProgramDerivedAddress"`.
pub const PDA_MARKER: &[u8; 21] = b"ProgramDerivedAddress";

// ── Address ──────────────────────────────────────────────────────────

/// A Solana address (public key): 32 bytes, transparent layout.
///
/// This is part of the Hopper runtime type surface.
///
/// `PartialEq`/`Eq` are implemented manually (see below) so every
/// `Address == Address` in the runtime and in user programs compiles
/// to the 4 x u64 word compare in [`address_eq`] rather than a
/// bytewise loop. `PartialOrd`/`Ord` stay derived: word-equality and
/// byte-equality decide the same pairs equal, so the derived ordering
/// remains consistent with the manual equality.
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialOrd, Ord)]
pub struct Address(pub(crate) [u8; 32]);

// SAFETY: `Address` is `#[repr(transparent)]` over `[u8; 32]`, so it
// inherits the POD contract of its inner type exactly:
// - Every byte pattern is valid (no niches).
// - Alignment is 1 (inherits `[u8; 32]`'s alignment).
// - No padding, no drop glue, no interior pointers.
unsafe impl crate::pod::Zeroable for Address {}
unsafe impl crate::pod::Pod for Address {}
// Audit Step 5 seal: Hopper-authored primitive.
unsafe impl crate::zerocopy::__sealed::HopperZeroCopySealed for Address {}

impl Address {
    /// Construct from a raw byte array.
    #[inline(always)]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Construct from a raw byte array (alias for compatibility).
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

    /// Borrow the underlying bytes.
    #[inline(always)]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Find a program-derived address and its bump seed.
    ///
    /// Iterates bump values from 255 to 0, returning the first valid PDA.
    /// Only available on-chain (`target_os = "solana"`).
    #[cfg(target_os = "solana")]
    pub fn find_program_address(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
        crate::native_boundary::find_program_address(seeds, program_id)
    }

    /// Create a program-derived address from seeds.
    ///
    /// This is the cheaper PDA path when the bump is already known.
    #[cfg(target_os = "solana")]
    pub fn create_program_address(
        seeds: &[&[u8]],
        program_id: &Address,
    ) -> Result<Address, crate::ProgramError> {
        crate::native_boundary::create_program_address(seeds, program_id)
    }
}

// ── Trait implementations ────────────────────────────────────────────

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

impl core::fmt::Display for Address {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Hex-encoded short form for no_std Display
        for byte in &self.0[..4] {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "..")
    }
}

// ── Fast equality ────────────────────────────────────────────────────

/// Fast address equality using 4 x u64 comparison.
#[inline(always)]
pub fn address_eq(a: &Address, b: &Address) -> bool {
    keys_eq(&a.0, &b.0)
}

/// Fast 32-byte key equality using 4 x u64 word comparison.
///
/// Short-circuits on the first differing 8-byte chunk. Equivalent to
/// `a == b` on the arrays but avoids the bytewise loop; this is the
/// single word-compare body every key check in the runtime routes
/// through ([`address_eq`], `Address == Address`, the
/// `require_keys_eq!` / `require_keys_neq!` macros, and the token
/// precondition helpers).
#[inline(always)]
pub fn keys_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let a_ptr = a.as_ptr() as *const u64;
    let b_ptr = b.as_ptr() as *const u64;
    // SAFETY: Both inputs are [u8; 32] = 4 x u64, so all four reads are
    // in bounds. Use unaligned reads because [u8; 32] is only
    // byte-aligned.
    unsafe {
        core::ptr::read_unaligned(a_ptr) == core::ptr::read_unaligned(b_ptr)
            && core::ptr::read_unaligned(a_ptr.add(1)) == core::ptr::read_unaligned(b_ptr.add(1))
            && core::ptr::read_unaligned(a_ptr.add(2)) == core::ptr::read_unaligned(b_ptr.add(2))
            && core::ptr::read_unaligned(a_ptr.add(3)) == core::ptr::read_unaligned(b_ptr.add(3))
    }
}

/// Fast key equality between an arbitrary byte slice and a 32-byte key.
///
/// Returns `false` unless `a.len() == 32`; otherwise performs the same
/// 4 x u64 word comparison as [`keys_eq`]. This serves call sites that
/// hold a slice view into account data (e.g. an SPL token account's
/// `owner` field at `data[32..64]`) and want to compare against an
/// expected key without first copying 32 bytes into a temporary array.
#[inline(always)]
pub fn keys_eq_bytes(a: &[u8], b: &[u8; 32]) -> bool {
    if a.len() != 32 {
        return false;
    }
    let a_ptr = a.as_ptr() as *const u64;
    let b_ptr = b.as_ptr() as *const u64;
    // SAFETY: `a.len() == 32` was checked above and `b` is [u8; 32], so
    // all four 8-byte reads on each side are in bounds. Use unaligned
    // reads because both buffers are only byte-aligned.
    unsafe {
        core::ptr::read_unaligned(a_ptr) == core::ptr::read_unaligned(b_ptr)
            && core::ptr::read_unaligned(a_ptr.add(1)) == core::ptr::read_unaligned(b_ptr.add(1))
            && core::ptr::read_unaligned(a_ptr.add(2)) == core::ptr::read_unaligned(b_ptr.add(2))
            && core::ptr::read_unaligned(a_ptr.add(3)) == core::ptr::read_unaligned(b_ptr.add(3))
    }
}

/// Fast is-zero check: OR-fold the address's 4 u64 words.
///
/// Cheaper than comparing against an all-zero constant because only one
/// operand is loaded. Useful for system-program / default-address
/// checks (the system program id is the all-zero address).
#[inline(always)]
pub fn address_is_zero(a: &Address) -> bool {
    let ptr = a.0.as_ptr() as *const u64;
    // SAFETY: Address is 32 bytes = 4 x u64, so all four reads are in
    // bounds. Use unaligned reads because Address is only byte-aligned.
    unsafe {
        (core::ptr::read_unaligned(ptr)
            | core::ptr::read_unaligned(ptr.add(1))
            | core::ptr::read_unaligned(ptr.add(2))
            | core::ptr::read_unaligned(ptr.add(3)))
            == 0
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Edge patterns exercised by every equality test below.
    fn edge_patterns() -> [[u8; 32]; 6] {
        let mut ramp = [0u8; 32];
        for (i, b) in ramp.iter_mut().enumerate() {
            *b = i as u8;
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
    fn keys_eq_matches_bytewise_on_equal_arrays() {
        for pat in edge_patterns() {
            let copy = pat;
            assert!(keys_eq(&pat, &copy));
            assert!(address_eq(&Address::new(pat), &Address::new(pat)));
            assert_eq!(Address::new(pat), Address::new(pat));
        }
    }

    #[test]
    fn keys_eq_detects_single_byte_difference_at_every_index() {
        for base in edge_patterns() {
            for idx in 0..32 {
                let mut other = base;
                other[idx] ^= 0x01;
                assert!(!keys_eq(&base, &other), "missed diff at byte {idx}");
                assert!(!address_eq(&Address::new(base), &Address::new(other)));
                assert_ne!(Address::new(base), Address::new(other));
                // Word compare must agree with bytewise compare.
                assert_eq!(keys_eq(&base, &other), base == other);
            }
        }
    }

    #[test]
    fn keys_eq_differs_only_in_last_byte() {
        let a = [7u8; 32];
        let mut b = a;
        b[31] = 8;
        assert!(!keys_eq(&a, &b));
        assert_ne!(Address::new(a), Address::new(b));
    }

    #[test]
    fn keys_eq_bytes_matches_slice_semantics() {
        for pat in edge_patterns() {
            assert!(keys_eq_bytes(&pat[..], &pat));
            for idx in 0..32 {
                let mut other = pat;
                other[idx] ^= 0x80;
                assert_eq!(keys_eq_bytes(&other[..], &pat), other == pat);
            }
        }
        // Wrong-length slices never compare equal.
        let key = [0u8; 32];
        assert!(!keys_eq_bytes(&[], &key));
        assert!(!keys_eq_bytes(&key[..31], &key));
        let long = [0u8; 33];
        assert!(!keys_eq_bytes(&long[..], &key));
    }

    #[test]
    fn eq_is_consistent_with_derived_ord() {
        use core::cmp::Ordering;
        let patterns = edge_patterns();
        for a in patterns {
            for b in patterns {
                let (aa, ab) = (Address::new(a), Address::new(b));
                // Manual PartialEq must agree with derived Ord.
                assert_eq!(aa == ab, aa.cmp(&ab) == Ordering::Equal);
                // ...and with bytewise equality on the raw arrays.
                assert_eq!(aa == ab, a == b);
                for idx in 0..32 {
                    let mut c = a;
                    c[idx] = c[idx].wrapping_add(1);
                    let ac = Address::new(c);
                    assert_eq!(aa == ac, aa.cmp(&ac) == Ordering::Equal);
                    assert_eq!(aa == ac, a == c);
                }
            }
        }
    }

    #[test]
    fn address_is_zero_or_fold() {
        assert!(address_is_zero(&Address::new([0u8; 32])));
        assert!(address_is_zero(&Address::default()));
        for idx in 0..32 {
            let mut bytes = [0u8; 32];
            bytes[idx] = 1;
            assert!(!address_is_zero(&Address::new(bytes)));
        }
        assert!(!address_is_zero(&Address::new([0xFFu8; 32])));
    }
}
