//! A user-authored type that bypasses the `#[hopper::pod]` macro must
//! NOT get `ZeroCopy` for free. The blanket `impl ZeroCopy for T`
//! requires the sealed `__sealed::HopperZeroCopySealed` marker, which
//! only Hopper-authored surfaces stamp. Calling a function bounded by
//! `T: ZeroCopy` with such a bypass type must fail at compile time.
//!
//! This closes Hopper Safety Audit final-API Step 5: "you cannot
//! implement `ZeroCopy` manually, only via the macro."

use hopper::hopper_runtime::ZeroCopy;

#[derive(Copy, Clone)]
#[repr(C)]
pub struct SneakyBypass {
    pub x: u64,
}

fn require_zero_copy<T: ZeroCopy>() {}

fn main() {
    // Expected compile failure: `SneakyBypass` does not stamp
    // `HopperZeroCopySealed`, so the `ZeroCopy` blanket cannot apply.
    require_zero_copy::<SneakyBypass>();
}
