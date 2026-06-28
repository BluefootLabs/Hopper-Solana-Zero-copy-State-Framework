use hopper::prelude::*;

// A compact layout WITH a dynamic tail: `[disc:u8][fixed_head][tail]`, no
// 16-byte universal header. This combination was previously rejected at
// macro-expansion time; it is now first-class.
#[derive(Clone, Copy)]
#[hopper::state(compact, disc = 8, raw_tail = true)]
#[repr(C)]
pub struct CompactLog {
    pub authority: [u8; 32],
    pub seq: WireU64,
}

fn main() {
    // Fixed-head consts are unchanged from a fixed compact layout.
    const _: usize = CompactLog::BODY_SIZE; // 40
    const _: usize = CompactLog::COMPACT_LEN; // 41
    const _: u8 = CompactLog::DISC; // 8
    const _: bool = CompactLog::HAS_DYNAMIC_TAIL; // true

    // The tail's length prefix sits immediately after the 1-byte disc + fixed
    // head -- NOT after a 16-byte header. That 15-byte saving is the whole
    // point of a compact dynamic account.
    const _: () = assert!(CompactLog::TAIL_PREFIX_OFFSET == CompactLog::COMPACT_LEN);
    const _: () = assert!(CompactLog::TAIL_PREFIX_OFFSET == 41);

    // `space_for_tail` is a const fn: disc + head + 4-byte prefix + capacity.
    const _: () = assert!(CompactLog::space_for_tail(128) == 41 + 4 + 128);

    // It implements the relaxed-length `CompactDynamicLayout`, not the
    // fixed-length `CompactLayout`.
    use hopper::account::CompactDynamicLayout;
    const _: () = assert!(<CompactLog as CompactDynamicLayout>::MIN_LEN == 41);
    const _: () = assert!(<CompactLog as CompactDynamicLayout>::TAIL_OFFSET == 41);

    // The unified descriptor flips its dynamic-tail flag while keeping the
    // single registry/identity model.
    use hopper::manifest::LayoutDescriptor;
    let _entry = <CompactLog as LayoutDescriptor>::registry_entry();
}
