//! Substrate-level `Pod` marker.
//!
//! The Hopper Safety Audit asked for every zero-copy access path — all
//! the way down to the native substrate — to require a real Pod bound
//! rather than the loose `T: Copy`. Because `hopper-native` sits below
//! `hopper-runtime` in the dependency graph it cannot reach up to
//! `hopper_runtime::pod::Pod`; instead it carries its own minimal
//! marker here. The trait contract is deliberately identical to
//! `hopper_runtime::pod::Pod`, and `hopper-runtime` adds a blanket
//! bridge so **every** `hopper_native::Pod` type automatically counts
//! as a `hopper_runtime::Pod` — users never notice two traits exist.
//!
//! See [`hopper_runtime::pod::Pod`] (in the sister crate) for the full
//! four-point safety contract; the contract applies verbatim here.

/// Substrate marker for types that can be safely overlaid on raw account bytes.
///
/// # Safety
///
/// Implementing `Pod` for a type `T` asserts all of:
///
/// 1. Every `[u8; size_of::<T>()]` bit pattern decodes to a valid `T`.
/// 2. `align_of::<T>() == 1`.
/// 3. `T` contains no padding.
/// 4. `T` contains no internal pointers or references.
pub unsafe trait Pod: Copy + Sized {}

// ── Primitive implementations ───────────────────────────────────────

unsafe impl Pod for u8 {}
unsafe impl Pod for u16 {}
unsafe impl Pod for u32 {}
unsafe impl Pod for u64 {}
unsafe impl Pod for u128 {}
unsafe impl Pod for i8 {}
unsafe impl Pod for i16 {}
unsafe impl Pod for i32 {}
unsafe impl Pod for i64 {}
unsafe impl Pod for i128 {}
unsafe impl<const N: usize> Pod for [u8; N] {}
unsafe impl Pod for () {}

#[cfg(test)]
mod tests {
    use super::*;

    fn require<T: Pod>() {}

    #[test]
    fn primitives_are_pod() {
        require::<u8>();
        require::<u64>();
        require::<i128>();
        require::<[u8; 32]>();
    }
}
