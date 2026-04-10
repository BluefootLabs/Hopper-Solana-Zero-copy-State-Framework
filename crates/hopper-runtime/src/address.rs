//! Hopper-owned address type for Solana programs.
//!
//! `Address` is a 32-byte public key with `#[repr(transparent)]` layout
//! over `[u8; 32]`. This enables zero-cost reference casting from any
//! backend address type that shares the same representation.
//!
//! Hopper owns the canonical type. Backend-specific conversions live in
//! compatibility modules so the core identity remains framework-owned.

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
/// This is part of the Hopper runtime type surface. Backends convert
/// to and from this type at system boundaries.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub struct Address(pub(crate) [u8; 32]);

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
        crate::compat::find_program_address(seeds, program_id)
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
    let a_ptr = a.0.as_ptr() as *const u64;
    let b_ptr = b.0.as_ptr() as *const u64;
    // SAFETY: Address is 32 bytes = 4 x u64. Use unaligned reads because
    // Address itself is only byte-aligned.
    unsafe {
        core::ptr::read_unaligned(a_ptr) == core::ptr::read_unaligned(b_ptr)
            && core::ptr::read_unaligned(a_ptr.add(1)) == core::ptr::read_unaligned(b_ptr.add(1))
            && core::ptr::read_unaligned(a_ptr.add(2)) == core::ptr::read_unaligned(b_ptr.add(2))
            && core::ptr::read_unaligned(a_ptr.add(3)) == core::ptr::read_unaligned(b_ptr.add(3))
    }
}
