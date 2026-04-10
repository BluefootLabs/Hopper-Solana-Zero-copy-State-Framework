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

mod header;
mod pod;
mod overlay;
mod lifecycle;
mod cursor;
mod reader;
mod verified;
mod segment;
mod dynamic;
mod realloc_guard;
pub mod registry;
pub mod segment_role;

pub use cursor::{DataWriter, SliceCursor};
pub use header::{
    check_header, read_discriminator, read_header_flags, read_layout_id, read_version, write_header,
    AccountHeader, HEADER_FORMAT, HEADER_LEN,
};
pub use lifecycle::{
    safe_close, safe_close_with_sentinel, safe_realloc, zero_init, CLOSE_SENTINEL,
};
pub use overlay::{overlay, overlay_mut};
pub use pod::{
    pod_from_bytes, pod_from_bytes_mut, pod_read, pod_write,
    cast_unchecked, cast_unchecked_mut,
    FixedLayout, Pod,
};
pub use reader::AccountReader;
pub use segment::{
    SegmentDescriptor, SegmentSlice, SegmentSliceMut, SegmentTable, SegmentTableMut,
    MAX_SEGMENTS, SEGMENT_DESC_SIZE,
};
pub use verified::{VerifiedAccount, VerifiedAccountMut};
pub use realloc_guard::ReallocGuard;
pub use dynamic::{
    read_dynamic_u8, read_dynamic_u16, read_dynamic_u32,
    write_dynamic_u8, write_dynamic_u16, write_dynamic_u32,
    DynamicView, DynamicViewMut,
};
pub use registry::{
    SegmentRegistry, SegmentRegistryMut, SegmentEntry, SegmentId,
    segment_id, SEGMENT_ENTRY_SIZE, REGISTRY_HEADER_SIZE,
    MAX_REGISTRY_SEGMENTS, REGISTRY_OFFSET,
    SEG_FLAG_LOCKED, SEG_FLAG_FROZEN, SEG_FLAG_DYNAMIC,
};
pub use segment_role::{
    SegmentRole,
    SEG_ROLE_CORE, SEG_ROLE_EXTENSION, SEG_ROLE_JOURNAL,
    SEG_ROLE_INDEX, SEG_ROLE_CACHE, SEG_ROLE_AUDIT, SEG_ROLE_SHARD,
};
