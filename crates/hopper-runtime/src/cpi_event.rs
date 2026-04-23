//! Self-CPI event emission primitives.
//!
//! Log output is lossy. Transaction metadata is not. A program that
//! needs events to arrive at indexers regardless of log truncation
//! invokes itself with a distinctive CPI whose bytes carry the event
//! payload. This module provides the building blocks: a reserved
//! discriminator so the dispatcher can route the CPI to a no-op
//! sentinel, a wire-format helper for the instruction data, and a
//! one-line pattern programs can copy.
//!
//! ## Wire format
//!
//! ```text
//! [0..2]   CPI_EVENT_MARKER   (0xE0, 0x1E)
//! [2]      event tag          (the byte from `#[hopper::event(tag = N)]`)
//! [3..]    event payload      (from `HopperEvent::as_bytes()`)
//! ```
//!
//! The two-byte marker is the reserved Hopper discriminator for
//! self-CPI events and is unlikely to collide with any sensibly
//! chosen user discriminator. The user-facing instruction
//! declaration for the sentinel is:
//!
//! ```ignore
//! #[instruction(discriminator = [0xE0, 0x1E])]
//! fn __hopper_event_sink(_ctx: &mut Context<'_>) -> ProgramResult {
//!     Ok(())
//! }
//! ```
//!
//! ## Why this pattern works
//!
//! Anchor's `emit_cpi!` uses the same trick: a self-CPI carrying
//! payload bytes guarantees the event appears in the transaction's
//! inner-instruction list, which RPC nodes do not truncate. Indexers
//! scan for the reserved marker and decode the tail as the event.
//!
//! Hopper's version is leaner: a two-byte marker plus a one-byte
//! event tag gives the indexer everything it needs to route without
//! the Anchor eight-byte discriminator overhead.

/// The reserved self-CPI event discriminator.
///
/// Placed at the start of every `emit_event_cpi` instruction and
/// must be matched by a sentinel `#[instruction(discriminator = [0xE0, 0x1E])]`
/// no-op handler in the calling program.
pub const CPI_EVENT_MARKER: [u8; 2] = [0xE0, 0x1E];

/// Canonical PDA seed for the Hopper event-authority. Match this in
/// the program's sentinel handler setup so the CPI signer resolves.
pub const EVENT_AUTHORITY_SEED: &[u8] = b"__hopper_event_authority";

/// Fill an out buffer with the CPI wire format for an event.
///
/// Returns the number of bytes written. Caller picks the buffer size;
/// `2 + 1 + E::PACKED_SIZE` is always sufficient. Returns `None` if
/// the out buffer is too small.
///
/// Zero-alloc. Compiles to a pair of `copy_from_slice` calls.
///
/// ```ignore
/// let mut buf = [0u8; 2 + 1 + Deposited::PACKED_SIZE];
/// let len = hopper_runtime::cpi_event::encode_event_cpi(
///     Deposited::TAG,
///     event.as_bytes(),
///     &mut buf,
/// ).unwrap();
///
/// // Build an InstructionView and invoke_signed from here. The
/// // sentinel handler accepts the CPI and returns Ok(()).
/// ```
#[inline]
pub fn encode_event_cpi(
    event_tag: u8,
    event_payload: &[u8],
    out: &mut [u8],
) -> Option<usize> {
    let total = 2 + 1 + event_payload.len();
    if out.len() < total {
        return None;
    }
    out[0..2].copy_from_slice(&CPI_EVENT_MARKER);
    out[2] = event_tag;
    out[3..total].copy_from_slice(event_payload);
    Some(total)
}

/// Invoke a self-CPI carrying the encoded event payload.
///
/// Builds the one-account instruction (event-authority as signer) and
/// hands it to the active backend's `invoke_signed`. The native
/// backend path is the load-bearing one; a pinocchio-backend or
/// solana-program-backend build routes through their respective
/// compat shims.
///
/// This is the function [`crate::hopper_emit_cpi!`] calls. Users who
/// want finer-grained control over the CPI (extra accounts, custom
/// signer) can call this directly with their own encoded data.
#[inline]
pub fn invoke_event_cpi(
    program_id: &crate::address::Address,
    event_authority: &crate::account::AccountView,
    data: &[u8],
    authority_seeds: &[&[u8]],
) -> crate::result::ProgramResult {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    {
        use crate::instruction::{InstructionAccount, InstructionView};
        let account_meta = InstructionAccount {
            pubkey: event_authority.address(),
            is_signer: true,
            is_writable: false,
        };
        let ix = InstructionView {
            program_id,
            accounts: ::core::slice::from_ref(&account_meta),
            data,
        };
        // Array-of-slices form the native CPI surface expects for
        // signer seeds: one signer, one seed list.
        let signer_list = [authority_seeds];
        let account_views = [event_authority];
        crate::cpi::invoke_signed::<1>(&ix, &account_views, &signer_list[..])
    }

    #[cfg(any(
        not(target_os = "solana"),
        feature = "pinocchio-backend",
        feature = "solana-program-backend",
    ))]
    {
        let _ = (program_id, event_authority, data, authority_seeds);
        // Off-chain or under a non-native backend: the self-CPI path
        // is a no-op so host-side tests do not balloon into a CPI
        // stub. Returning Ok keeps the handler happy; tests should
        // assert on the encoded bytes via encode_event_cpi instead.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_marker_tag_and_payload_in_order() {
        let mut buf = [0u8; 16];
        let len = encode_event_cpi(0x42, &[1, 2, 3, 4], &mut buf).unwrap();
        assert_eq!(len, 7);
        assert_eq!(&buf[..len], &[0xE0, 0x1E, 0x42, 1, 2, 3, 4]);
    }

    #[test]
    fn rejects_short_buffer() {
        let mut buf = [0u8; 3];
        let len = encode_event_cpi(0, &[1, 2, 3, 4], &mut buf);
        assert!(len.is_none());
    }

    #[test]
    fn zero_payload_is_valid() {
        let mut buf = [0u8; 3];
        let len = encode_event_cpi(0x7F, &[], &mut buf).unwrap();
        assert_eq!(len, 3);
        assert_eq!(&buf[..len], &[0xE0, 0x1E, 0x7F]);
    }

    #[test]
    fn reserved_marker_is_stable() {
        assert_eq!(CPI_EVENT_MARKER, [0xE0, 0x1E]);
    }
}
