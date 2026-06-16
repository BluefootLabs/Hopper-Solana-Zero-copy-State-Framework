use hopper::hopper_core::prelude_core::{FixedLayout, Pod, VerifiedAccount};

#[repr(C)]
#[derive(Clone, Copy)]
struct Tiny {
    value: u8,
}

unsafe impl hopper::hopper_runtime::Zeroable for Tiny {}
unsafe impl Pod for Tiny {}

impl FixedLayout for Tiny {
    const SIZE: usize = 1;
}

fn leak_verified_ref<'a>() -> &'a Tiny {
    let data = [0u8; 1];
    let verified = VerifiedAccount::<Tiny>::new(&data).unwrap();
    verified.get()
}

fn main() {}
