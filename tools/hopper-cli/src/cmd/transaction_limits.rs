//! Transaction-envelope limits shared by Hopper's native Rust send paths.

use solana_transaction::versioned::VersionedTransaction;
use solana_transaction::Transaction;

/// Legacy and v0 envelopes remain capped at 1,232 bytes. The 4,096-byte
/// ceiling belongs only to the v1 format (SIMD-0385), whose feature gate
/// activated on mainnet-beta at slot 447,120,000 on 2026-09-15 and on
/// devnet at slot 492,480,000. `hopper tx send --v1` builds that envelope;
/// every other send path still builds legacy, so the legacy cap is the one
/// they enforce.
pub const LEGACY_V0_MAX_BYTES: usize = 1_232;

/// SIMD-0385 transaction v1 envelope ceiling (`MAX_TRANSACTION_SIZE`). It
/// rides QUIC streams, not a single UDP datagram, which is why it can be
/// larger than the 1,232-byte packet.
pub const V1_MAX_BYTES: usize = 4_096;

/// Bytes one Ed25519 signature occupies in every envelope.
const SIGNATURE_BYTES: usize = 64;

pub fn ensure_legacy_transaction_size(
    transaction: &Transaction,
    operation: &str,
) -> Result<usize, String> {
    let bytes = bincode::serialized_size(transaction)
        .map_err(|error| format!("serialize {operation} transaction: {error}"))?
        as usize;
    validate_legacy_transaction_size(bytes, operation)?;
    Ok(bytes)
}

/// Wire length of a v1 transaction: the serialized message (version byte
/// included) followed by one bare 64-byte signature per required signer,
/// with no length prefix. Wincode's serializer produces exactly this, so
/// counting it by hand avoids a direct dependency on the codec crate.
pub fn v1_transaction_wire_len(transaction: &VersionedTransaction) -> usize {
    let message_bytes = transaction.message.serialize().len();
    let signatures = transaction.message.header().num_required_signatures as usize;
    message_bytes + signatures * SIGNATURE_BYTES
}

pub fn ensure_v1_transaction_size(
    transaction: &VersionedTransaction,
    operation: &str,
) -> Result<usize, String> {
    let bytes = v1_transaction_wire_len(transaction);
    validate_v1_transaction_size(bytes, operation)?;
    Ok(bytes)
}

fn validate_legacy_transaction_size(bytes: usize, operation: &str) -> Result<(), String> {
    if bytes <= LEGACY_V0_MAX_BYTES {
        return Ok(());
    }
    Err(format!(
        "{operation} builds a {bytes}-byte legacy transaction, exceeding the 1,232-byte \
         legacy/v0 network limit. Split the operation or reduce accounts/data, or send a \
         transaction v1 envelope (`hopper tx send --v1`), whose ceiling is 4,096 bytes"
    ))
}

fn validate_v1_transaction_size(bytes: usize, operation: &str) -> Result<(), String> {
    if bytes <= V1_MAX_BYTES {
        return Ok(());
    }
    Err(format!(
        "{operation} builds a {bytes}-byte transaction v1 envelope, exceeding the 4,096-byte \
         SIMD-0385 limit. Split the operation or reduce accounts/data"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_limit_is_inclusive() {
        assert!(validate_legacy_transaction_size(LEGACY_V0_MAX_BYTES, "test").is_ok());
    }

    #[test]
    fn oversize_error_points_at_the_v1_envelope() {
        let error = validate_legacy_transaction_size(1_233, "invoke").unwrap_err();
        assert!(error.contains("1,232-byte"));
        assert!(error.contains("transaction v1"));
        assert!(error.contains("--v1"));
    }

    #[test]
    fn v1_limit_is_inclusive_and_named() {
        assert!(validate_v1_transaction_size(V1_MAX_BYTES, "test").is_ok());
        let error = validate_v1_transaction_size(V1_MAX_BYTES + 1, "send").unwrap_err();
        assert!(error.contains("4,096-byte"));
        assert!(error.contains("SIMD-0385"));
    }
}
