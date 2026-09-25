use hopper::hopper_core::account::{FixedLayout, Pod, Zeroable};

#[repr(transparent)]
#[derive(Clone, Copy)]
struct Bytes([u8; 8]);
// SAFETY: Byte array wrapper, alignment 1, all patterns valid, no padding.
unsafe impl Zeroable for Bytes {}
// SAFETY: Byte array wrapper, alignment 1, all patterns valid, no padding.
unsafe impl Pod for Bytes {}
impl FixedLayout for Bytes {
    #[cfg(any(feature = "pod", feature = "verified", feature = "collection"))]
    const SIZE: usize = 1;
    #[cfg(feature = "event")]
    const SIZE: usize = 16;
    const _SIZE_IS_HONEST: () = (); // Deliberately spoof the overridable constant.
}

fn main() {
    use hopper::hopper_core::account::{pod_from_bytes, VerifiedAccount};
    #[cfg(not(any(
        feature = "pod",
        feature = "verified",
        feature = "collection",
        feature = "event"
    )))]
    {
        assert_eq!(pod_from_bytes::<Bytes>(&[0; 8]).unwrap().0, [0; 8]);
        assert_eq!(
            VerifiedAccount::<Bytes>::new(&[0; 8]).unwrap().get().0,
            [0; 8]
        );
    }
    #[cfg(feature = "pod")]
    {
        std::hint::black_box(pod_from_bytes::<Bytes>(&[0; 1]));
    }
    #[cfg(feature = "verified")]
    {
        std::hint::black_box(VerifiedAccount::<Bytes>::new(&[0; 1]));
    }
    #[cfg(feature = "collection")]
    {
        std::hint::black_box(
            hopper::hopper_core::collections::FixedVec::<Bytes>::from_bytes(&mut [0; 5]),
        );
    }
    #[cfg(feature = "event")]
    {
        std::hint::black_box(hopper::hopper_core::event::emit_event(&Bytes([0; 8])));
    }
}
