use sha2::{Digest, Sha256};

use crate::{TopologyError, WorkloadProfile, PROFILE_SCHEMA_V1};

const PROFILE_DOMAIN: &[u8] = b"hopper.topology.profile.v0.1\0";
pub(crate) const PLAN_DOMAIN: &[u8] = b"hopper.topology.plan.v0.1\0";

pub fn sha256_hex(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Commit a profile after semantic normalization. Vector ordering and duplicate
/// read entries therefore cannot alter the commitment.
pub fn commit_profile(profile: &WorkloadProfile) -> Result<String, TopologyError> {
    if profile.schema != PROFILE_SCHEMA_V1 {
        return Err(TopologyError::new(format!(
            "unsupported profile schema `{}` (expected `{PROFILE_SCHEMA_V1}`)",
            profile.schema
        )));
    }
    let normalized = crate::solver::normalize_and_validate(profile.clone())?;
    let bytes = serde_json::to_vec(&normalized)
        .map_err(|error| TopologyError::new(format!("serialize normalized profile: {error}")))?;
    Ok(sha256_hex(PROFILE_DOMAIN, &bytes))
}
