//! The raw memory primitives (`hopper_native::mem`) off chain, where they
//! run the `core::ptr` equivalents of the syscalls. `docs/UNSAFE_INVARIANTS.md`
//! cites these tests by name.

use core::cmp::Ordering;
use hopper_native::mem;

#[test]
fn memcpy_copies_exactly_n_bytes() {
    let src: [u8; 16] = core::array::from_fn(|i| i as u8 + 1);
    let mut dst = [0xAAu8; 16];
    // SAFETY: both regions are valid for 8 bytes and do not overlap.
    unsafe { mem::memcpy(dst.as_mut_ptr().add(4), src.as_ptr(), 8) };
    assert_eq!(dst[..4], [0xAA; 4]);
    assert_eq!(dst[4..12], src[..8]);
    assert_eq!(dst[12..], [0xAA; 4]);
    // A zero length touches nothing.
    // SAFETY: a zero-length copy reads and writes no byte.
    unsafe { mem::memcpy(dst.as_mut_ptr(), src.as_ptr(), 0) };
    assert_eq!(dst[0], 0xAA);
}

#[test]
fn memmove_is_correct_when_the_regions_overlap() {
    for (from, to) in [(0usize, 3usize), (3, 0)] {
        let mut buf: [u8; 16] = core::array::from_fn(|i| i as u8);
        let expected: [u8; 8] = core::array::from_fn(|i| (from + i) as u8);
        // SAFETY: both ranges lie inside `buf`; `memmove` permits overlap.
        unsafe { mem::memmove(buf.as_mut_ptr().add(to), buf.as_ptr().add(from), 8) };
        assert_eq!(buf[to..to + 8], expected, "from {from} to {to}");
    }
}

#[test]
fn memset_fills_exactly_n_bytes() {
    let mut buf = [0x11u8; 16];
    // SAFETY: the range lies inside `buf`.
    unsafe { mem::memset(buf.as_mut_ptr().add(2), 0, 10) };
    assert_eq!(buf[..2], [0x11; 2]);
    assert_eq!(buf[2..12], [0; 10]);
    assert_eq!(buf[12..], [0x11; 4]);
}

#[test]
fn memcmp_orders_by_the_first_differing_byte() {
    let a = [1u8, 2, 3, 4];
    let lower = [1u8, 2, 2, 9];
    let higher = [1u8, 2, 4, 0];
    // SAFETY: every pointer is valid for the compared length.
    unsafe {
        assert_eq!(mem::memcmp(a.as_ptr(), a.as_ptr(), 4), Ordering::Equal);
        assert_eq!(
            mem::memcmp(a.as_ptr(), lower.as_ptr(), 4),
            Ordering::Greater
        );
        assert_eq!(mem::memcmp(a.as_ptr(), higher.as_ptr(), 4), Ordering::Less);
        // Bytes past `n` do not take part.
        assert_eq!(mem::memcmp(a.as_ptr(), lower.as_ptr(), 2), Ordering::Equal);
        assert_eq!(mem::memcmp(a.as_ptr(), lower.as_ptr(), 0), Ordering::Equal);
    }
}

#[test]
fn the_safe_wrappers_check_lengths() {
    let mut dst = [0u8; 4];
    assert!(mem::copy_bytes(&mut dst, &[1, 2, 3, 4]).is_ok());
    assert_eq!(dst, [1, 2, 3, 4]);
    // A source longer than the destination is refused and writes nothing.
    assert!(mem::copy_bytes(&mut dst, &[9, 9, 9, 9, 9]).is_err());
    assert_eq!(dst, [1, 2, 3, 4]);
    // A shorter source fills the front and leaves the rest.
    assert!(mem::copy_bytes(&mut dst, &[7, 7]).is_ok());
    assert_eq!(dst, [7, 7, 3, 4]);
    dst = [1, 2, 3, 4];
    assert!(mem::bytes_eq(&dst, &[1, 2, 3, 4]));
    assert!(!mem::bytes_eq(&dst, &[1, 2, 3]));
    assert!(!mem::bytes_eq(&dst, &[1, 2, 3, 5]));
    mem::zero_fill(&mut dst);
    assert_eq!(dst, [0; 4]);
}
