//! Anchor discriminator computation and account body extraction.
//!
//! Anchor uses `SHA256("account:<TypeName>")[..8]` as an 8-byte
//! discriminator prefix. This module lets Hopper programs verify and
//! strip that prefix to read Anchor-created account data.

use hopper_runtime::error::ProgramError;

// ── Discriminator Computation ────────────────────────────────────────────────

/// Compute the Anchor account discriminator for a type name.
///
/// `disc = SHA256("account:<type_name>")[..8]`
///
/// This is a `const fn` -- the discriminator is computed at compile time
/// when `type_name` is a string literal.
///
/// ```rust,ignore
/// const MY_DISC: [u8; 8] = anchor_disc("MyAccount");
/// ```
pub const fn anchor_disc(type_name: &str) -> [u8; 8] {
    sha256_prefix(b"account:", type_name.as_bytes())
}

/// Compute the Anchor instruction discriminator for a function name.
///
/// `disc = SHA256("global:<fn_name>")[..8]`
pub const fn anchor_ix_disc(fn_name: &str) -> [u8; 8] {
    sha256_prefix(b"global:", fn_name.as_bytes())
}

/// Compute the Anchor event discriminator for an event name.
///
/// `disc = SHA256("event:<event_name>")[..8]`
pub const fn anchor_event_disc(event_name: &str) -> [u8; 8] {
    sha256_prefix(b"event:", event_name.as_bytes())
}

// ── Discriminator Verification ───────────────────────────────────────────────

/// Check that the first 8 bytes of `data` match `expected`.
///
/// Returns `InvalidAccountData` if the discriminator doesn't match
/// or the data is too short.
#[inline(always)]
pub fn check_anchor_disc(data: &[u8], expected: &[u8; 8]) -> Result<(), ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if data[..8] != expected[..] {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Return the account body (everything after the 8-byte discriminator).
///
/// Does not validate the discriminator. Use `check_and_body` if you
/// need validation + extraction in one call.
#[inline(always)]
pub fn anchor_body(data: &[u8]) -> Result<&[u8], ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(&data[8..])
}

/// Return a mutable reference to the body after the discriminator.
#[inline(always)]
pub fn anchor_body_mut(data: &mut [u8]) -> Result<&mut [u8], ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(&mut data[8..])
}

/// Validate the discriminator and return the body in one call.
#[inline(always)]
pub fn check_and_body<'a>(data: &'a [u8], expected: &[u8; 8]) -> Result<&'a [u8], ProgramError> {
    check_anchor_disc(data, expected)?;
    Ok(&data[8..])
}

/// Validate an instruction discriminator and return the body.
#[inline(always)]
pub fn check_ix_and_body<'a>(data: &'a [u8], expected: &[u8; 8]) -> Result<&'a [u8], ProgramError> {
    check_anchor_disc(data, expected)?;
    Ok(&data[8..])
}

// ── Internal SHA256 helpers ──────────────────────────────────────────────────

/// Compute SHA256(prefix || suffix)[..8] at compile time.
const fn sha256_prefix(prefix: &[u8], suffix: &[u8]) -> [u8; 8] {
    let hash = sha2_const_stable::Sha256::new()
        .update(prefix)
        .update(suffix)
        .finalize();

    let mut disc = [0u8; 8];
    let mut k = 0;
    while k < 8 {
        disc[k] = hash[k];
        k += 1;
    }
    disc
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::vec;

    #[test]
    fn disc_is_deterministic() {
        let d1 = anchor_disc("MyAccount");
        let d2 = anchor_disc("MyAccount");
        assert_eq!(d1, d2);
    }

    #[test]
    fn different_names_different_discs() {
        let d1 = anchor_disc("Vault");
        let d2 = anchor_disc("Escrow");
        assert_ne!(d1, d2);
    }

    #[test]
    fn ix_disc_differs_from_account_disc() {
        let account = anchor_disc("Initialize");
        let ix = anchor_ix_disc("initialize");
        assert_ne!(account, ix);
    }

    #[test]
    fn event_disc_differs_from_others() {
        let account = anchor_disc("Transfer");
        let ix = anchor_ix_disc("transfer");
        let event = anchor_event_disc("Transfer");
        assert_ne!(account, event);
        assert_ne!(ix, event);
    }

    #[test]
    fn check_disc_succeeds() {
        let disc = anchor_disc("TestAccount");
        let mut data = vec![0u8; 100];
        data[..8].copy_from_slice(&disc);
        assert!(check_anchor_disc(&data, &disc).is_ok());
    }

    #[test]
    fn check_disc_rejects_wrong() {
        let disc = anchor_disc("TestAccount");
        let data = vec![0u8; 100];
        assert!(check_anchor_disc(&data, &disc).is_err());
    }

    #[test]
    fn check_disc_rejects_short() {
        let disc = anchor_disc("TestAccount");
        let data = vec![0u8; 4];
        assert!(check_anchor_disc(&data, &disc).is_err());
    }

    #[test]
    fn anchor_body_returns_tail() {
        let mut data = vec![0u8; 20];
        data[8] = 42;
        data[9] = 99;
        let body = anchor_body(&data).unwrap();
        assert_eq!(body.len(), 12);
        assert_eq!(body[0], 42);
        assert_eq!(body[1], 99);
    }

    #[test]
    fn anchor_body_mut_writes() {
        let mut data = vec![0u8; 20];
        let body = anchor_body_mut(&mut data).unwrap();
        body[0] = 0xFF;
        assert_eq!(data[8], 0xFF);
    }

    #[test]
    fn check_and_body_combined() {
        let disc = anchor_disc("Vault");
        let mut data = vec![0u8; 50];
        data[..8].copy_from_slice(&disc);
        data[8] = 77;
        let body = check_and_body(&data, &disc).unwrap();
        assert_eq!(body[0], 77);
        assert_eq!(body.len(), 42);
    }

    #[test]
    fn check_and_body_rejects_wrong_disc() {
        let disc = anchor_disc("Vault");
        let data = vec![0u8; 50];
        assert!(check_and_body(&data, &disc).is_err());
    }

    #[test]
    fn check_ix_and_body_works() {
        let disc = anchor_ix_disc("initialize");
        let mut data = vec![0u8; 30];
        data[..8].copy_from_slice(&disc);
        data[8] = 5;
        let body = check_ix_and_body(&data, &disc).unwrap();
        assert_eq!(body[0], 5);
    }
}
