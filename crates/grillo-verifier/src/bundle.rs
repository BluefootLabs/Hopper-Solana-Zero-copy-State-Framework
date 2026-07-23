//! Offline evidence bundles: the `grillo` CLI's input format.
//!
//! A bundle carries everything an INDEPENDENT party needs to reproduce a
//! byte-precise verdict for one instruction invocation, with no RPC and no
//! trust in the producer beyond the evidence itself:
//!
//! ```json
//! {
//!   "instruction": "execute_intent",
//!   "argumentPayload": "0300",
//!   "touchMap": "7a01000202...",
//!   "accounts": [
//!     { "index": 2, "pre": "<hex>", "post": "<hex>",
//!       "preLamports": 1000, "postLamports": 1000 }
//!   ]
//! }
//! ```
//!
//! - `argumentPayload` is the post-discriminator instruction payload,
//!   required when the instruction publishes parametric exact-cell rules
//!   (the resolver decodes selectors from it; see `EFFECT_ABI_V0_1.md`).
//! - `touchMap` is the program's emitted touch-map wire blob (`hopper tx
//!   explain` prints it; test harnesses capture it from `Program data:`).
//! - `accounts` carries pre/post data snapshots by positional index.
//!   Lamport balances are attached as a PAIR or not at all — an omitted
//!   pair means "lamports unobserved", never "observed 0 -> 0".
//!
//! All byte blobs are lowercase/uppercase hex; hex keeps the format
//! dependency-free and diff-able in fixtures.

use serde::Deserialize;

use crate::touch_map::decode_touch_map;
use crate::verify::{verify, verify_invocation, AccountDelta, Verdict};
use grillo_manifest::{MutationManifest, ResolveError};

/// One account's evidence in a bundle.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleAccount {
    /// Positional index in the instruction's account list.
    pub index: u8,
    /// Hex account data before the instruction.
    pub pre: String,
    /// Hex account data after the instruction.
    pub post: String,
    /// Lamports before; attach together with [`post_lamports`](Self::post_lamports).
    #[serde(rename = "preLamports", default)]
    pub pre_lamports: Option<u64>,
    /// Lamports after.
    #[serde(rename = "postLamports", default)]
    pub post_lamports: Option<u64>,
}

/// A complete offline evidence bundle for one invocation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBundle {
    /// Instruction name in the manifest.
    pub instruction: String,
    /// Hex post-discriminator argument payload (required for parametric
    /// instructions; ignored by resolution-free ones when absent).
    #[serde(rename = "argumentPayload", default)]
    pub argument_payload: Option<String>,
    /// Hex touch-map wire blob as emitted by the program.
    #[serde(rename = "touchMap")]
    pub touch_map: String,
    /// Pre/post account evidence.
    pub accounts: Vec<BundleAccount>,
}

/// Why a bundle could not be verified (distinct from a byte verdict —
/// a malformed bundle is never a PASS and never a VIOLATION).
#[derive(Debug)]
pub enum BundleError {
    Json(String),
    Hex { field: String, message: String },
    UnknownInstruction(String),
    TouchMap(String),
    LamportsHalfObserved { index: u8 },
    Resolve(ResolveError),
}

impl core::fmt::Display for BundleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BundleError::Json(m) => write!(f, "bundle JSON error: {m}"),
            BundleError::Hex { field, message } => {
                write!(f, "bundle field `{field}` is not valid hex: {message}")
            }
            BundleError::UnknownInstruction(name) => {
                write!(f, "manifest has no instruction named `{name}`")
            }
            BundleError::TouchMap(m) => write!(f, "touch map blob rejected: {m}"),
            BundleError::LamportsHalfObserved { index } => write!(
                f,
                "account {index}: preLamports/postLamports must be provided together or not at all"
            ),
            BundleError::Resolve(e) => write!(f, "parametric resolution failed: {e}"),
        }
    }
}

impl std::error::Error for BundleError {}

fn decode_hex(field: &str, s: &str) -> Result<Vec<u8>, BundleError> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err(BundleError::Hex {
            field: field.to_string(),
            message: "odd number of hex digits".to_string(),
        });
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let nibble = |c: u8| -> Result<u8, BundleError> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            other => Err(BundleError::Hex {
                field: field.to_string(),
                message: format!("invalid hex character `{}`", other as char),
            }),
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        out.push((nibble(bytes[i])? << 4) | nibble(bytes[i + 1])?);
        i += 2;
    }
    Ok(out)
}

/// Parse a bundle from JSON.
pub fn parse_bundle(json: &str) -> Result<EvidenceBundle, BundleError> {
    serde_json::from_str(json).map_err(|e| BundleError::Json(e.to_string()))
}

/// Verify a parsed bundle against a parsed manifest and return the byte
/// verdict. Fail-closed: any malformed evidence is a [`BundleError`], and
/// a parametric instruction without `argumentPayload` yields the verifier's
/// own INCONCLUSIVE (never a false PASS against the static envelope).
pub fn verify_bundle(
    manifest: &MutationManifest,
    bundle: &EvidenceBundle,
) -> Result<Verdict, BundleError> {
    let contract = manifest
        .instruction(&bundle.instruction)
        .ok_or_else(|| BundleError::UnknownInstruction(bundle.instruction.clone()))?;

    let map_bytes = decode_hex("touchMap", &bundle.touch_map)?;
    let touch_map =
        decode_touch_map(&map_bytes).map_err(|e| BundleError::TouchMap(format!("{e:?}")))?;

    // Decode account evidence up front so deltas can borrow the buffers.
    let mut decoded: Vec<(u8, Vec<u8>, Vec<u8>, Option<(u64, u64)>)> =
        Vec::with_capacity(bundle.accounts.len());
    for account in &bundle.accounts {
        let pre = decode_hex(&format!("accounts[{}].pre", account.index), &account.pre)?;
        let post = decode_hex(&format!("accounts[{}].post", account.index), &account.post)?;
        let lamports = match (account.pre_lamports, account.post_lamports) {
            (Some(a), Some(b)) => Some((a, b)),
            (None, None) => None,
            _ => {
                return Err(BundleError::LamportsHalfObserved {
                    index: account.index,
                })
            }
        };
        decoded.push((account.index, pre, post, lamports));
    }
    let deltas: Vec<AccountDelta<'_>> = decoded
        .iter()
        .map(|(index, pre, post, lamports)| {
            let delta = AccountDelta::new(*index, pre, post);
            match lamports {
                Some((a, b)) => delta.with_lamports(*a, *b),
                None => delta,
            }
        })
        .collect();

    match &bundle.argument_payload {
        Some(payload_hex) => {
            let payload = decode_hex("argumentPayload", payload_hex)?;
            verify_invocation(contract, &payload, &deltas, &touch_map).map_err(BundleError::Resolve)
        }
        None => Ok(verify(contract, &deltas, &touch_map)),
    }
}
