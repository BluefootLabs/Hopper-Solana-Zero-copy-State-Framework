#![cfg(feature = "proc-macros")]

use hopper::__runtime::ProgramError;
use hopper::prelude::*;

#[hopper::dynamic_account(discriminator = 7, version = 1)]
pub struct InlineMultisig {
    pub creator: Address,
    pub threshold: u8,
    pub slots: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,
}

#[hopper::dynamic_account(discriminator = 9, version = 1)]
pub struct WeightedVotes {
    pub epoch: u64,

    #[tail(vec<u16, 4>)]
    pub weights: Vec<u16>,
}

#[hopper::account(discriminator = 12, version = 1)]
pub struct BareNote<'a> {
    pub author: Address,
    pub content: TailStr<'a>,
}

#[hopper::account(discriminator = 13, version = 1)]
pub struct BareBlob<'a> {
    pub tag: u8,
    pub blob: TailBytes<'a>,
}

#[hopper::account(discriminator = 10, version = 1)]
pub struct PrettyMultisig<'a> {
    pub creator: Address,
    pub threshold: u8,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
}

#[allow(dead_code)]
fn generated_account_wrapper_methods_compile<'info>(
    account: Account<'info, PrettyMultisig>,
) -> ProgramResult {
    let _label = account.label()?;
    let _signers = account.signers()?;
    account.set_label("ops")?;
    let _ = account.push_unique_signer(Address::new([7u8; 32]))?;
    Ok(())
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
fn dynamic_account_accepts_generic_tail_vectors() {
    let body = WeightedVotes::new(99);
    assert_eq!(body.epoch(), 99);

    let mut tail = WeightedVotesTail::default();
    tail.weights.push(7).unwrap();
    tail.weights.push(11).unwrap();

    let mut data = vec![0u8; WeightedVotes::ALLOC_SPACE];
    WeightedVotes::tail_write(&mut data, &tail).unwrap();

    let view = WeightedVotes::tail_view(&data).unwrap();
    let weights = view.weights().unwrap();
    assert_eq!(weights.as_slice(), &[7, 11]);

    let weights = WeightedVotes::weights(&data).unwrap();
    assert_eq!(weights.as_slice(), &[7, 11]);

    assert!(WeightedVotes::push_unique_weight(&mut data, 13).unwrap());
    assert!(!WeightedVotes::push_unique_weight(&mut data, 13).unwrap());
    assert!(WeightedVotes::remove_weight(&mut data, &7).unwrap());
    let weights = WeightedVotes::weights(&data).unwrap();
    assert_eq!(weights.as_slice(), &[11, 13]);
}

#[test]
fn bare_tail_str_consumes_remaining_payload() {
    let author = Address::new([8u8; 32]);
    let body = BareNote::new(author);
    assert_eq!(body.author(), author);
    assert!(BareNote::HAS_DYNAMIC_TAIL);
    assert!(BareNote::HAS_RAW_DYNAMIC_TAIL);
    assert_eq!(BareNote::TAIL_PREFIX_OFFSET, BareNote::LEN);

    let mut data = vec![0u8; BareNote::space_for_tail("hello hopper".len())];
    let tail = BareNoteTail {
        content: TailStr::from_str("hello hopper"),
    };
    BareNote::tail_write(&mut data, &tail).unwrap();

    assert_eq!(
        BareNote::tail_len(&data).unwrap(),
        "hello hopper".len() as u32
    );
    assert_eq!(
        BareNote::content(&data).unwrap().as_str().unwrap(),
        "hello hopper"
    );
    assert_eq!(
        BareNote::tail_read(&data)
            .unwrap()
            .content
            .as_str()
            .unwrap(),
        "hello hopper"
    );

    BareNote::set_content(&mut data, "raw tail").unwrap();
    assert_eq!(
        BareNote::content(&data).unwrap().as_str().unwrap(),
        "raw tail"
    );
}

#[test]
fn bare_tail_bytes_are_binary_safe() {
    let body = BareBlob::new(7);
    assert_eq!(body.tag(), 7);

    let raw = [0, 1, 2, 0xFF, 4, 5];
    let mut data = vec![0u8; BareBlob::space_for_tail(raw.len())];
    let tail = BareBlobTail {
        blob: TailBytes::new(&raw),
    };
    BareBlob::tail_write(&mut data, &tail).unwrap();

    let view = BareBlob::tail_view(&data).unwrap();
    assert_eq!(view.blob().unwrap().as_bytes(), raw);
    assert_eq!(BareBlob::blob(&data).unwrap().as_bytes(), raw);

    BareBlob::set_blob(&mut data, &[9, 8, 7]).unwrap();
    assert_eq!(BareBlob::blob(&data).unwrap().as_bytes(), &[9, 8, 7]);
}

#[test]
fn bare_tail_str_validates_utf8_on_access() {
    let mut data = vec![0u8; BareNote::space_for_tail(1)];
    let offset = BareNote::TAIL_PREFIX_OFFSET;
    data[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
    data[offset + 4] = 0xFF;

    let err = BareNote::content(&data).unwrap().as_str().unwrap_err();
    assert!(matches!(err, ProgramError::InvalidAccountData));

    let err =
        BareNote::tail_write_parts(&mut data, &BareNoteTailHead::default(), &[0xFF]).unwrap_err();
    assert!(matches!(err, ProgramError::InvalidAccountData));
}

#[test]
fn account_attribute_auto_upgrades_pretty_dynamic_fields() {
    let creator = Address::new([4u8; 32]);
    let signer = Address::new([5u8; 32]);

    let body = PrettyMultisig::new(creator, 3);
    assert_eq!(body.creator(), creator);
    assert_eq!(body.threshold(), 3);
    assert!(PrettyMultisig::HAS_DYNAMIC_TAIL);
    assert_eq!(PrettyMultisig::TAIL_PREFIX_OFFSET, PrettyMultisig::LEN);

    let mut tail = PrettyMultisigTail::default();
    tail.label.set_str("governance").unwrap();
    tail.signers.push(signer).unwrap();

    let mut data = vec![0u8; PrettyMultisig::ALLOC_SPACE];
    PrettyMultisig::tail_write(&mut data, &tail).unwrap();
    assert_eq!(PrettyMultisig::label(&data).unwrap(), "governance");
    assert_eq!(PrettyMultisig::signers(&data).unwrap(), &[signer]);

    PrettyMultisig::set_label(&mut data, "ops").unwrap();
    assert_eq!(PrettyMultisig::label(&data).unwrap(), "ops");
}

mod pretty_cap_32 {
    use super::*;

    #[hopper::account(discriminator = 11, version = 1)]
    pub struct PrettySameFixedBody<'a> {
        pub creator: Address,
        pub label: String<'a, 32>,
    }
}

mod pretty_cap_64 {
    use super::*;

    #[hopper::account(discriminator = 11, version = 1)]
    pub struct PrettySameFixedBody<'a> {
        pub creator: Address,
        pub label: String<'a, 64>,
    }
}

#[test]
fn account_pretty_tail_schema_changes_layout_fingerprint() {
    assert_ne!(
        pretty_cap_32::PrettySameFixedBody::LAYOUT_ID,
        pretty_cap_64::PrettySameFixedBody::LAYOUT_ID
    );
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

mod bare_tail_str_fp {
    use super::*;

    #[hopper::account(discriminator = 14, version = 1)]
    pub struct SameFixedBody<'a> {
        pub author: Address,
        pub content: TailStr<'a>,
    }
}

mod bare_tail_bytes_fp {
    use super::*;

    #[hopper::account(discriminator = 14, version = 1)]
    pub struct SameFixedBody<'a> {
        pub author: Address,
        pub content: TailBytes<'a>,
    }
}

#[test]
fn layout_fingerprint_includes_dynamic_tail_schema() {
    assert_ne!(
        tail_cap_32::SameFixedBody::LAYOUT_ID,
        tail_cap_64::SameFixedBody::LAYOUT_ID
    );
}

#[test]
fn layout_fingerprint_distinguishes_bare_tail_kind() {
    assert_ne!(
        bare_tail_str_fp::SameFixedBody::LAYOUT_ID,
        bare_tail_bytes_fp::SameFixedBody::LAYOUT_ID
    );
}
