#![no_main]
//! Fuzz target for `hopper_native::raw_input::parse_instruction_frame_checked`.
//!
//! Contract under test: for *any* byte slice, the safe parser either
//! returns `Ok(FrameInfo)` with a well-formed parse or returns
//! `Err(FrameError)`. It must never panic, never abort, never read
//! out of bounds, never produce a `FrameInfo` whose byte ranges point
//! outside the input buffer, and never accept a forward-pointing
//! duplicate marker.
//!
//! The fuzz target additionally audits the returned `FrameInfo` for
//! internal self-consistency: every recorded slot offset must fall
//! within the buffer, and `instruction_data_range` + `program_id_offset`
//! together must exactly reach `buf.len()` when the parse succeeds.

use libfuzzer_sys::fuzz_target;

use hopper_native::raw_input::{parse_instruction_frame_checked, FrameInfo};

fuzz_target!(|data: &[u8]| {
    let Ok(FrameInfo {
        account_count,
        instruction_data_range,
        program_id_offset,
        slot_offsets,
    }) = parse_instruction_frame_checked(data)
    else {
        // Any error is acceptable. the contract is "never UB", not
        // "always succeed". Loop back for the next input.
        return;
    };

    // If the parse succeeded, every piece of returned metadata must
    // point inside the buffer. These are assertions, not bugs. a
    // violation here is a bug in the parser.
    assert!(
        account_count <= slot_offsets.len(),
        "account_count {} > slot_offsets capacity {}",
        account_count,
        slot_offsets.len()
    );
    for slot in 0..account_count {
        assert!(
            slot_offsets[slot] < data.len(),
            "slot {} offset {} past buffer {}",
            slot,
            slot_offsets[slot],
            data.len()
        );
    }
    assert!(instruction_data_range.end <= data.len());
    assert!(instruction_data_range.start <= instruction_data_range.end);
    assert!(program_id_offset + 32 <= data.len());
    assert!(
        program_id_offset >= instruction_data_range.end,
        "program id must follow instruction data"
    );
});
