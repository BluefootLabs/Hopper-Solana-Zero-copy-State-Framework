#![no_main]
//! Fuzz target for `hopper_schema::decode_header`.
//!
//! Contract: for any byte slice, `decode_header` returns
//! `Some(DecodedHeader)` with fields whose byte spans lie entirely
//! within the input, or `None`. Never panics, never OOBs.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hopper_schema::decode_header(data);
});
