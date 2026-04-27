//! Field-level layout descriptors for inspectable state contracts.
//!
//! Runtime-owned field maps let Hopper tie typed account access, runtime
//! inspection, and schema export back to the same metadata source.

/// Descriptor for one field in a Hopper layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldInfo {
    /// Human-readable field name.
    pub name: &'static str,
    /// Byte offset from the start of the account data.
    pub offset: usize,
    /// Field size in bytes.
    pub size: usize,
}

impl FieldInfo {
    #[inline(always)]
    pub const fn new(name: &'static str, offset: usize, size: usize) -> Self {
        Self { name, offset, size }
    }

    #[inline(always)]
    pub const fn end(&self) -> usize {
        self.offset + self.size
    }

    #[inline(always)]
    pub fn read_bytes<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        if data.len() >= self.end() {
            Some(&data[self.offset..self.end()])
        } else {
            None
        }
    }
}

/// Trait for layouts that expose field metadata in wire order.
pub trait FieldMap {
    const FIELDS: &'static [FieldInfo];

    #[inline(always)]
    fn field_count() -> usize {
        Self::FIELDS.len()
    }

    #[inline]
    fn field_by_name(name: &str) -> Option<&'static FieldInfo> {
        let mut index = 0;
        while index < Self::FIELDS.len() {
            let field = &Self::FIELDS[index];
            if str_eq(field.name, name) {
                return Some(field);
            }
            index += 1;
        }
        None
    }
}

#[inline(always)]
fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut index = 0;
    while index < a.len() {
        if a[index] != b[index] {
            return false;
        }
        index += 1;
    }
    true
}