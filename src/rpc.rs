//! Lightweight Solana JSON-RPC client and PDA derivation for the Hopper CLI.
//!
//! No heavy SDK dependency — just `ureq` for HTTP, `sha2` + `curve25519-dalek`
//! for PDA derivation, and `bs58`/`base64` for encoding.

use sha2::{Sha256, Digest};

// ---------------------------------------------------------------------------
// PDA derivation (matches solana_program::pubkey::Pubkey::find_program_address)
// ---------------------------------------------------------------------------

/// Derive a Program Derived Address from seeds and a program ID.
///
/// Iterates bump seeds 255..0 until SHA-256(seeds || [bump] || program_id ||
/// "ProgramDerivedAddress") produces a point NOT on the ed25519 curve.
pub fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> Option<([u8; 32], u8)> {
    for bump in (0u8..=255).rev() {
        if let Some(addr) = create_program_address(seeds, &[bump], program_id) {
            return Some((addr, bump));
        }
    }
    None
}

/// Attempt to create a program address with explicit bump seed.
/// Returns `None` if the resulting hash IS on the ed25519 curve.
fn create_program_address(
    seeds: &[&[u8]],
    bump_seed: &[u8],
    program_id: &[u8; 32],
) -> Option<[u8; 32]> {
    let mut hasher = Sha256::new();
    for seed in seeds {
        hasher.update(seed);
    }
    hasher.update(bump_seed);
    hasher.update(program_id);
    hasher.update(b"ProgramDerivedAddress");
    let hash: [u8; 32] = hasher.finalize().into();

    if is_on_curve(&hash) {
        return None;
    }
    Some(hash)
}

/// Check whether 32 bytes represent a valid compressed ed25519 point.
fn is_on_curve(bytes: &[u8; 32]) -> bool {
    let compressed = curve25519_dalek::edwards::CompressedEdwardsY(*bytes);
    compressed.decompress().is_some()
}

// ---------------------------------------------------------------------------
// Base58 helpers
// ---------------------------------------------------------------------------

/// Decode a base58-encoded Solana address to 32 bytes.
pub fn decode_pubkey(s: &str) -> Result<[u8; 32], String> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| format!("Invalid base58: {}", e))?;
    if bytes.len() != 32 {
        return Err(format!("Expected 32 bytes, got {}", bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Encode 32 bytes as a base58 Solana address.
pub fn encode_pubkey(bytes: &[u8; 32]) -> String {
    bs58::encode(bytes).into_string()
}

// ---------------------------------------------------------------------------
// Solana JSON-RPC client
// ---------------------------------------------------------------------------

/// Default RPC endpoint (mainnet-beta).
pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";

/// Devnet RPC endpoint.
#[allow(dead_code)]
pub const DEVNET_RPC_URL: &str = "https://api.devnet.solana.com";

/// Resolved RPC endpoint: checks `SOLANA_RPC_URL` env, then falls back.
pub fn resolve_rpc_url(cli_override: Option<&str>) -> String {
    if let Some(url) = cli_override {
        return url.to_string();
    }
    if let Ok(url) = std::env::var("SOLANA_RPC_URL") {
        if !url.is_empty() {
            return url;
        }
    }
    DEFAULT_RPC_URL.to_string()
}

/// Account data returned from `getAccountInfo`.
#[allow(dead_code)]
pub struct AccountInfo {
    /// Raw account data bytes.
    pub data: Vec<u8>,
    /// Account owner (base58).
    pub owner: String,
    /// Lamport balance.
    pub lamports: u64,
    /// Whether the account is executable.
    pub executable: bool,
}

/// Fetch account info from a Solana JSON-RPC endpoint.
///
/// Returns `None` if the account does not exist (value is null).
pub fn get_account_info(rpc_url: &str, pubkey: &str) -> Result<Option<AccountInfo>, String> {
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":["{pubkey}",{{"encoding":"base64"}}]}}"#
    );

    let resp = ureq::post(rpc_url)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| format!("RPC request failed: {}", e))?;

    let text = resp
        .into_string()
        .map_err(|e| format!("Failed to read RPC response: {}", e))?;

    // Check for JSON-RPC error
    if let Some(err_msg) = extract_rpc_error(&text) {
        return Err(format!("RPC error: {}", err_msg));
    }

    // Check if value is null (account doesn't exist)
    if is_value_null(&text) {
        return Ok(None);
    }

    // Parse the response
    let data_b64 = extract_account_data_base64(&text)?;
    let owner = extract_account_owner(&text)?;
    let lamports = extract_account_lamports(&text)?;
    let executable = extract_account_executable(&text)?;

    let data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &data_b64,
    )
    .map_err(|e| format!("Base64 decode error: {}", e))?;

    Ok(Some(AccountInfo {
        data,
        owner,
        lamports,
        executable,
    }))
}

// ---------------------------------------------------------------------------
// Minimal JSON response parsing (no serde dependency)
// ---------------------------------------------------------------------------

/// Check if the RPC response contains an error object.
fn extract_rpc_error(json: &str) -> Option<String> {
    // Look for "error": { "message": "..." }
    let error_key = "\"error\"";
    let pos = json.find(error_key)?;
    let after = &json[pos + error_key.len()..];
    let after = after.trim_start().strip_prefix(':')?;
    let after = after.trim_start();
    if !after.starts_with('{') {
        return None;
    }
    // Extract message
    let msg_key = "\"message\"";
    let msg_pos = after.find(msg_key)?;
    let after_msg = &after[msg_pos + msg_key.len()..];
    let after_msg = after_msg.trim_start().strip_prefix(':')?;
    let after_msg = after_msg.trim_start().strip_prefix('"')?;
    let end = after_msg.find('"')?;
    Some(after_msg[..end].to_string())
}

/// Check if "value" is null in the RPC response.
fn is_value_null(json: &str) -> bool {
    // Find "value" key — be careful not to match nested value keys
    if let Some(pos) = json.find("\"value\"") {
        let after = &json[pos + 7..]; // len of "value"
        let after = after.trim_start();
        if let Some(after) = after.strip_prefix(':') {
            let after = after.trim_start();
            return after.starts_with("null");
        }
    }
    false
}

/// Extract the base64-encoded account data from the response.
///
/// Expects: "data": ["<base64>", "base64"]
fn extract_account_data_base64(json: &str) -> Result<String, String> {
    // Find "data" key
    let key = "\"data\"";
    let pos = json.find(key).ok_or("Missing 'data' in response")?;
    let after = &json[pos + key.len()..];
    let after = after.trim_start()
        .strip_prefix(':')
        .ok_or("Expected :")?;
    let after = after.trim_start();

    // data is an array: ["base64string", "base64"]
    if !after.starts_with('[') {
        return Err("Expected array for 'data'".to_string());
    }
    let after = &after[1..]; // skip '['
    let after = after.trim_start();

    // First element is a quoted base64 string
    if !after.starts_with('"') {
        return Err("Expected string in data array".to_string());
    }
    let after = &after[1..]; // skip opening "
    let end = after.find('"').ok_or("Unterminated data string")?;
    Ok(after[..end].to_string())
}

/// Extract the owner field from the response.
fn extract_account_owner(json: &str) -> Result<String, String> {
    // Find "owner" inside "value" object
    let key = "\"owner\"";
    let pos = json.find(key).ok_or("Missing 'owner'")?;
    let after = &json[pos + key.len()..];
    let after = after.trim_start()
        .strip_prefix(':')
        .ok_or("Expected :")?;
    let after = after.trim_start();
    if !after.starts_with('"') {
        return Err("Expected string for owner".to_string());
    }
    let after = &after[1..];
    let end = after.find('"').ok_or("Unterminated owner string")?;
    Ok(after[..end].to_string())
}

/// Extract lamports from the response.
fn extract_account_lamports(json: &str) -> Result<u64, String> {
    let key = "\"lamports\"";
    let pos = json.find(key).ok_or("Missing 'lamports'")?;
    let after = &json[pos + key.len()..];
    let after = after.trim_start()
        .strip_prefix(':')
        .ok_or("Expected :")?;
    let after = after.trim_start();
    let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    after[..end].parse().map_err(|e| format!("Invalid lamports: {}", e))
}

/// Extract executable flag from the response.
fn extract_account_executable(json: &str) -> Result<bool, String> {
    let key = "\"executable\"";
    let pos = json.find(key).ok_or("Missing 'executable'")?;
    let after = &json[pos + key.len()..];
    let after = after.trim_start()
        .strip_prefix(':')
        .ok_or("Expected :")?;
    let after = after.trim_start();
    if after.starts_with("true") {
        Ok(true)
    } else if after.starts_with("false") {
        Ok(false)
    } else {
        Err("Invalid executable value".to_string())
    }
}

// ---------------------------------------------------------------------------
// Manifest account decoding
// ---------------------------------------------------------------------------

use hopper_schema::{MANIFEST_MAGIC, MANIFEST_HEADER_LEN, MANIFEST_COMPRESS_NONE, MANIFEST_COMPRESS_ZLIB};

/// Decoded on-chain manifest payload.
pub struct OnChainManifest {
    /// Wire format version.
    pub version: u32,
    /// Decompressed manifest JSON.
    pub json: String,
}

/// Decode a Hopper manifest from raw account data.
///
/// Validates the magic bytes, reads the header, and decompresses if needed.
pub fn decode_manifest_account(data: &[u8]) -> Result<OnChainManifest, String> {
    if data.len() < MANIFEST_HEADER_LEN {
        return Err(format!(
            "Account too small for manifest header (need {}, got {})",
            MANIFEST_HEADER_LEN,
            data.len()
        ));
    }

    // Check magic
    if data[0..8] != MANIFEST_MAGIC {
        return Err("Not a Hopper manifest account (bad magic bytes)".to_string());
    }

    // Read header fields
    let version = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let data_len = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;
    let compression = data[16];

    let payload_start = MANIFEST_HEADER_LEN;
    let payload_end = payload_start + data_len;

    if payload_end > data.len() {
        return Err(format!(
            "Manifest payload extends beyond account data ({} + {} > {})",
            payload_start, data_len, data.len()
        ));
    }

    let payload = &data[payload_start..payload_end];

    let json = match compression {
        MANIFEST_COMPRESS_NONE => {
            String::from_utf8(payload.to_vec())
                .map_err(|e| format!("Invalid UTF-8 in manifest: {}", e))?
        }
        MANIFEST_COMPRESS_ZLIB => {
            use flate2::read::ZlibDecoder;
            use std::io::Read;
            let mut decoder = ZlibDecoder::new(payload);
            let mut decompressed = String::new();
            decoder
                .read_to_string(&mut decompressed)
                .map_err(|e| format!("Zlib decompression failed: {}", e))?;
            decompressed
        }
        other => {
            return Err(format!("Unknown compression type: {}", other));
        }
    };

    Ok(OnChainManifest { version, json })
}

/// Encode a manifest JSON into the on-chain account format.
///
/// Returns the full account data bytes (header + payload).
#[cfg(test)]
fn encode_manifest_account(json: &str, compress: bool) -> Vec<u8> {
    let (compression_tag, payload) = if compress {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json.as_bytes()).expect("zlib write failed");
        let compressed = encoder.finish().expect("zlib finish failed");
        (MANIFEST_COMPRESS_ZLIB, compressed)
    } else {
        (MANIFEST_COMPRESS_NONE, json.as_bytes().to_vec())
    };

    let mut buf = Vec::with_capacity(MANIFEST_HEADER_LEN + payload.len());
    buf.extend_from_slice(&MANIFEST_MAGIC);
    buf.extend_from_slice(&1u32.to_le_bytes()); // version
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes()); // data_len
    buf.push(compression_tag);
    buf.extend_from_slice(&[0u8; 3]); // reserved
    buf.extend_from_slice(&payload);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pda_derivation_deterministic() {
        // Known program ID (system program = all zeros doesn't work well,
        // use a fake one)
        let program_id = [1u8; 32];
        let (addr, bump) = find_program_address(&[b"hopper:manifest"], &program_id)
            .expect("PDA derivation failed");
        // Must produce a valid off-curve point
        assert!(!is_on_curve(&addr));
        // Bump is already u8, so always <= 255
        let _ = bump;
        // Same inputs produce same output
        let (addr2, bump2) = find_program_address(&[b"hopper:manifest"], &program_id).unwrap();
        assert_eq!(addr, addr2);
        assert_eq!(bump, bump2);
    }

    #[test]
    fn encode_decode_manifest_roundtrip() {
        let json = r#"{"name":"test","version":"0.1.0"}"#;
        let encoded = encode_manifest_account(json, false);
        let decoded = decode_manifest_account(&encoded).unwrap();
        assert_eq!(decoded.json, json);
        assert_eq!(decoded.version, 1);
    }

    #[test]
    fn encode_decode_manifest_compressed() {
        let json = r#"{"name":"test_program","version":"0.1.0","layouts":[]}"#;
        let encoded = encode_manifest_account(json, true);
        let decoded = decode_manifest_account(&encoded).unwrap();
        assert_eq!(decoded.json, json);
    }

    #[test]
    fn decode_manifest_bad_magic() {
        let mut data = encode_manifest_account("{}", false);
        data[0] = 0xFF; // corrupt magic
        assert!(decode_manifest_account(&data).is_err());
    }

    #[test]
    fn decode_manifest_too_short() {
        let data = vec![0u8; 10];
        assert!(decode_manifest_account(&data).is_err());
    }

    #[test]
    fn pubkey_roundtrip() {
        let bytes = [42u8; 32];
        let encoded = encode_pubkey(&bytes);
        let decoded = decode_pubkey(&encoded).unwrap();
        assert_eq!(bytes, decoded);
    }

    #[test]
    fn is_value_null_detection() {
        let null_resp = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1},"value":null},"id":1}"#;
        assert!(is_value_null(null_resp));

        let non_null = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1},"value":{"data":["","base64"]}},"id":1}"#;
        assert!(!is_value_null(non_null));
    }
}
