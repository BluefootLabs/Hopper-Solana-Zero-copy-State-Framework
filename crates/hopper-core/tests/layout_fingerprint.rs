//! Layout fingerprints are persisted ABI, so this test is intentionally
//! independent of every optional `hopper-systems` feature.

#[test]
fn layout_id_matches_sha256_without_feature_dependence() {
    let hash = hopper_core::__sha256_const(b"hopper:v1:Test:1:field_a:WireU64:8,");
    assert_eq!(
        &hash[..8],
        &[0xf6, 0x9f, 0x84, 0xb5, 0x8d, 0xde, 0x9e, 0x9c]
    );
}
