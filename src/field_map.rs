//! Field-level layout descriptors for inspectable state contracts.
//!
//! `FieldInfo` and `FieldMap` provide compile-time field metadata that
//! enables manager account inspection, layout visualization, schema
//! export, upgrade safety checks, and client reflection.
//!
//! Unlike runtime data, field maps are pure metadata -- they describe
//! *where* fields live in the wire format, not their values.

/// Descriptor for a single field in a Hopper layout.
///
/// Each entry records the field's name, byte offset within the layout
/// (relative to the 16-byte header), and wire size. This is sufficient
/// for tools to locate, read, and display any field without the Rust type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldInfo {
    /// Human-readable field name (e.g. "authority", "balance").
    pub name: &'static str,
    /// Byte offset from the start of the account data (including header).
    pub offset: usize,
    /// Wire size of the field in bytes.
    pub size: usize,
}

impl FieldInfo {
    /// Construct a new field descriptor.
    #[inline(always)]
    pub const fn new(name: &'static str, offset: usize, size: usize) -> Self {
        Self { name, offset, size }
    }

    /// The exclusive end offset (offset + size).
    #[inline(always)]
    pub const fn end(&self) -> usize {
        self.offset + self.size
    }

    /// Read the raw bytes of this field from an account data slice.
    ///
    /// Returns `None` if the data is too short.
    #[inline(always)]
    pub fn read_bytes<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        if data.len() >= self.end() {
            Some(&data[self.offset..self.end()])
        } else {
            None
        }
    }
}

/// Trait for layout types that expose their field map for inspection.
///
/// Implementing this trait makes a layout fully inspectable at runtime
/// without requiring the concrete Rust type. This powers:
/// - Manager account inspection and field-level display
/// - Schema export to JSON/IDL
/// - Layout diff and upgrade safety checks
/// - Client code generation with field offsets
pub trait FieldMap {
    /// Complete list of fields in wire order.
    const FIELDS: &'static [FieldInfo];

    /// Total number of fields.
    #[inline(always)]
    fn field_count() -> usize {
        Self::FIELDS.len()
    }

    /// Look up a field by name.
    #[inline]
    fn field_by_name(name: &str) -> Option<&'static FieldInfo> {
        let mut i = 0;
        while i < Self::FIELDS.len() {
            // Const-compatible string comparison
            let f = &Self::FIELDS[i];
            if str_eq(f.name, name) {
                return Some(f);
            }
            i += 1;
        }
        None
    }
}

/// Byte-level string equality (for no_std field lookup).
#[inline(always)]
fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
