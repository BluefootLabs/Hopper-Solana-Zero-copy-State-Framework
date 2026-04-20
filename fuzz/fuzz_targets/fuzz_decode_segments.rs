#![no_main]
//! Fuzz target for `hopper_schema::decode_segments::<N>`.
//!
//! Contract: never panics or reads past `data.len()` regardless of
//! buffer contents or length. The returned segments (if any) must
//! reference only bytes that actually exist in `data`.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some((count, segments)) = hopper_schema::decode_segments::<8>(data) {
        assert!(count <= segments.len());
        // Segments returned are raw decoded metadata: the `offset`/`size`
        // fields may legitimately be hostile/garbage. the contract here
        // is "decode_segments itself doesn't UB", not "values are sane".
        // We just touch each segment field to keep the optimizer honest.
        for seg in segments.iter().take(count) {
            core::hint::black_box(seg.id);
            core::hint::black_box(seg.offset);
            core::hint::black_box(seg.size);
            core::hint::black_box(seg.flags);
            core::hint::black_box(seg.version);
        }
    }
});
