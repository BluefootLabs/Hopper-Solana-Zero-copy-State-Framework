use hopper::hopper_core::account::{pod_read, SegmentDescriptor, SegmentSlice, SegmentSliceMut};
use hopper::prelude::{ProgramError, WireU64};

fn descriptor(offset: u32, count: u16, capacity: u16, size: u16) -> SegmentDescriptor {
    let mut desc = pod_read::<SegmentDescriptor>(&[0; 12]).unwrap();
    desc.set_offset(offset);
    desc.set_count(count);
    desc.set_capacity(capacity);
    desc.set_element_size(size);
    desc
}

#[test]
fn malformed_geometry_is_refused_before_exposing_elements() {
    let mut data = [0u8; 40];
    for (desc, expected) in [
        (descriptor(8, 2, 1, 8), ProgramError::InvalidAccountData),
        (descriptor(8, 1, 2, 4), ProgramError::InvalidAccountData),
        (descriptor(8, 1, 5, 8), ProgramError::AccountDataTooSmall),
        (
            descriptor(u32::MAX, 0, 0, 8),
            ProgramError::AccountDataTooSmall,
        ),
    ] {
        assert!(
            matches!(SegmentSlice::<WireU64>::from_descriptor(&data, &desc), Err(e) if e == expected)
        );
        assert!(
            matches!(SegmentSliceMut::<WireU64>::from_descriptor(&mut data, &desc), Err(e) if e == expected)
        );
        assert_eq!(data, [0; 40]);
    }
}

#[test]
fn valid_slice_mutation_preserves_neighboring_regions() {
    let mut data = [0xabu8; 40];
    data[8..16].copy_from_slice(&42u64.to_le_bytes());
    let desc = descriptor(8, 1, 2, 8);
    {
        let mut slice = SegmentSliceMut::<WireU64>::from_descriptor(&mut data, &desc).unwrap();
        assert_eq!(slice.read(0).unwrap().get(), 42);
        slice.write(0, WireU64::new(43)).unwrap();
        assert!(slice.read(1).is_err());
    }
    assert_eq!(&data[..8], &[0xab; 8]);
    assert_eq!(&data[16..], &[0xab; 24]);
    assert_eq!(
        SegmentSlice::<WireU64>::from_descriptor(&data, &desc)
            .unwrap()
            .read(0)
            .unwrap()
            .get(),
        43
    );
}
