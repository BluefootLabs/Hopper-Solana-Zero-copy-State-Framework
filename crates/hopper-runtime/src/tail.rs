//! Hybrid serialization tail for `#[hopper::state(dynamic_tail = T)]`.
//!
//! Closes Hopper Safety Audit innovation I5 ("Hybrid serialization").
//! The rationale from the audit (page 14):
//!
//! > Lets Hopper own the fixed-layout hot path while still supporting a
//! > dynamic tail for vectors, strings, and optional metadata.
//!
//! # Wire format
//!
//! After the layout's fixed body (offset `TYPE_OFFSET + WIRE_SIZE`), the
//! tail is encoded as:
//!
//! ```text
//! [ len: u32 LE ] [ payload: len bytes ]
//! ```
//!
//! The fixed-body fast path remains fully zero-copy. code that never
//! touches the tail pays zero overhead. Tail access is explicit
//! (`tail_read::<T>()` / `tail_write::<T>()`), which is why the tail
//! is **not** zero-copy: the typed representation is reconstructed on
//! read and serialized on write.
//!
//! # Canonical tail encoding (`TailCodec`)
//!
//! `TailCodec` is a minimal Borsh-subset serializer:
//!
//! * integers: native little-endian
//! * `[u8; N]`: raw bytes, fixed width
//! * bounded byte/string payloads: program-defined length prefix + bytes
//! * `Option<T>`: 1-byte tag (0 = None, 1 = Some) + inner payload
//!
//! Programs that need richer types (bounded strings, bounded vectors,
//! custom structs) implement `TailCodec` themselves; the framework does not
//! force a derive or pull `Vec` / `String` into the no-alloc runtime surface.

use crate::error::ProgramError;

/// Canonical serializer for dynamic-tail payloads.
///
/// Implementations encode into a caller-provided buffer and decode
/// from a caller-provided slice, returning the byte count consumed
/// in both directions. Byte counts drive the length-prefix handling
/// inside `#[hopper::state]`'s generated tail accessors. the
/// encoding must be deterministic and bidirectional.
pub trait TailCodec: Sized {
    /// Upper bound on the encoded size. Used by generated helpers to
    /// verify the account has enough room before invoking `encode`.
    /// Implementors should pick the smallest valid bound. Hopper
    /// uses this to pre-size reallocs.
    const MAX_ENCODED_LEN: usize;

    /// Serialize `self` into `out`. Returns the number of bytes
    /// written (always `<= MAX_ENCODED_LEN`). Fails with
    /// `AccountDataTooSmall` when `out.len() < encoded_len`.
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError>;

    /// Deserialize from `input`. Returns `(value, bytes_consumed)`.
    /// Fails with `InvalidAccountData` on malformed encoding.
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError>;
}

/// Element type accepted by `#[tail(vec<T, N>)]` in `#[hopper::dynamic_account]`.
///
/// A tail element must have deterministic Hopper tail encoding, be cheap to copy
/// into the fixed-capacity backing array, have a default empty slot value, and be
/// comparable for generated `push_unique_*` / `remove_*` helpers.
pub trait TailElement: TailCodec + Copy + Default + PartialEq {}

impl<T> TailElement for T where T: TailCodec + Copy + Default + PartialEq {}

/// Borrowed final raw-byte tail.
///
/// `TailBytes<'a>` is intentionally not a `TailCodec`: it is a borrowed view
/// over the bytes remaining after any bounded compact-tail fields. The owning
/// account data provides the storage.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct TailBytes<'a> {
    bytes: &'a [u8],
}

impl<'a> TailBytes<'a> {
    /// Borrow `bytes` as a final raw tail.
    #[inline(always)]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Return the raw tail bytes.
    #[inline(always)]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Number of raw tail bytes.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the raw tail is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Borrowed final UTF-8 tail.
///
/// The raw bytes consume the remaining dynamic-tail payload. UTF-8 is checked
/// only when the caller asks for `&str`, so binary-safe inspection and strict
/// text validation are both available without copying.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct TailStr<'a> {
    bytes: &'a [u8],
}

impl<'a> TailStr<'a> {
    /// Borrow `bytes` as a final UTF-8 tail.
    #[inline(always)]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Borrow a string as a final UTF-8 tail.
    #[inline(always)]
    pub const fn from_str(value: &'a str) -> Self {
        Self {
            bytes: value.as_bytes(),
        }
    }

    /// Return the raw UTF-8 bytes without validating again.
    #[inline(always)]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Validate and return the tail as UTF-8.
    #[inline]
    pub fn as_str(&self) -> Result<&'a str, ProgramError> {
        core::str::from_utf8(self.bytes).map_err(|_| ProgramError::InvalidAccountData)
    }

    /// Number of raw UTF-8 bytes.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the raw tail is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

// ── Primitive impls (little-endian, fixed width) ────────────────────

impl TailCodec for u8 {
    const MAX_ENCODED_LEN: usize = 1;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.is_empty() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[0] = *self;
        Ok(1)
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        input
            .first()
            .copied()
            .map(|b| (b, 1))
            .ok_or(ProgramError::InvalidAccountData)
    }
}

macro_rules! tail_codec_int {
    ( $( $ty:ty : $n:expr ),+ $(,)? ) => {
        $(
            impl TailCodec for $ty {
                const MAX_ENCODED_LEN: usize = $n;
                #[inline]
                fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
                    if out.len() < $n {
                        return Err(ProgramError::AccountDataTooSmall);
                    }
                    out[..$n].copy_from_slice(&self.to_le_bytes());
                    Ok($n)
                }
                #[inline]
                fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
                    if input.len() < $n {
                        return Err(ProgramError::InvalidAccountData);
                    }
                    let mut bytes = [0u8; $n];
                    bytes.copy_from_slice(&input[..$n]);
                    Ok((Self::from_le_bytes(bytes), $n))
                }
            }
        )+
    };
}

tail_codec_int! {
    u16: 2, u32: 4, u64: 8, u128: 16,
    i16: 2, i32: 4, i64: 8, i128: 16,
}

// `bool` as 1 byte (0 = false, 1 = true; anything else rejected).
impl TailCodec for bool {
    const MAX_ENCODED_LEN: usize = 1;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.is_empty() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[0] = if *self { 1 } else { 0 };
        Ok(1)
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        match input.first().copied() {
            Some(0) => Ok((false, 1)),
            Some(1) => Ok((true, 1)),
            _ => Err(ProgramError::InvalidAccountData),
        }
    }
}

// `[u8; N]`. raw fixed-width bytes.
impl<const N: usize> TailCodec for [u8; N] {
    const MAX_ENCODED_LEN: usize = N;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.len() < N {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[..N].copy_from_slice(self);
        Ok(N)
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        if input.len() < N {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&input[..N]);
        Ok((out, N))
    }
}

// `Option<T>`. 1-byte tag + inner payload when present.
impl<T: TailCodec> TailCodec for Option<T> {
    const MAX_ENCODED_LEN: usize = 1 + T::MAX_ENCODED_LEN;
    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.is_empty() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        match self {
            None => {
                out[0] = 0;
                Ok(1)
            }
            Some(inner) => {
                out[0] = 1;
                let written = inner.encode(&mut out[1..])?;
                Ok(1 + written)
            }
        }
    }
    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        match input.first().copied() {
            Some(0) => Ok((None, 1)),
            Some(1) => {
                let (inner, n) = T::decode(&input[1..])?;
                Ok((Some(inner), 1 + n))
            }
            _ => Err(ProgramError::InvalidAccountData),
        }
    }
}

// -- Bounded dynamic-tail helpers ------------------------------------------

/// Bounded UTF-8 string for Hopper dynamic tails.
///
/// This is the common migration target for bounded string account metadata.
/// It keeps a fixed `[u8; N]` backing buffer, carries a small length prefix on
/// the tail wire, and validates UTF-8 when read as `&str`.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BoundedString<const N: usize> {
    len: u16,
    bytes: [u8; N],
}

impl<const N: usize> BoundedString<N> {
    /// Construct an empty bounded string.
    #[inline]
    pub const fn empty() -> Self {
        Self {
            len: 0,
            bytes: [0u8; N],
        }
    }

    /// Construct from UTF-8 bytes, rejecting values longer than `N`.
    #[inline]
    pub fn from_str(value: &str) -> Result<Self, ProgramError> {
        Self::from_bytes(value.as_bytes())
    }

    /// Construct from bytes, rejecting values longer than `N`.
    #[inline]
    pub fn from_bytes(value: &[u8]) -> Result<Self, ProgramError> {
        if value.len() > N || value.len() > u16::MAX as usize {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut out = Self::empty();
        out.bytes[..value.len()].copy_from_slice(value);
        out.len = value.len() as u16;
        Ok(out)
    }

    /// Replace the contents in place.
    #[inline]
    pub fn set_str(&mut self, value: &str) -> Result<(), ProgramError> {
        self.set_bytes(value.as_bytes())
    }

    /// Replace the contents in place.
    #[inline]
    pub fn set_bytes(&mut self, value: &[u8]) -> Result<(), ProgramError> {
        if value.len() > N || value.len() > u16::MAX as usize {
            return Err(ProgramError::InvalidInstructionData);
        }
        self.bytes = [0u8; N];
        self.bytes[..value.len()].copy_from_slice(value);
        self.len = value.len() as u16;
        Ok(())
    }

    /// Clear the string without changing its capacity.
    #[inline]
    pub fn clear(&mut self) {
        self.bytes = [0u8; N];
        self.len = 0;
    }

    /// Return the initialized bytes.
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// Return the initialized bytes as UTF-8.
    #[inline]
    pub fn as_str(&self) -> Result<&str, ProgramError> {
        core::str::from_utf8(self.as_bytes()).map_err(|_| ProgramError::InvalidAccountData)
    }

    /// Number of initialized bytes.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Maximum byte capacity.
    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Remaining byte capacity.
    #[inline(always)]
    pub const fn remaining_capacity(&self) -> usize {
        N - self.len as usize
    }

    /// Whether the string has reached its maximum byte capacity.
    #[inline(always)]
    pub const fn is_full(&self) -> bool {
        self.len as usize == N
    }

    /// Whether the string is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for BoundedString<N> {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<const N: usize> TailCodec for BoundedString<N> {
    const MAX_ENCODED_LEN: usize = 2 + N;

    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        let len = self.len as usize;
        if len > N || out.len() < 2 + len {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[..2].copy_from_slice(&self.len.to_le_bytes());
        out[2..2 + len].copy_from_slice(&self.bytes[..len]);
        Ok(2 + len)
    }

    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        if input.len() < 2 {
            return Err(ProgramError::InvalidAccountData);
        }
        let len = u16::from_le_bytes([input[0], input[1]]) as usize;
        if len > N || input.len() < 2 + len {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut out = Self::empty();
        out.len = len as u16;
        out.bytes[..len].copy_from_slice(&input[2..2 + len]);
        Ok((out, 2 + len))
    }
}

/// Bounded dynamic vector for Hopper dynamic tails.
///
/// This is the common migration target for bounded vector account metadata.
/// Elements use `TailCodec`, so the vector can carry wire integers, addresses,
/// or small custom structs declared with `hopper_dynamic_tail!`.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BoundedVec<T, const N: usize>
where
    T: TailCodec + Copy + Default,
{
    len: u16,
    items: [T; N],
}

impl<T, const N: usize> BoundedVec<T, N>
where
    T: TailCodec + Copy + Default,
{
    /// Construct an empty bounded vector.
    #[inline]
    pub fn empty() -> Self {
        Self {
            len: 0,
            items: [T::default(); N],
        }
    }

    /// Construct from a slice, rejecting values longer than `N`.
    #[inline]
    pub fn from_slice(values: &[T]) -> Result<Self, ProgramError> {
        if values.len() > N || values.len() > u16::MAX as usize {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut out = Self::empty();
        out.items[..values.len()].copy_from_slice(values);
        out.len = values.len() as u16;
        Ok(out)
    }

    /// Push one item into the bounded vector.
    #[inline]
    pub fn push(&mut self, item: T) -> Result<(), ProgramError> {
        let len = self.len as usize;
        if len >= N || len >= u16::MAX as usize {
            return Err(ProgramError::AccountDataTooSmall);
        }
        self.items[len] = item;
        self.len += 1;
        Ok(())
    }

    /// Pop the last initialized item, if present.
    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        let len = self.len as usize;
        if len == 0 {
            return None;
        }
        let new_len = len - 1;
        let item = self.items[new_len];
        self.items[new_len] = T::default();
        self.len = new_len as u16;
        Some(item)
    }

    /// Clear all initialized items without changing capacity.
    #[inline]
    pub fn clear(&mut self) {
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            self.items[i] = T::default();
            i += 1;
        }
        self.len = 0;
    }

    /// Return the initialized items.
    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len as usize]
    }

    /// Return the initialized items mutably.
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.items[..self.len as usize]
    }

    /// Number of initialized items.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Maximum number of items.
    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Remaining element capacity.
    #[inline(always)]
    pub const fn remaining_capacity(&self) -> usize {
        N - self.len as usize
    }

    /// Whether the vector has reached its maximum element capacity.
    #[inline(always)]
    pub const fn is_full(&self) -> bool {
        self.len as usize == N
    }

    /// Whether the vector is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<T, const N: usize> BoundedVec<T, N>
where
    T: TailCodec + Copy + Default + PartialEq,
{
    /// Return true when the initialized items contain `item`.
    #[inline]
    pub fn contains(&self, item: &T) -> bool {
        self.as_slice().iter().any(|candidate| candidate == item)
    }

    /// Push `item` only if it is not already initialized.
    ///
    /// Returns `Ok(true)` when an item was inserted and `Ok(false)` when it
    /// was already present.
    #[inline]
    pub fn push_unique(&mut self, item: T) -> Result<bool, ProgramError> {
        if self.contains(&item) {
            return Ok(false);
        }
        self.push(item)?;
        Ok(true)
    }

    /// Remove the first matching item, preserving order.
    ///
    /// Returns `true` when an item was removed.
    #[inline]
    pub fn remove_first(&mut self, item: &T) -> bool {
        let len = self.len as usize;
        let mut found = None;
        let mut i = 0;
        while i < len {
            if &self.items[i] == item {
                found = Some(i);
                break;
            }
            i += 1;
        }
        let Some(index) = found else {
            return false;
        };
        let mut j = index;
        while j + 1 < len {
            self.items[j] = self.items[j + 1];
            j += 1;
        }
        self.items[len - 1] = T::default();
        self.len = (len - 1) as u16;
        true
    }
}

/// Short alias for bounded UTF-8 strings in dynamic tails.
pub type HopperString<const N: usize> = BoundedString<N>;

/// Short alias for bounded vectors in dynamic tails.
pub type HopperVec<T, const N: usize> = BoundedVec<T, N>;

impl<T, const N: usize> Default for BoundedVec<T, N>
where
    T: TailCodec + Copy + Default,
{
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<T, const N: usize> TailCodec for BoundedVec<T, N>
where
    T: TailCodec + Copy + Default,
{
    const MAX_ENCODED_LEN: usize = 2 + (N * T::MAX_ENCODED_LEN);

    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        let len = self.len as usize;
        if len > N || out.len() < 2 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[..2].copy_from_slice(&self.len.to_le_bytes());
        let mut cursor = 2;
        for item in self.as_slice() {
            let written = item.encode(&mut out[cursor..])?;
            cursor = cursor
                .checked_add(written)
                .ok_or(ProgramError::AccountDataTooSmall)?;
        }
        Ok(cursor)
    }

    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        if input.len() < 2 {
            return Err(ProgramError::InvalidAccountData);
        }
        let len = u16::from_le_bytes([input[0], input[1]]) as usize;
        if len > N {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut out = Self::empty();
        let mut cursor = 2;
        let mut i = 0;
        while i < len {
            let (item, consumed) = T::decode(&input[cursor..])?;
            out.items[i] = item;
            cursor = cursor
                .checked_add(consumed)
                .ok_or(ProgramError::InvalidAccountData)?;
            i += 1;
        }
        out.len = len as u16;
        Ok((out, cursor))
    }
}

impl TailCodec for crate::address::Address {
    const MAX_ENCODED_LEN: usize = 32;

    #[inline]
    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        if out.len() < 32 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[..32].copy_from_slice(self.as_array());
        Ok(32)
    }

    #[inline]
    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        if input.len() < 32 {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&input[..32]);
        Ok((crate::address::Address::new(bytes), 32))
    }
}

// ── Framework helpers used by `#[hopper::state(dynamic_tail = T)]` ──

/// Read the tail's u32-LE length prefix.
///
/// `body_end` is the byte offset immediately after the layout's fixed
/// body (i.e. `TYPE_OFFSET + WIRE_SIZE` for a layout with no header
/// beyond the 16-byte Hopper prefix, otherwise `HEADER_LEN +
/// WIRE_SIZE`). Returns `AccountDataTooSmall` if the account has
/// fewer than 4 tail bytes available.
#[inline]
pub fn read_tail_len(data: &[u8], body_end: usize) -> Result<u32, ProgramError> {
    let end = body_end
        .checked_add(4)
        .ok_or(ProgramError::AccountDataTooSmall)?;
    if data.len() < end {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[body_end..end]);
    Ok(u32::from_le_bytes(bytes))
}

/// Return a slice referencing just the tail payload bytes (excluding
/// the 4-byte length prefix). Length-bounded by the u32 prefix.
#[inline]
pub fn tail_payload(data: &[u8], body_end: usize) -> Result<&[u8], ProgramError> {
    let len = read_tail_len(data, body_end)? as usize;
    let start = body_end + 4;
    let end = start
        .checked_add(len)
        .ok_or(ProgramError::InvalidAccountData)?;
    if data.len() < end {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(&data[start..end])
}

/// Return the account bytes available after the tail length prefix.
///
/// This is useful before a grow/realloc path: if the encoded payload to write
/// is larger than this value, the caller must resize the account before
/// calling `write_tail`.
#[inline]
pub fn tail_capacity(data: &[u8], body_end: usize) -> Result<usize, ProgramError> {
    let start = body_end
        .checked_add(4)
        .ok_or(ProgramError::AccountDataTooSmall)?;
    if data.len() < start {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(data.len() - start)
}

/// Borrow one bounded UTF-8 string from a compact dynamic-tail payload.
///
/// The returned `usize` is the number of bytes consumed from `input`, so a
/// generated view can walk subsequent compact-tail fields without decoding the
/// whole tail into an owned value.
#[inline]
pub fn borrow_bounded_str<const N: usize>(input: &[u8]) -> Result<(&str, usize), ProgramError> {
    if input.len() < 2 {
        return Err(ProgramError::InvalidAccountData);
    }
    let len = u16::from_le_bytes([input[0], input[1]]) as usize;
    if len > N || input.len() < 2 + len {
        return Err(ProgramError::InvalidAccountData);
    }
    let bytes = &input[2..2 + len];
    let value = core::str::from_utf8(bytes).map_err(|_| ProgramError::InvalidAccountData)?;
    Ok((value, 2 + len))
}

/// Borrow one bounded address vector from a compact dynamic-tail payload.
///
/// This is the zero-copy read path for the common multisig/authority-list case
/// that Quasar represents as `Vec<'a, Address, N>`. `Address` is
/// `repr(transparent)` over `[u8; 32]` and alignment-1, so the slice cast is
/// layout-safe after the length and capacity checks below.
#[inline]
pub fn borrow_address_slice<const N: usize>(
    input: &[u8],
) -> Result<(&[crate::address::Address], usize), ProgramError> {
    if input.len() < 2 {
        return Err(ProgramError::InvalidAccountData);
    }
    let len = u16::from_le_bytes([input[0], input[1]]) as usize;
    if len > N {
        return Err(ProgramError::InvalidAccountData);
    }
    let byte_len = len
        .checked_mul(32)
        .ok_or(ProgramError::InvalidAccountData)?;
    let end = 2usize
        .checked_add(byte_len)
        .ok_or(ProgramError::InvalidAccountData)?;
    if input.len() < end {
        return Err(ProgramError::InvalidAccountData);
    }
    let bytes = &input[2..end];
    let ptr = bytes.as_ptr() as *const crate::address::Address;
    // SAFETY: Address has alignment 1 and is transparent over [u8; 32]. The
    // byte range length is exactly len * 32, checked above.
    let values = unsafe { core::slice::from_raw_parts(ptr, len) };
    Ok((values, end))
}

/// Decode the tail as `T: TailCodec`, checking that the encoded length
/// exactly matches the u32 prefix. Extra bytes beyond `T`'s decode
/// are a malformed-encoding signal.
#[inline]
pub fn read_tail<T: TailCodec>(data: &[u8], body_end: usize) -> Result<T, ProgramError> {
    let payload = tail_payload(data, body_end)?;
    let (value, consumed) = T::decode(payload)?;
    if consumed != payload.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(value)
}

/// Encode `tail` into the account's tail slot, rewriting the u32
/// length prefix. Returns `AccountDataTooSmall` when the existing
/// account byte buffer can't fit the encoded payload. in that case
/// the caller should `realloc` first.
#[inline]
pub fn write_tail<T: TailCodec>(
    data: &mut [u8],
    body_end: usize,
    tail: &T,
) -> Result<usize, ProgramError> {
    let prefix_end = body_end
        .checked_add(4)
        .ok_or(ProgramError::AccountDataTooSmall)?;
    if data.len() < prefix_end {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let written = tail.encode(&mut data[prefix_end..])?;
    if written > u32::MAX as usize {
        return Err(ProgramError::InvalidAccountData);
    }
    data[body_end..prefix_end].copy_from_slice(&(written as u32).to_le_bytes());
    Ok(written)
}

/// Write an already-encoded dynamic-tail payload.
///
/// This is used by generated bare-final-tail accounts: bounded fields are
/// encoded first, then `TailStr` / `TailBytes` contributes the remaining raw
/// payload without its own field-level length prefix.
#[inline]
pub fn write_tail_payload(
    data: &mut [u8],
    body_end: usize,
    payload: &[u8],
) -> Result<usize, ProgramError> {
    let prefix_end = body_end
        .checked_add(4)
        .ok_or(ProgramError::AccountDataTooSmall)?;
    let payload_end = prefix_end
        .checked_add(payload.len())
        .ok_or(ProgramError::AccountDataTooSmall)?;
    if data.len() < payload_end || payload.len() > u32::MAX as usize {
        return Err(ProgramError::AccountDataTooSmall);
    }
    data[prefix_end..payload_end].copy_from_slice(payload);
    data[body_end..prefix_end].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    Ok(payload.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u32_roundtrip() {
        let mut buf = [0u8; 8];
        let n = 0xDEAD_BEEFu32.encode(&mut buf).unwrap();
        assert_eq!(n, 4);
        let (back, consumed) = u32::decode(&buf).unwrap();
        assert_eq!(consumed, 4);
        assert_eq!(back, 0xDEAD_BEEF);
    }

    #[test]
    fn u64_roundtrip() {
        let mut buf = [0u8; 8];
        0x0123_4567_89AB_CDEFu64.encode(&mut buf).unwrap();
        let (back, _) = u64::decode(&buf).unwrap();
        assert_eq!(back, 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn bool_encode_decode() {
        let mut buf = [0u8; 1];
        true.encode(&mut buf).unwrap();
        assert_eq!(buf[0], 1);
        assert_eq!(bool::decode(&buf).unwrap(), (true, 1));
        false.encode(&mut buf).unwrap();
        assert_eq!(buf[0], 0);
        assert_eq!(bool::decode(&buf).unwrap(), (false, 1));
    }

    #[test]
    fn bool_rejects_garbage() {
        let buf = [2u8];
        assert!(bool::decode(&buf).is_err());
    }

    #[test]
    fn byte_array_roundtrip() {
        let src: [u8; 8] = *b"HOPPER!!";
        let mut buf = [0u8; 16];
        let n = src.encode(&mut buf).unwrap();
        assert_eq!(n, 8);
        let (back, consumed) = <[u8; 8]>::decode(&buf).unwrap();
        assert_eq!(consumed, 8);
        assert_eq!(back, src);
    }

    #[test]
    fn option_none_encodes_to_one_byte() {
        let mut buf = [0u8; 16];
        let n = Option::<u64>::None.encode(&mut buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0);
        let (back, c) = <Option<u64>>::decode(&buf).unwrap();
        assert_eq!(back, None);
        assert_eq!(c, 1);
    }

    #[test]
    fn option_some_includes_inner_payload() {
        let mut buf = [0u8; 16];
        let n = Option::<u64>::Some(0xAAAA_BBBB_CCCC_DDDD)
            .encode(&mut buf)
            .unwrap();
        assert_eq!(n, 9);
        assert_eq!(buf[0], 1);
        let (back, c) = <Option<u64>>::decode(&buf).unwrap();
        assert_eq!(back, Some(0xAAAA_BBBB_CCCC_DDDD));
        assert_eq!(c, 9);
    }

    #[test]
    fn option_rejects_invalid_tag() {
        let buf = [7u8, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(<Option<u64>>::decode(&buf).is_err());
    }

    #[test]
    fn tail_length_prefix_roundtrip() {
        // Simulate an account body: 16-byte "header" + 8-byte body +
        // 4-byte length prefix + tail bytes. body_end = 24.
        let mut data = [0u8; 64];
        let body_end = 24usize;
        let tail_value: u64 = 0x1234_5678_9ABC_DEF0;
        let written = write_tail(&mut data, body_end, &tail_value).unwrap();
        assert_eq!(written, 8);
        let read_len = read_tail_len(&data, body_end).unwrap();
        assert_eq!(read_len, 8);
        let back: u64 = read_tail::<u64>(&data, body_end).unwrap();
        assert_eq!(back, tail_value);
    }

    #[test]
    fn tail_decode_rejects_excess_payload() {
        // If the tail encodes as 4 bytes but the length prefix claims
        // 8, the decode must refuse rather than silently succeed.
        let mut data = [0u8; 32];
        // body_end = 16; prefix says 8 bytes; payload is u32 (4 bytes) +
        // garbage (4 bytes). Decoding as u32 leaves 4 bytes unconsumed
        // which is caught by `read_tail`.
        let body_end = 16usize;
        data[body_end..body_end + 4].copy_from_slice(&8u32.to_le_bytes());
        // Fill payload with something that decodes as u32=0x11223344
        // and then trailing garbage.
        data[body_end + 4..body_end + 8].copy_from_slice(&0x1122_3344u32.to_le_bytes());
        data[body_end + 8..body_end + 12].copy_from_slice(&0xFFu32.to_le_bytes());
        // u32 decodes 4 bytes but prefix claims 8. expect error.
        let result = read_tail::<u32>(&data, body_end);
        assert!(result.is_err());
    }

    #[test]
    fn tail_bounds_check_on_short_buffer() {
        let data = [0u8; 10];
        assert!(read_tail_len(&data, 16).is_err());
        assert!(tail_payload(&data, 16).is_err());
    }

    #[test]
    fn max_encoded_len_matches_actual_encode_size() {
        let mut buf = [0u8; 32];
        assert_eq!(0u32.encode(&mut buf).unwrap(), u32::MAX_ENCODED_LEN);
        assert_eq!(0u64.encode(&mut buf).unwrap(), u64::MAX_ENCODED_LEN);
        assert_eq!(true.encode(&mut buf).unwrap(), bool::MAX_ENCODED_LEN);
        assert_eq!(
            [0u8; 7].encode(&mut buf).unwrap(),
            <[u8; 7]>::MAX_ENCODED_LEN
        );
        assert_eq!(Option::<u32>::None.encode(&mut buf).unwrap(), 1);
        assert_eq!(
            Option::<u32>::Some(0).encode(&mut buf).unwrap(),
            <Option<u32>>::MAX_ENCODED_LEN
        );
    }

    #[test]
    fn bounded_string_roundtrip() {
        let label = BoundedString::<32>::from_str("multisig").unwrap();
        let mut buf = [0u8; BoundedString::<32>::MAX_ENCODED_LEN];
        let written = label.encode(&mut buf).unwrap();
        assert_eq!(written, 10);
        let (back, consumed) = BoundedString::<32>::decode(&buf).unwrap();
        assert_eq!(consumed, written);
        assert_eq!(back.as_str().unwrap(), "multisig");
    }

    #[test]
    fn bounded_string_capacity_helpers() {
        let mut label = HopperString::<8>::from_str("ops").unwrap();
        assert_eq!(label.remaining_capacity(), 5);
        assert!(!label.is_full());
        label.set_str("12345678").unwrap();
        assert!(label.is_full());
        label.clear();
        assert!(label.is_empty());
        assert_eq!(label.as_bytes(), b"");
    }

    #[test]
    fn bounded_vec_roundtrip() {
        let mut vec = BoundedVec::<u64, 4>::empty();
        vec.push(7).unwrap();
        vec.push(9).unwrap();
        let mut buf = [0u8; BoundedVec::<u64, 4>::MAX_ENCODED_LEN];
        let written = vec.encode(&mut buf).unwrap();
        assert_eq!(written, 18);
        let (back, consumed) = BoundedVec::<u64, 4>::decode(&buf).unwrap();
        assert_eq!(consumed, written);
        assert_eq!(back.as_slice(), &[7, 9]);
    }

    #[test]
    fn bounded_vec_set_helpers_preserve_order() {
        let mut vec = HopperVec::<u64, 4>::empty();
        assert_eq!(vec.remaining_capacity(), 4);
        assert_eq!(vec.push_unique(7).unwrap(), true);
        assert_eq!(vec.push_unique(7).unwrap(), false);
        vec.push(9).unwrap();
        vec.push(11).unwrap();
        assert!(vec.contains(&9));
        assert!(vec.remove_first(&9));
        assert_eq!(vec.as_slice(), &[7, 11]);
        assert_eq!(vec.pop(), Some(11));
        assert_eq!(vec.as_slice(), &[7]);
        vec.clear();
        assert!(vec.is_empty());
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn bounded_string_decode_never_exceeds_capacity() {
        let len: u16 = kani::any();
        let mut input = [0u8; BoundedString::<4>::MAX_ENCODED_LEN];
        input[..2].copy_from_slice(&len.to_le_bytes());

        let result = BoundedString::<4>::decode(&input);
        if len as usize > 4 {
            assert!(result.is_err());
        } else {
            let (decoded, consumed) = result.unwrap();
            assert!(decoded.len() <= decoded.capacity());
            assert_eq!(consumed, 2 + decoded.len());
        }
    }

    #[kani::proof]
    fn bounded_vec_mutators_preserve_capacity() {
        let values: [u8; 5] = kani::any();
        let mut vec = BoundedVec::<u8, 4>::empty();

        let _ = vec.push(values[0]);
        let _ = vec.push(values[1]);
        let _ = vec.push(values[2]);
        let _ = vec.push(values[3]);
        let fifth = vec.push(values[4]);

        assert!(vec.len() <= vec.capacity());
        assert!(fifth.is_err());
        let _ = vec.pop();
        assert!(vec.len() <= vec.capacity());
        vec.clear();
        assert_eq!(vec.len(), 0);
    }

    #[kani::proof]
    fn tail_payload_bounds_checks_arbitrary_prefixes() {
        let data: [u8; 16] = kani::any();
        let body_end: usize = kani::any();
        kani::assume(body_end < data.len());

        let result = tail_payload(&data, body_end);
        if let Ok(payload) = result {
            assert!(payload.len() <= data.len());
        }
    }
}
