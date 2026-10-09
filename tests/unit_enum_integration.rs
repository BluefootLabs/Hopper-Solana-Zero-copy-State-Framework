//! `#[hopper::unit_enum]` + `EnumByte<E>`: a fieldless enum as a layout
//! field and as an instruction argument, validated on every read.

#![cfg(feature = "proc-macros")]

use hopper::prelude::*;

#[hopper::unit_enum]
pub enum Status {
    Open = 1,
    Settled = 2,
    Cancelled = 7,
}

/// Implicit discriminants and the user's own derives and repr are kept.
#[hopper::unit_enum]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 91, version = 1)]
pub struct Order {
    pub maker: Address,
    pub status: EnumByte<Status>,
    pub side: EnumByte<Side>,
    pub referrer: OptionByte<[u8; 32]>,
    pub amount: WireU64,
}

#[hopper::args]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct PlaceArgs {
    pub side: EnumByte<Side>,
    pub amount: WireU64,
    pub referrer: OptionByte<[u8; 32]>,
}

#[test]
fn the_macro_generates_the_mapping_from_the_declaration() {
    assert_eq!(Status::VARIANT_COUNT, 3);
    assert_eq!(
        Status::VARIANTS,
        [Status::Open, Status::Settled, Status::Cancelled]
    );
    for variant in Status::VARIANTS {
        assert_eq!(Status::from_byte(variant.to_byte()), Some(variant));
    }
    assert_eq!(Status::Cancelled.to_byte(), 7);
    assert_eq!(Status::from_byte(0), None);
    assert_eq!(Status::from_byte(3), None);
    assert_eq!(Side::Bid.to_byte(), 0);
    assert_eq!(Side::from_byte(1), Some(Side::Ask));
    assert_eq!(Side::from_byte(2), None);
    assert!(Side::Bid < Side::Ask, "the user's own derives survive");
}

#[test]
fn a_layout_carries_enums_and_options_as_single_bytes() {
    // maker 32 + status 1 + side 1 + referrer 33 + amount 8
    assert_eq!(Order::BODY_SIZE, 75);
    assert_eq!(core::mem::align_of::<Order>(), 1);

    let mut data = vec![0u8; Order::LEN];
    Order::write_init_header(&mut data).unwrap();
    {
        let order = Order::overlay_mut(&mut data[hopper::layout::HEADER_LEN..]).unwrap();
        // A zero-filled account holds no valid `Status` (no variant is 0)
        // and the first `Side`.
        assert!(order.status.get().is_err());
        assert_eq!(order.side.get(), Ok(Side::Bid));
        assert!(order.referrer.get().unwrap().is_none());

        order.status.set(Status::Open);
        order.side.set(Side::Ask);
        order.referrer = OptionByte::some([9u8; 32]);
        order.amount = WireU64::new(5);
    }
    let body = &data[hopper::layout::HEADER_LEN..];
    assert_eq!(body[32], 1, "status byte");
    assert_eq!(body[33], 1, "side byte");
    assert_eq!(body[34], 1, "option tag");
    assert_eq!(&body[35..67], &[9u8; 32]);

    let order = Order::overlay(body).unwrap();
    assert!(order.status == Status::Open);
    assert_eq!(order.status.get(), Ok(Status::Open));
    assert_eq!(order.referrer.get().unwrap(), Some(&[9u8; 32]));
}

#[test]
fn a_corrupt_byte_is_an_error_never_an_invalid_enum() {
    let mut data = vec![0u8; Order::LEN];
    Order::write_init_header(&mut data).unwrap();
    data[hopper::layout::HEADER_LEN + 32] = 200;
    data[hopper::layout::HEADER_LEN + 33] = 2;
    let order = Order::overlay(&data[hopper::layout::HEADER_LEN..]).unwrap();
    assert_eq!(order.status.get(), Err(ProgramError::InvalidAccountData));
    assert_eq!(order.side.get(), Err(ProgramError::InvalidAccountData));
    assert_eq!(order.status.raw(), 200);
    assert!(!order.status.is(Status::Open));
}

#[test]
fn arguments_refuse_an_unknown_variant_at_parse() {
    let mut good = vec![1u8];
    good.extend_from_slice(&42u64.to_le_bytes());
    good.push(1);
    good.extend_from_slice(&[3u8; 32]);
    let args = PlaceArgs::parse(&good).unwrap();
    assert_eq!(args.side.get(), Ok(Side::Ask));
    assert_eq!(args.amount.get(), 42);

    let mut bad = vec![5u8];
    bad.extend_from_slice(&42u64.to_le_bytes());
    bad.push(0);
    bad.extend_from_slice(&[0u8; 32]);
    // `parse` is the raw overlay; `parse_checked` validates every tagged
    // field. Call the checked parser at the instruction's input boundary.
    assert_eq!(
        PlaceArgs::parse_checked(&bad).err(),
        Some(ProgramError::InvalidInstructionData)
    );
    let checked = PlaceArgs::parse_checked(&good).unwrap();
    assert_eq!(checked.referrer.get().unwrap(), Some(&[3u8; 32]));

    // An option tag other than 0 or 1 is refused the same way.
    let mut bad_tag = good.clone();
    bad_tag[9] = 2;
    assert_eq!(
        PlaceArgs::parse_checked(&bad_tag).err(),
        Some(ProgramError::InvalidInstructionData)
    );
}
