//! # Hopper SDK. Off-chain companion for the Hopper framework
//!
//! This crate is the **symmetric off-chain half** of Hopper. Where `hopper-core`,
//! `hopper-runtime`, and `hopper-macros-proc` own the on-chain safety surface,
//! `hopper-sdk` owns the off-chain consumer surface: indexers, explorers,
//! wallets, back-ends, and clients.
//!
//! ## Why this exists
//!
//! Neither Pinocchio, Anchor zero-copy, nor Quasar ships a symmetric off-chain
//! SDK that understands the framework's own wire shapes. Clients for those
//! frameworks tend to re-implement borsh/IDL decoders from scratch and always
//! lag on-chain semantics. Hopper closes that loop:
//!
//! - **Receipts are a first-class wire format** (64-byte fixed, documented in
//!   the program manifest). This crate parses them and narrates them.
//! - **Layout fingerprints are mutual**. A client can verify the on-chain
//!   account header matches the layout_id it was compiled against before any
//!   decoding. No "surprise layout change" incidents.
//! - **Segment-aware partial reads**. Because Hopper knows field offsets at the
//!   segment level, clients can load just the bytes they need. the same
//!   property the on-chain side uses to minimize CU cost.
//! - **Manifest-driven builders**. Instructions and account lists come out of
//!   the `ProgramManifest` so the on-chain definition is the single source of
//!   truth.
//!
//! ## Module map
//!
//! - [`receipt`]. Decode the Hopper 64-byte receipt wire format and convert
//!   it into structured data or a human-readable narrative.
//! - [`reader`]. Segment-aware partial account readers that only pull the
//!   fields the caller asked for. Rejects mismatched `layout_id`.
//! - [`builder`]. Instruction and account-list builder driven by the
//!   `ProgramManifest`. Zero borsh dependency.
//! - [`diff`]. Snapshot-to-snapshot diff producer symmetric with
//!   `hopper-core::diff`.
//! - [`fingerprint`]. Runtime layout_id verification helpers.
//!
//! ## Relationship to `hopper-schema`
//!
//! This crate is a **consumer** of the `ProgramManifest` types defined in
//! `hopper-schema`. It does not duplicate the schema. it operates over it.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

// `alloc` is always available. the SDK ships zero-copy primitives in the
// hot path (`SegmentReader`, `DecodedReceipt::parse`), but the optional
// narrative / diff / builder surfaces allocate for `String` and `Vec`. We
// pull `alloc` in unconditionally so those modules compile cleanly in
// `no_std + alloc` targets (the default deployment shape for indexers).
extern crate alloc;

pub mod diff;
pub mod fingerprint;
pub mod reader;
pub mod receipt;

#[cfg(feature = "builder")]
pub mod builder;

// Surface the most commonly used types at the crate root.
pub use fingerprint::{FingerprintCheck, FingerprintError};
pub use reader::{ReaderError, SegmentReader};
pub use receipt::{DecodedReceipt, ReceiptError, ReceiptWire};

#[cfg(feature = "narrate")]
pub use receipt::narrative::{Narrator, ReceiptNarrative};

/// SDK-level error surface. All sub-errors lift into this enum for easy
/// `?`-propagation in consumer code.
#[derive(Debug)]
pub enum SdkError {
    /// Receipt decode failure.
    Receipt(ReceiptError),
    /// Segment-aware reader failure.
    Reader(ReaderError),
    /// Layout fingerprint mismatch.
    Fingerprint(FingerprintError),
}

impl core::fmt::Display for SdkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SdkError::Receipt(e) => write!(f, "receipt: {:?}", e),
            SdkError::Reader(e) => write!(f, "reader: {:?}", e),
            SdkError::Fingerprint(e) => write!(f, "fingerprint: {:?}", e),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SdkError {}

impl From<ReceiptError> for SdkError {
    fn from(e: ReceiptError) -> Self {
        SdkError::Receipt(e)
    }
}
impl From<ReaderError> for SdkError {
    fn from(e: ReaderError) -> Self {
        SdkError::Reader(e)
    }
}
impl From<FingerprintError> for SdkError {
    fn from(e: FingerprintError) -> Self {
        SdkError::Fingerprint(e)
    }
}
