//! Byte-exact instruction-data encoders for the Associated Token Account
//! program CPI builders, with Kani proofs pinning the wire format.
//!
//! The ATA program's instructions carry no payload: the instruction data
//! is a single discriminator byte, and every account (payer, wallet,
//! mint, ATA, system/token programs) travels in the account-meta list.
//!
//! These encoders are the single source of truth for the bytes the
//! builders in this crate emit: [`super::Create`],
//! [`super::CreateIdempotent`], and [`super::RecoverNested`] call the
//! `encode_*` functions below, so the `#[cfg(kani)]` proofs verify the
//! actual bytes the CPI path emits.
//!
//! Reference: `<spl-associated-token-account>/src/instruction.rs`
//! `AssociatedTokenAccountInstruction` (Create = 0, CreateIdempotent = 1,
//! RecoverNested = 2).

/// `AssociatedTokenAccountInstruction::Create` discriminator.
pub const IX_CREATE: u8 = 0;
/// `AssociatedTokenAccountInstruction::CreateIdempotent` discriminator.
pub const IX_CREATE_IDEMPOTENT: u8 = 1;
/// `AssociatedTokenAccountInstruction::RecoverNested` discriminator.
pub const IX_RECOVER_NESTED: u8 = 2;

/// Encode `Create`. Layout: `[0]` — 1 byte.
#[inline(always)]
pub fn encode_create() -> [u8; 1] {
    [IX_CREATE]
}

/// Encode `CreateIdempotent`. Layout: `[1]` — 1 byte.
#[inline(always)]
pub fn encode_create_idempotent() -> [u8; 1] {
    [IX_CREATE_IDEMPOTENT]
}

/// Encode `RecoverNested`. Layout: `[2]` — 1 byte.
#[inline(always)]
pub fn encode_recover_nested() -> [u8; 1] {
    [IX_RECOVER_NESTED]
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_vectors() {
        assert_eq!(encode_create(), [0]);
        assert_eq!(encode_create_idempotent(), [1]);
        assert_eq!(encode_recover_nested(), [2]);
    }
}

// =====================================================================
// Kani layout proofs. Each ATA encoder is deterministic (one path), so
// the discriminator + length verdicts below are universal.
// =====================================================================
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn create_is_single_byte_0() {
        let d = encode_create();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], 0);
    }

    #[kani::proof]
    fn create_idempotent_is_single_byte_1() {
        let d = encode_create_idempotent();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], 1);
    }

    #[kani::proof]
    fn recover_nested_is_single_byte_2() {
        let d = encode_recover_nested();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], 2);
    }
}
