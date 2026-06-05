//! Account memory architecture.
//!
//! Hopper supports four account memory styles:
//!
//! 1. **Fixed Layout** -- Classic zero-copy `#[repr(C)]` overlay with a 16-byte header.
//! 2. **Overlay Layout** -- Multiple typed views over different regions of one account.
//! 3. **Segmented Layout** -- Fixed prefix + dynamic typed segments with a segment table.
//! 4. **Arena Layout** -- Accounts as typed storage arenas (slab allocators, ring buffers).
//!
//! All styles share the same 16-byte header format for self-description.

mod cursor;
mod dynamic;
mod header;
mod lifecycle;
mod overlay;
mod pod;
mod reader;
mod realloc_guard;
pub mod registry;
mod segment;
pub mod segment_role;
mod verified;

pub use cursor::{DataWriter, SliceCursor};
pub use dynamic::{
    read_dynamic_u16, read_dynamic_u32, read_dynamic_u8, write_dynamic_u16, write_dynamic_u32,
    write_dynamic_u8, DynamicView, DynamicViewMut,
};
pub use header::{
    check_header, read_discriminator, read_header_flags, read_layout_id, read_version,
    write_header, AccountHeader, HEADER_FORMAT, HEADER_LEN,
};
pub use lifecycle::{
    safe_close, safe_close_unchecked, safe_close_with_sentinel, safe_close_with_sentinel_unchecked,
    safe_realloc, safe_realloc_unchecked, zero_init, CLOSE_SENTINEL,
};
pub use overlay::{overlay, overlay_mut};
pub use pod::{
    cast_unchecked, cast_unchecked_mut, pod_from_bytes, pod_from_bytes_mut, pod_read, pod_write,
    FixedLayout, Pod,
};
pub use reader::AccountReader;
pub use realloc_guard::ReallocGuard;
pub use registry::{
    segment_id, SegmentEntry, SegmentId, SegmentRegistry, SegmentRegistryMut,
    MAX_REGISTRY_SEGMENTS, REGISTRY_HEADER_SIZE, REGISTRY_OFFSET, SEGMENT_ENTRY_SIZE,
    SEG_FLAG_DYNAMIC, SEG_FLAG_FROZEN, SEG_FLAG_LOCKED,
};
pub use segment::{
    SegmentDescriptor, SegmentSlice, SegmentSliceMut, SegmentTable, SegmentTableMut, MAX_SEGMENTS,
    SEGMENT_DESC_SIZE,
};
pub use segment_role::{
    SegmentRole, SEG_ROLE_AUDIT, SEG_ROLE_CACHE, SEG_ROLE_CORE, SEG_ROLE_EXTENSION, SEG_ROLE_INDEX,
    SEG_ROLE_JOURNAL, SEG_ROLE_SHARD,
};
pub use verified::{VerifiedAccount, VerifiedAccountMut};
