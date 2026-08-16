//! Transaction-envelope limits shared by Hopper's native Rust send paths.

use solana_transaction::Transaction;

/// Legacy and v0 envelopes remain capped at 1,232 bytes. The planned 4,096
/// byte ceiling belongs only to the new v1 format.
pub const LEGACY_V0_MAX_BYTES: usize = 1_232;

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

fn validate_legacy_transaction_size(bytes: usize, operation: &str) -> Result<(), String> {
    if bytes <= LEGACY_V0_MAX_BYTES {
        return Ok(());
    }
    Err(format!(
        "{operation} builds a {bytes}-byte legacy transaction, exceeding the 1,232-byte \
         legacy/v0 network limit. Split the operation or reduce accounts/data. The 4,096-byte \
         ceiling requires transaction v1, which is not finalized or active and is not emitted \
         by this Hopper CLI yet"
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
    fn oversize_error_does_not_imply_v1_is_live() {
        let error = validate_legacy_transaction_size(1_233, "invoke").unwrap_err();
        assert!(error.contains("1,232-byte"));
        assert!(error.contains("transaction v1"));
        assert!(error.contains("not finalized or active"));
    }
}
