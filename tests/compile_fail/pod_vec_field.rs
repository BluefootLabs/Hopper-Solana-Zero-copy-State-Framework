//! `Vec<u8>` carries a heap pointer and a length, neither of which are
//! valid Pod. The `#[hopper::pod]` macro must reject it via the
//! field-level `__FieldPodProof<T: bytemuck::Pod + Zeroable>` marker.
//! Any struct body that lands a heap-allocated field in a zero-copy
//! layout would corrupt the account on every load.

extern crate alloc;

use hopper::pod;

#[pod]
#[derive(Clone)]
#[repr(C)]
pub struct BadHeap {
    pub tail: alloc::vec::Vec<u8>,
}

fn main() {}
