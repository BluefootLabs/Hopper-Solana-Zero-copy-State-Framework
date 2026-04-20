//! `bool` is not a valid Pod field: not every byte pattern maps to
//! a valid `bool` (only `0` and `1` are valid). The field-level
//! `__FieldPodProof<T: bytemuck::Pod + Zeroable>` marker instantiated
//! inside the expanded `#[hopper::pod]` output must reject this.

use hopper::pod;

#[pod]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct BadBool {
    pub flag: bool,
}

fn main() {}
