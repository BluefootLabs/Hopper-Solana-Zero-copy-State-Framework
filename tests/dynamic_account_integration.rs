#![cfg(feature = "proc-macros")]

use hopper::__runtime::ProgramError;
use hopper::prelude::*;

#[hopper::dynamic_account(disc = 7, version = 1)]
pub struct InlineMultisig {
    pub creator: Address,
    pub threshold: u8,
    pub slots: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,
}

#[test]
fn dynamic_account_generates_fixed_body_tail_and_views() {
    let creator = Address::new([1u8; 32]);
    let signer_a = Address::new([2u8; 32]);
    let signer_b = Address::new([3u8; 32]);

    let body = InlineMultisig::new(creator, 2, 42);
    assert_eq!(body.creator(), creator);
    assert_eq!(body.threshold(), 2);
    assert_eq!(body.slots(), 42);
    assert!(InlineMultisig::HAS_DYNAMIC_TAIL);
    assert_eq!(InlineMultisig::TAIL_PREFIX_OFFSET, InlineMultisig::LEN);

    let mut tail = InlineMultisigTail::default();
    tail.label.set_str("council").unwrap();
    tail.signers.push(signer_a).unwrap();
    tail.signers.push(signer_b).unwrap();

    let mut data = vec![0u8; InlineMultisig::ALLOC_SPACE];
    InlineMultisig::tail_write(&mut data, &tail).unwrap();

    let view = InlineMultisig::tail_view(&data).unwrap();
    assert_eq!(view.label().unwrap(), "council");
    assert_eq!(view.signers().unwrap(), &[signer_a, signer_b]);
    assert_eq!(InlineMultisig::label(&data).unwrap(), "council");
    assert_eq!(
        InlineMultisig::signers(&data).unwrap(),
        &[signer_a, signer_b]
    );
}

#[test]
fn generated_editor_commits_string_and_vector_changes() {
    let signer = Address::new([9u8; 32]);
    let mut data = vec![0u8; InlineMultisig::ALLOC_SPACE];
    InlineMultisig::tail_write(&mut data, &InlineMultisigTail::default()).unwrap();

    InlineMultisig::set_label(&mut data, "ops").unwrap();
    assert_eq!(InlineMultisig::label(&data).unwrap(), "ops");

    assert!(InlineMultisig::push_unique_signer(&mut data, signer).unwrap());
    assert!(!InlineMultisig::push_unique_signer(&mut data, signer).unwrap());
    assert_eq!(InlineMultisig::signers(&data).unwrap(), &[signer]);
    assert!(InlineMultisig::remove_signer(&mut data, &signer).unwrap());
    assert!(InlineMultisig::signers(&data).unwrap().is_empty());
}

#[test]
fn invalid_utf8_in_compact_tail_is_rejected_by_borrowed_view() {
    let mut data = vec![0u8; InlineMultisig::ALLOC_SPACE];
    let tail_offset = InlineMultisig::TAIL_PREFIX_OFFSET;
    data[tail_offset..tail_offset + 4].copy_from_slice(&5u32.to_le_bytes());
    data[tail_offset + 4..tail_offset + 9].copy_from_slice(&[1, 0, 0xFF, 0, 0]);

    let err = InlineMultisig::label(&data).unwrap_err();
    assert!(matches!(err, ProgramError::InvalidAccountData));
}

#[test]
fn bounded_tail_capacity_is_enforced() {
    let mut tail = InlineMultisigTail::default();
    let too_long = "x".repeat(33);
    assert!(tail.label.set_str(&too_long).is_err());

    for value in 0u8..10 {
        tail.signers.push(Address::new([value; 32])).unwrap();
    }
    let err = tail.signers.push(Address::new([42u8; 32])).unwrap_err();
    assert!(matches!(err, ProgramError::AccountDataTooSmall));
}

#[test]
fn account_too_small_errors_before_tail_write() {
    let tail = InlineMultisigTail::default();
    let mut data = vec![0u8; InlineMultisig::INIT_SPACE + 4 + 2];

    assert_eq!(InlineMultisig::tail_capacity(&data).unwrap(), 2);
    let err = InlineMultisig::tail_write(&mut data, &tail).unwrap_err();
    assert!(matches!(err, ProgramError::AccountDataTooSmall));
}

mod tail_cap_32 {
    use super::*;

    #[hopper::dynamic_account(disc = 8, version = 1)]
    pub struct SameFixedBody {
        pub creator: Address,

        #[tail(string<32>)]
        pub label: String,
    }
}

mod tail_cap_64 {
    use super::*;

    #[hopper::dynamic_account(disc = 8, version = 1)]
    pub struct SameFixedBody {
        pub creator: Address,

        #[tail(string<64>)]
        pub label: String,
    }
}

#[test]
fn layout_fingerprint_includes_dynamic_tail_schema() {
    assert_ne!(
        tail_cap_32::SameFixedBody::LAYOUT_ID,
        tail_cap_64::SameFixedBody::LAYOUT_ID
    );
}
