//! Self-CPI event emission: the wire format, the verification
//! primitives, and the runtime half of the one-line macro surface.
//!
//! Log output is lossy. Transaction metadata is not. A program that
//! needs events to arrive at indexers regardless of log truncation
//! invokes itself with a distinctive CPI whose bytes carry the event
//! payload — the same trick as Anchor's `emit_cpi!`, on a leaner wire.
//!
//! ## The one-liner
//!
//! Programs do not normally call this module directly. The macro
//! surface wires everything:
//!
//! ```ignore
//! #[hopper::context(event_cpi)]        // ← one attribute option
//! pub struct Deposit {
//!     #[account(mut)]
//!     pub vault: Vault,
//! }
//!
//! #[hopper::program]
//! mod vault_prog {
//!     #[instruction(0)]
//!     fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
//!         // ... state changes ...
//!         ctx.emit_event_cpi(&Deposited { amount: WireU64::new(amount) })?;  // ← one call
//!         Ok(())
//!     }
//! }
//! ```
//!
//! `event_cpi` appends two trailing accounts to the context (the
//! event-authority PDA and the program account itself — the same two
//! Anchor's `#[event_cpi]` appends), validates them at bind, exposes
//! `ctx.emit_event_cpi(&event)` on the bound context, and the
//! `#[hopper::program]` dispatcher grows a self-CPI sink on the
//! reserved `[0xE0, 0x1E]` marker that authenticates each event before
//! accepting it. Programs that never opt in pay nothing: the sink
//! guard is a `const` that dead-code-eliminates.
//!
//! ## Wire format
//!
//! ```text
//! [0..2]   CPI_EVENT_MARKER   (0xE0, 0x1E)
//! [2]      event tag          (the byte from `#[hopper::event(tag = N)]`)
//! [3..]    event payload      (the event's Pod bytes)
//! ```
//!
//! Three bytes of instruction-data overhead per event. Anchor's
//! `emit_cpi!` spends sixteen: the 8-byte `EVENT_IX_TAG_LE` instruction
//! discriminator plus the event's own 8-byte account-style
//! discriminator. Hopper's 2-byte marker + 1-byte tag table carries the
//! same routing information for 13 fewer instruction-data bytes per
//! event (5 fewer counting only the event-identification layer: 3-byte
//! marker+tag vs one 8-byte hash discriminator).
//!
//! ## Why the sink verifies
//!
//! The generated sink is not a bare no-op. Any program can CPI into
//! any other program, so an unauthenticated sink would let an attacker
//! program plant forged "events" in your program's inner-instruction
//! list. The sink therefore requires the event-authority PDA — seeds
//! [`EVENT_AUTHORITY_SEED`], owned by *this* program's id — to sign the
//! CPI. Only this program's own `invoke_signed` can produce that
//! signature, which is exactly Anchor's authenticity argument for its
//! `event_authority` account.
//!
//! On-chain the verification is the sha256-only compare loop from
//! [`crate::pda::find_and_verify_pda`] (~200 CU for bump 255, +~200 per
//! additional attempt) — no `create_program_address` syscalls, no curve
//! validation. Anchor v0.31+ pins the same PDA against a compile-time
//! constant; Hopper has no compile-time program id, so it derives at
//! runtime and keeps the check honest. Off-chain hosts have no sha256
//! syscall (see [`crate::pda`]), so host builds enforce the marker and
//! the signer flag and document the address pin as an on-chain check.
//!
//! ## Manual wiring (appendix)
//!
//! The pre-`event_cpi` manual pattern remains supported for programs
//! that want custom control. Declare the sentinel yourself:
//!
//! ```ignore
//! #[instruction(discriminator = [0xE0, 0x1E])]
//! fn __hopper_event_sink(ctx: &mut Context<'_>) -> ProgramResult {
//!     // Recommended: authenticate instead of no-op'ing.
//!     hopper_runtime::cpi_event::handle_event_sink(ctx, ctx.instruction_data())
//! }
//! ```
//!
//! pass the event-authority PDA + program account in your context, and
//! emit through [`crate::hopper_emit_cpi!`] (or [`encode_event_cpi`] +
//! [`invoke_event_cpi`] for full control).

/// The reserved self-CPI event discriminator.
///
/// Placed at the start of every emitted event CPI. The generated
/// dispatcher routes instruction data with this prefix to the event
/// sink; manual programs match it with
/// `#[instruction(discriminator = [0xE0, 0x1E])]`.
pub const CPI_EVENT_MARKER: [u8; 2] = [0xE0, 0x1E];

/// Canonical PDA seed for the Hopper event-authority. The
/// `#[hopper::context(event_cpi)]` machinery derives, verifies, and
/// signs with this seed; manual programs must match it so the CPI
/// signer resolves.
pub const EVENT_AUTHORITY_SEED: &[u8] = b"__hopper_event_authority";

/// Maximum event payload accepted by the emit helpers' stack buffer.
///
/// `3 + MAX_EVENT_PAYLOAD` bytes of stack per emit call. 512 payload
/// bytes fits every sensibly-sized event; larger events should use the
/// log-based `emit!` path or hand-rolled encoding.
pub const MAX_EVENT_PAYLOAD: usize = 512;

/// The bump byte host builds report for the event authority.
///
/// Off-chain targets have no sha256 syscall, so the real bump cannot be
/// derived there (see [`crate::pda::find_program_address`]). Host-side
/// `bind()` therefore records this placeholder; the host CPI emulation
/// validates the signer *dimension* (the fixture must be marked signer)
/// rather than the derivation. On-chain the recorded bump is always the
/// real one returned by the sha256 verify loop.
pub const HOST_EVENT_AUTHORITY_BUMP: u8 = 255;

/// A self-CPI-emittable event: a stable 1-byte tag plus a borrowed
/// payload view.
///
/// `#[hopper::event]` implements this automatically (the tag is the
/// `tag = N` byte, the payload is the struct's Pod bytes), which is
/// what lets `ctx.emit_event_cpi(&event)` and
/// [`crate::hopper_emit_cpi!`] accept any declared event without
/// hand-written glue.
pub trait CpiEvent {
    /// Stable event discriminator tag byte (`#[hopper::event(tag = N)]`).
    const TAG: u8;

    /// Borrowed byte view of the event payload (no marker, no tag).
    fn payload_bytes(&self) -> &[u8];

    /// Value-level accessor for [`Self::TAG`], for macro call sites
    /// that only hold an expression.
    #[inline(always)]
    fn tag(&self) -> u8 {
        Self::TAG
    }
}

/// Fill an out buffer with the CPI wire format for an event.
///
/// Returns the number of bytes written. Caller picks the buffer size;
/// `2 + 1 + payload.len()` is always sufficient. Returns `None` if
/// the out buffer is too small.
///
/// Zero-alloc. Compiles to a pair of `copy_from_slice` calls.
///
/// ```ignore
/// let mut buf = [0u8; 3 + Deposited::PACKED_SIZE];
/// let len = hopper_runtime::cpi_event::encode_event_cpi(
///     Deposited::TAG,
///     event.payload_bytes(),
///     &mut buf,
/// ).unwrap();
/// ```
#[inline]
pub fn encode_event_cpi(event_tag: u8, event_payload: &[u8], out: &mut [u8]) -> Option<usize> {
    let total = 2 + 1 + event_payload.len();
    if out.len() < total {
        return None;
    }
    out[0..2].copy_from_slice(&CPI_EVENT_MARKER);
    out[2] = event_tag;
    out[3..total].copy_from_slice(event_payload);
    Some(total)
}

/// Decode the CPI wire format back into `(tag, payload)`.
///
/// The exact inverse of [`encode_event_cpi`]: returns `None` unless the
/// data starts with [`CPI_EVENT_MARKER`] and carries at least the tag
/// byte. Indexers scanning inner instructions and tests asserting
/// round-trips both use this as the single source of decode truth.
#[inline]
pub fn decode_event_cpi(data: &[u8]) -> Option<(u8, &[u8])> {
    if data.len() < 3 || data[0..2] != CPI_EVENT_MARKER {
        return None;
    }
    Some((data[2], &data[3..]))
}

/// Verify an account is this program's event-authority PDA and return
/// its bump.
///
/// This is the bind-time check behind `#[hopper::context(event_cpi)]`:
/// the appended `event_authority` account must be the PDA of
/// [`EVENT_AUTHORITY_SEED`] under the executing program id. On-chain it
/// runs the sha256-only verify loop (`find_and_verify_pda`, ~200 CU for
/// bump 255) and returns the real bump for the CPI signer seeds.
///
/// Off-chain hosts cannot derive (no sha256 syscall; see
/// [`crate::pda::find_program_address`]), so the host branch accepts
/// the account and reports [`HOST_EVENT_AUTHORITY_BUMP`]; the host CPI
/// emulation still enforces the signer dimension at emit time.
#[inline]
pub fn verify_event_authority(
    event_authority: &crate::account::AccountView<'_>,
    program_id: &crate::address::Address,
) -> Result<u8, crate::error::ProgramError> {
    #[cfg(target_os = "solana")]
    {
        crate::pda::find_and_verify_pda(event_authority, &[EVENT_AUTHORITY_SEED], program_id)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (event_authority, program_id);
        Ok(HOST_EVENT_AUTHORITY_BUMP)
    }
}

/// The event sink: validate an incoming self-CPI event instruction.
///
/// The `#[hopper::program]` dispatcher routes instruction data whose
/// first bytes match [`CPI_EVENT_MARKER`] here (for programs whose
/// contexts opted into `event_cpi`). Checks, in order:
///
/// 1. the data really is `[0xE0, 0x1E, tag, ..]` (≥ 3 bytes);
/// 2. `accounts[0]` — the event authority — signed the CPI. Only this
///    program's own `invoke_signed` can sign for its event-authority
///    PDA, so this is what makes accepted events authentic;
/// 3. (on-chain) `accounts[0]`'s address is the PDA of
///    [`EVENT_AUTHORITY_SEED`] under this program id, via the sha256
///    verify loop. Without the address pin, any keypair the attacker
///    controls could satisfy the signer check. Hosts have no sha256
///    syscall, so off-chain builds stop at the signer check.
///
/// Returns `Ok(())` for a valid event, which is all a sink must do:
/// the payload lives in the transaction's inner-instruction record.
#[inline]
pub fn handle_event_sink(
    ctx: &crate::context::Context<'_>,
    data: &[u8],
) -> crate::result::ProgramResult {
    if data.len() < 3 || data[0..2] != CPI_EVENT_MARKER {
        return Err(crate::error::ProgramError::InvalidInstructionData);
    }
    let authority = ctx.account(0)?;
    if !authority.is_signer() {
        return Err(crate::error::ProgramError::MissingRequiredSignature);
    }
    #[cfg(target_os = "solana")]
    {
        let _bump =
            crate::pda::find_and_verify_pda(authority, &[EVENT_AUTHORITY_SEED], ctx.program_id())?;
    }
    Ok(())
}

/// Invoke a self-CPI carrying the encoded event payload.
///
/// Builds the one-account instruction (event-authority as signer) and
/// hands it to Hopper's checked `invoke_signed`, so the emit rides the
/// cheapest safe invoke tier: on-chain that is the fused
/// validate+build pass over one account followed by the CPI syscall;
/// off-chain the same call runs the host emulation's meta/signer/borrow
/// validation, then records the would-be inner instruction for test
/// harnesses (under `test` or the `thread-local-registry` feature) so
/// end-to-end tests can assert the exact wire bytes.
///
/// This is the function `ctx.emit_event_cpi(..)` and
/// [`crate::hopper_emit_cpi!`] call. Users who want finer-grained
/// control over the CPI (extra accounts, custom signer) can call this
/// directly with their own encoded data.
#[inline]
pub fn invoke_event_cpi(
    program_id: &crate::address::Address,
    event_authority: &crate::account::AccountView<'_>,
    data: &[u8],
    authority_seeds: &[&[u8]],
) -> crate::result::ProgramResult {
    use crate::instruction::{InstructionAccount, InstructionView, Seed, Signer};
    if authority_seeds.len() > crate::address::MAX_SEEDS {
        return Err(crate::error::ProgramError::MaxSeedLengthExceeded);
    }

    let account_meta = InstructionAccount {
        address: event_authority.address(),
        is_signer: true,
        is_writable: false,
    };
    let ix = InstructionView {
        program_id,
        accounts: ::core::slice::from_ref(&account_meta),
        data,
    };
    let mut seed_storage: [::core::mem::MaybeUninit<Seed<'_>>; crate::address::MAX_SEEDS] =
        // SAFETY: MaybeUninit elements do not require initialization.
        unsafe { ::core::mem::MaybeUninit::uninit().assume_init() };
    let mut seed_index = 0;
    while seed_index < authority_seeds.len() {
        seed_storage[seed_index].write(Seed::from(authority_seeds[seed_index]));
        seed_index += 1;
    }
    let seed_slice =
        // SAFETY: The first `authority_seeds.len()` slots were initialized above.
        unsafe {
            ::core::slice::from_raw_parts(
                seed_storage.as_ptr() as *const Seed<'_>,
                authority_seeds.len(),
            )
        };
    let signer_list = [Signer::from(seed_slice)];
    let account_views = [event_authority];
    crate::cpi::invoke_signed::<1>(&ix, &account_views, &signer_list)?;

    // Host observation point: after the emulated CPI validates, record
    // the inner instruction a real transaction would carry, so host
    // test harnesses can assert the exact marker+tag+payload bytes.
    // Compiled out entirely on-chain and on hosts without the test cfg.
    #[cfg(all(
        not(target_os = "solana"),
        any(test, feature = "thread-local-registry")
    ))]
    host_capture::record(program_id, event_authority.address(), data);

    Ok(())
}

/// Host-side capture of emitted event CPIs, for test observation.
///
/// On-chain the self-CPI lands in the transaction's inner-instruction
/// metadata; off-chain there is no ledger, so [`invoke_event_cpi`]
/// records each successful emit here instead. Per-thread (the same
/// invocation-scope reasoning as the borrow registry's
/// `thread-local-registry` lane), available under `test` or the
/// `thread-local-registry` feature — exactly the lanes where `std` is
/// already linked.
#[cfg(all(
    not(target_os = "solana"),
    any(test, feature = "thread-local-registry")
))]
mod host_capture {
    use core::cell::RefCell;

    /// One captured self-CPI event emission.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct CapturedEventCpi {
        /// The program that emitted (and is the CPI target).
        pub program_id: crate::address::Address,
        /// The event-authority account passed as the CPI signer.
        pub authority: crate::address::Address,
        /// The exact instruction data: marker + tag + payload.
        pub data: std::vec::Vec<u8>,
    }

    std::thread_local! {
        static CAPTURED: RefCell<std::vec::Vec<CapturedEventCpi>> =
            const { RefCell::new(std::vec::Vec::new()) };
    }

    pub(super) fn record(
        program_id: &crate::address::Address,
        authority: &crate::address::Address,
        data: &[u8],
    ) {
        CAPTURED.with(|captured| {
            captured.borrow_mut().push(CapturedEventCpi {
                program_id: *program_id,
                authority: *authority,
                data: data.to_vec(),
            });
        });
    }

    /// Drain this thread's captured event CPIs (oldest first).
    pub fn take_host_captured_event_cpis() -> std::vec::Vec<CapturedEventCpi> {
        CAPTURED.with(|captured| core::mem::take(&mut *captured.borrow_mut()))
    }
}

#[cfg(all(
    not(target_os = "solana"),
    any(test, feature = "thread-local-registry")
))]
pub use host_capture::{take_host_captured_event_cpis, CapturedEventCpi};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::AccountView;
    use crate::address::Address;
    use crate::context::Context;
    use crate::error::ProgramError;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

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

    #[test]
    fn decode_is_the_exact_inverse_of_encode() {
        let payload = [9u8, 8, 7, 6, 5];
        let mut buf = [0u8; 3 + 5];
        let len = encode_event_cpi(0x2A, &payload, &mut buf).unwrap();
        let (tag, decoded) = decode_event_cpi(&buf[..len]).expect("decodable");
        assert_eq!(tag, 0x2A);
        assert_eq!(decoded, &payload);

        // Zero payload round-trips too.
        let mut buf3 = [0u8; 3];
        let len3 = encode_event_cpi(0x01, &[], &mut buf3).unwrap();
        assert_eq!(decode_event_cpi(&buf3[..len3]), Some((0x01, &[][..])));
    }

    #[test]
    fn decode_rejects_short_or_mismarked_data() {
        assert_eq!(decode_event_cpi(&[]), None);
        assert_eq!(decode_event_cpi(&[0xE0, 0x1E]), None, "marker without tag");
        assert_eq!(decode_event_cpi(&[0xE0, 0x77, 0x01]), None, "wrong marker");
        assert_eq!(decode_event_cpi(&[0x00, 0x1E, 0x01]), None, "wrong marker");
    }

    #[test]
    fn trait_tag_defaults_to_the_associated_const() {
        struct Ping;
        impl CpiEvent for Ping {
            const TAG: u8 = 0x5A;
            fn payload_bytes(&self) -> &[u8] {
                &[]
            }
        }
        assert_eq!(Ping.tag(), 0x5A);
    }

    /// Build a minimal host account fixture (same pattern as the
    /// context write-policy tests).
    fn make_account(
        address_byte: u8,
        is_signer: bool,
    ) -> (std::vec::Vec<u8>, AccountView<'static>) {
        const DATA_LEN: usize = 8;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + DATA_LEN];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: `backing` is sized for the header plus DATA_LEN bytes and
        // outlives the returned view (the caller holds the Vec).
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: is_signer as u8,
                is_writable: 0,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([address_byte; 32]),
                owner: NativeAddress::new_from_array([0; 32]),
                lamports: 0,
                data_len: DATA_LEN as u64,
            });
        }
        // SAFETY: `raw` points at a fully initialized RuntimeAccount with
        // its data region in the same allocation.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    #[test]
    fn sink_rejects_short_data_and_wrong_marker() {
        let (_b, authority) = make_account(1, true);
        let accounts = [authority];
        let pid = Address::new([9u8; 32]);
        let ctx = Context::new(&pid, &accounts, &[]);

        assert_eq!(
            handle_event_sink(&ctx, &[0xE0, 0x1E]),
            Err(ProgramError::InvalidInstructionData),
            "marker without a tag byte must be refused"
        );
        assert_eq!(
            handle_event_sink(&ctx, &[0xE0, 0x77, 0x01]),
            Err(ProgramError::InvalidInstructionData),
            "a wrong second marker byte must be refused"
        );
    }

    #[test]
    fn sink_requires_the_authority_to_sign() {
        let (_b, authority) = make_account(1, false);
        let accounts = [authority];
        let pid = Address::new([9u8; 32]);
        let ctx = Context::new(&pid, &accounts, &[]);

        assert_eq!(
            handle_event_sink(&ctx, &[0xE0, 0x1E, 0x42, 1, 2]),
            Err(ProgramError::MissingRequiredSignature),
            "an unsigned authority is a forged event and must be refused"
        );
    }

    #[test]
    fn sink_accepts_a_signed_authority_on_host() {
        // Host builds stop at the signer check (no sha256 syscall to pin
        // the PDA address); the address pin is on-chain-only, documented
        // on `handle_event_sink`.
        let (_b, authority) = make_account(1, true);
        let accounts = [authority];
        let pid = Address::new([9u8; 32]);
        let ctx = Context::new(&pid, &accounts, &[]);

        assert_eq!(handle_event_sink(&ctx, &[0xE0, 0x1E, 0x42]), Ok(()));
    }

    #[test]
    fn sink_requires_the_authority_account_to_be_present() {
        let pid = Address::new([9u8; 32]);
        let accounts: [AccountView<'static>; 0] = [];
        let ctx = Context::new(&pid, &accounts, &[]);
        assert!(
            handle_event_sink(&ctx, &[0xE0, 0x1E, 0x42]).is_err(),
            "a sink CPI without the authority account must be refused"
        );
    }

    #[test]
    fn host_verify_event_authority_reports_the_placeholder_bump() {
        let (_b, authority) = make_account(3, false);
        let pid = Address::new([9u8; 32]);
        assert_eq!(
            verify_event_authority(&authority, &pid),
            Ok(HOST_EVENT_AUTHORITY_BUMP)
        );
    }

    #[test]
    fn host_invoke_validates_the_signer_and_captures_the_wire_bytes() {
        let _ = take_host_captured_event_cpis();

        let pid = Address::new([9u8; 32]);
        let bump = [HOST_EVENT_AUTHORITY_BUMP];
        let seeds: [&[u8]; 2] = [EVENT_AUTHORITY_SEED, &bump];

        let mut buf = [0u8; 3 + MAX_EVENT_PAYLOAD];
        let len = encode_event_cpi(0x42, &[7, 7, 7], &mut buf).unwrap();

        // Unsigned authority: the host CPI emulation refuses the emit
        // (off-chain, PDA-seed satisfaction cannot be derived, so the
        // fixture itself must carry the signer flag) and captures nothing.
        let (_b0, unsigned) = make_account(4, false);
        assert_eq!(
            invoke_event_cpi(&pid, &unsigned, &buf[..len], &seeds),
            Err(ProgramError::MissingRequiredSignature)
        );
        assert!(take_host_captured_event_cpis().is_empty());

        // Signed authority: the emit validates and the exact wire bytes
        // are captured for the harness.
        let (_b1, signed) = make_account(5, true);
        assert_eq!(invoke_event_cpi(&pid, &signed, &buf[..len], &seeds), Ok(()));
        let captured = take_host_captured_event_cpis();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].program_id, pid);
        assert_eq!(captured[0].authority, *signed.address());
        assert_eq!(captured[0].data, &buf[..len]);
        assert_eq!(
            decode_event_cpi(&captured[0].data),
            Some((0x42, &[7u8, 7, 7][..])),
            "captured bytes must round-trip the public decoder"
        );

        // The take drained the buffer.
        assert!(take_host_captured_event_cpis().is_empty());
    }

    #[test]
    fn invoke_rejects_too_many_seeds() {
        let (_b, authority) = make_account(6, true);
        let pid = Address::new([9u8; 32]);
        let too_many: [&[u8]; crate::address::MAX_SEEDS + 1] =
            [&[1u8][..]; crate::address::MAX_SEEDS + 1];
        assert_eq!(
            invoke_event_cpi(&pid, &authority, &[0xE0, 0x1E, 0x01], &too_many),
            Err(ProgramError::MaxSeedLengthExceeded)
        );
    }
}
