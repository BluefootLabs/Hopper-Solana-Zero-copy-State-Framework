//! Tier 1 of the three-tier metadata model: compact account access.
//!
//! See [`docs/THREE_TIER_METADATA.md`](../../../docs/THREE_TIER_METADATA.md).
//!
//! A *compact* account stores exactly one discriminator byte followed by
//! the zero-copy body:
//!
//! ```text
//! byte 0   : disc (u8)
//! bytes 1..: zero-copy body (alignment-1 Pod fields)
//! ```
//!
//! There is **no 16-byte universal header**. The hot path is
//! `check_len_exact` + `check_disc` + cast-body-at-offset-1: no layout_id read,
//! no schema-epoch comparison, no registry fetch. Identity of the layout
//! behind a discriminator is a *program-level* fact (Tier 2 / Tier 3),
//! not a per-account one.
//!
//! This is additive: the 16-byte-header [`crate::layout::LayoutContract`]
//! path is unchanged and remains the default. A type opts into compact
//! by implementing [`CompactLayout`]; the two are distinguished by which
//! loader the caller invokes.

use crate::error::ProgramError;
use crate::ProgramResult;

/// Byte offset of a compact account body (immediately after the 1-byte
/// discriminator).
pub const COMPACT_BODY_OFFSET: usize = 1;

/// A zero-copy account layout stored in compact `[disc:u8][body]` form.
///
/// # Safety
///
/// The blanket access methods overlay `Self` directly on account bytes
/// starting at [`COMPACT_BODY_OFFSET`]. Implementing this trait asserts
/// the same contract as [`crate::Pod`] for the body: alignment 1, no
/// padding, every bit pattern valid, no internal pointers. The `Pod`
/// supertrait carries that obligation; `CompactLayout` only adds the
/// discriminator and the compact wire-length math.
pub trait CompactLayout: Sized + Copy + crate::Pod {
    /// Discriminator stored at byte 0 of the account.
    const DISC: u8;

    /// Body size in bytes (the zero-copy struct).
    const BODY_SIZE: usize = core::mem::size_of::<Self>();

    /// Total compact wire length: 1 discriminator byte + body.
    const COMPACT_LEN: usize = COMPACT_BODY_OFFSET + Self::BODY_SIZE;

    /// Validate that `data` is a compact account of this type.
    ///
    /// Checks the buffer has exactly the fixed compact wire length and
    /// the discriminator at byte 0 matches. Deliberately does **not**
    /// read a layout_id or epoch.
    #[inline(always)]
    fn validate_compact(data: &[u8]) -> ProgramResult {
        if data.len() < Self::COMPACT_LEN {
            return Err(ProgramError::AccountDataTooSmall);
        }
        if data.len() != Self::COMPACT_LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        if data[0] != Self::DISC {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }
}

/// A compact account with a fixed zero-copy head followed by a dynamic tail:
///
/// ```text
/// byte 0            : disc (u8)
/// bytes 1..1+H      : fixed head (this Pod struct), H = size_of::<Self>()
/// bytes 1+H..       : dynamic tail (u32 LE length prefix + payload)
/// ```
///
/// This is the 1-byte-header analogue of the headered `dynamic_tail` layout:
/// it gives an account a Quasar-class `[disc][fixed_head][tail]` shape with
/// **no** 16-byte universal header, while keeping Hopper's registry, schema,
/// and fingerprint tooling.
///
/// Unlike [`CompactLayout`], the wire length is **not** fixed — the tail may
/// be empty or grow via `resize`. The fixed head is still overlaid zero-copy
/// at [`COMPACT_BODY_OFFSET`]; the tail is read/written through the
/// macro-generated `tail_*` helpers, which operate on a length-prefixed
/// payload anchored at [`Self::TAIL_OFFSET`] (the same offset-parameterized
/// `read_tail`/`write_tail` runtime the headered path uses).
///
/// # Safety
///
/// As with [`CompactLayout`], the fixed head is overlaid directly on account
/// bytes starting at [`COMPACT_BODY_OFFSET`]; the `Pod` supertrait carries the
/// alignment-1 / no-padding / valid-for-all-bits obligation for that head.
/// The tail bytes beyond the head are never reinterpreted as `Self`.
pub trait CompactDynamicLayout: Sized + Copy + crate::Pod {
    /// Discriminator stored at byte 0 of the account.
    const DISC: u8;

    /// Fixed head size in bytes (the zero-copy struct), excluding the
    /// discriminator and the tail.
    const FIXED_HEAD_SIZE: usize = core::mem::size_of::<Self>();

    /// Minimum wire length: 1 discriminator byte + the fixed head. The tail
    /// may be empty, so this is the floor, not the exact length.
    const MIN_LEN: usize = COMPACT_BODY_OFFSET + Self::FIXED_HEAD_SIZE;

    /// Byte offset of the tail region (its `u32` LE length prefix),
    /// immediately after the fixed head. Equal to [`Self::MIN_LEN`].
    const TAIL_OFFSET: usize = Self::MIN_LEN;

    /// Validate that `data` is a compact-dynamic account of this type: it is
    /// at least [`MIN_LEN`](Self::MIN_LEN) bytes (discriminator + fixed head
    /// present) and the discriminator at byte 0 matches.
    ///
    /// Deliberately does **not** require an exact length: trailing tail bytes
    /// are expected. The tail's own length prefix and payload bounds are
    /// validated by the tail accessors when used.
    #[inline(always)]
    fn validate_compact_dynamic(data: &[u8]) -> ProgramResult {
        if data.len() < Self::MIN_LEN {
            return Err(ProgramError::AccountDataTooSmall);
        }
        if data[0] != Self::DISC {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pod::{Pod, Zeroable};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Body {
        authority: [u8; 32],
        balance: [u8; 8],
    }
    // SAFETY: alignment-1 byte arrays, all bit patterns valid, no padding.
    unsafe impl Zeroable for Body {}
    unsafe impl Pod for Body {}
    impl CompactLayout for Body {
        const DISC: u8 = 7;
    }

    #[test]
    fn compact_len_is_one_plus_body() {
        assert_eq!(Body::BODY_SIZE, 40);
        assert_eq!(Body::COMPACT_LEN, 41);
    }

    #[test]
    fn validate_checks_len_and_disc() {
        let mut buf = [0u8; 41];
        buf[0] = 7;
        assert!(Body::validate_compact(&buf).is_ok());

        buf[0] = 8;
        assert!(matches!(
            Body::validate_compact(&buf),
            Err(ProgramError::InvalidAccountData)
        ));

        buf[0] = 7;
        assert!(matches!(
            Body::validate_compact(&buf[..40]),
            Err(ProgramError::AccountDataTooSmall)
        ));

        let mut oversized = [0u8; 42];
        oversized[0] = 7;
        assert!(matches!(
            Body::validate_compact(&oversized),
            Err(ProgramError::InvalidAccountData)
        ));
    }
}
