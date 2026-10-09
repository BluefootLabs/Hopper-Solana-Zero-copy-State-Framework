//! Borrowed argument validation follows types through aliases and composition.

#![cfg(feature = "proc-macros")]

use hopper::prelude::*;

#[hopper::unit_enum]
pub enum Mode {
    Open = 1,
    Closed = 7,
}

type ModeByte = EnumByte<Mode>;
type OptionalMode = OptionByte<ModeByte>;

#[hopper::args]
#[repr(C)]
pub struct AliasedArgs {
    pub mode: ModeByte,
    pub optional: OptionalMode,
}

#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Modes {
    pub values: [OptionalMode; 2],
}

#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct WrappedModes(pub Modes);

#[hopper::args(tail, cu = 420)]
#[repr(C)]
pub struct NestedArgs {
    pub modes: WrappedModes,
}

#[hopper::args]
#[repr(C)]
pub struct NestedOptions {
    pub value: OptionByte<OptionByte<ModeByte>>,
}

#[hopper::state(disc = 231, version = 1)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct HeaderedModes {
    pub modes: Modes,
}

#[hopper::state(compact, disc = 232, version = 1)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct CompactModes {
    pub modes: Modes,
}

hopper::hopper_layout! {
    pub struct DeclaredModes, disc = 233, version = 1 {
        modes: Modes = 4,
    }
}

hopper::hopper_interface! {
    pub struct ForeignModes, disc = 233, version = 1 {
        modes: Modes = 4,
    }
}

#[hopper::args]
#[repr(C)]
pub struct LayoutArgs {
    pub headered: HeaderedModes,
    pub compact: CompactModes,
    pub declared: DeclaredModes,
    pub foreign: ForeignModes,
}

#[test]
fn aliases_validate_all_enum_bytes_and_option_tags() {
    for byte in 0..=u8::MAX {
        let bytes = [byte, 0, 255];
        assert_eq!(
            AliasedArgs::parse_checked(&bytes).is_ok(),
            matches!(byte, 1 | 7),
            "enum byte {byte}",
        );
        let bytes = [1, byte, 7];
        assert_eq!(
            AliasedArgs::parse_checked(&bytes).is_ok(),
            byte <= 1,
            "option tag {byte}",
        );
        let bytes = [1, 1, byte];
        assert_eq!(
            AliasedArgs::parse_checked(&bytes).is_ok(),
            matches!(byte, 1 | 7),
            "present payload {byte}",
        );
        // Absent payloads are ignored, including invalid enum encodings.
        assert!(AliasedArgs::parse_checked(&[1, 0, byte]).is_ok());
    }
}

#[test]
fn nested_arrays_and_tuple_structs_validate_every_element() {
    assert_eq!(NestedArgs::PACKED_SIZE, 4);
    assert_eq!(NestedArgs::CU_HINT, 420);
    for index in 0..2 {
        for byte in 0..=u8::MAX {
            let mut bytes = [1, 1, 1, 7];
            bytes[index * 2] = byte;
            assert_eq!(NestedArgs::parse_checked(&bytes).is_ok(), byte <= 1);
            let mut bytes = [1, 1, 1, 7];
            bytes[index * 2 + 1] = byte;
            assert_eq!(
                NestedArgs::parse_checked(&bytes).is_ok(),
                matches!(byte, 1 | 7),
            );
        }
    }
}

#[test]
fn nested_options_only_validate_present_payloads() {
    for bytes in [[0, 255, 255], [1, 0, 255], [1, 1, 7]] {
        assert!(NestedOptions::parse_checked(&bytes).is_ok());
    }
    for bytes in [[2, 0, 0], [1, 2, 7], [1, 1, 255]] {
        assert_eq!(
            NestedOptions::parse_checked(&bytes).err(),
            Some(ProgramError::InvalidInstructionData),
        );
    }
}

#[test]
fn raw_overlays_and_validation_remain_separate() {
    let bytes = [1, 1, 1, 255];
    let args = NestedArgs::parse(&bytes).unwrap();
    assert_eq!(
        Pod::validate_value(&args.modes).map_err(ProgramError::from),
        Err(ProgramError::InvalidAccountData),
    );
    assert_eq!(
        args.validate_values(),
        Err(ProgramError::InvalidInstructionData)
    );
    assert_eq!(args.validate_tags(), args.validate_values());
}

#[test]
fn fixed_payloads_reject_every_truncation_and_trailing_bytes() {
    let bytes = [1, 1, 1, 7, 99];
    for length in 0..NestedArgs::PACKED_SIZE {
        assert!(NestedArgs::parse_checked(&bytes[..length]).is_err());
        assert!(NestedArgs::parse_exact_checked(&bytes[..length]).is_err());
        assert!(NestedArgs::parse_with_tail_checked(&bytes[..length]).is_err());
    }
    assert!(NestedArgs::parse_checked(&bytes).is_ok());
    assert!(NestedArgs::parse_exact_checked(&bytes[..4]).is_ok());
    assert_eq!(
        NestedArgs::parse_exact_checked(&bytes).err(),
        Some(ProgramError::InvalidInstructionData),
    );
}

#[test]
fn checked_prefix_and_tail_borrow_the_original_input_at_any_offset() {
    let bytes = [99, 1, 1, 1, 7, 88, 77];
    let input = &bytes[1..];
    let (args, tail) = NestedArgs::parse_with_tail_checked(input).unwrap();
    assert_eq!(args as *const NestedArgs as *const u8, input.as_ptr());
    assert_eq!(tail, &[88, 77]);
    assert_eq!(tail.as_ptr(), input[4..].as_ptr());
    assert!(NestedArgs::parse_with_tail_checked(&input[..4])
        .unwrap()
        .1
        .is_empty());
    assert!(NestedArgs::parse_with_tail(&[1, 255, 1, 7, 88]).is_ok());
    assert!(NestedArgs::parse_with_tail_checked(&[1, 255, 1, 7, 88]).is_err());
}

#[test]
fn every_layout_macro_composes_value_validation_without_changing_size() {
    assert_eq!(core::mem::size_of::<HeaderedModes>(), 4);
    assert_eq!(core::mem::size_of::<CompactModes>(), 4);
    let header_size = hopper::hopper_core::account::HEADER_LEN;
    assert_eq!(core::mem::size_of::<DeclaredModes>(), header_size + 4);
    assert_eq!(core::mem::size_of::<ForeignModes>(), header_size + 4);
    let mut bytes = vec![0; LayoutArgs::PACKED_SIZE];
    let offsets = [0, 4, 8 + header_size, 12 + header_size * 2];
    for offset in offsets {
        bytes[offset..offset + 4].copy_from_slice(&[1, 1, 1, 7]);
    }
    assert!(LayoutArgs::parse_exact_checked(&bytes).is_ok());
    for offset in offsets {
        bytes[offset + 3] = 255;
        assert_eq!(
            LayoutArgs::parse_checked(&bytes).err(),
            Some(ProgramError::InvalidInstructionData),
        );
        bytes[offset + 3] = 7;
    }
}

#[test]
fn scalar_and_empty_array_validation_preserve_existing_value_semantics() {
    assert_eq!(Pod::validate_value(&[255u8; 32]), Ok(()));
    assert_eq!(Pod::validate_value(&[] as &[EnumByte<Mode>; 0]), Ok(()));
    assert_eq!(
        Pod::validate_value(&OptionByte::none(EnumByte::<Mode>::from_raw(255))),
        Ok(())
    );
}
