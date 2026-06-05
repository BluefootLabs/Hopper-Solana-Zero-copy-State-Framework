//! Devnet tail lab: bounded compact fields plus explicit bare final tails.
//!
//! The example keeps the authoring surface compact while proving Hopper's
//! layout contracts: role metadata, fingerprints, generated account wrappers,
//! bounded tail editors, and final-only raw tails.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;
use hopper::systems::{init_header, HopperHeader};

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

pub const NOTE_BODY_MAX: usize = 160;
pub const BLOB_BYTES_MAX: usize = 96;

#[hopper::account(discriminator = 21, version = 1)]
pub struct TailNote<'a> {
    #[role(authority)]
    pub authority: Address,

    #[role(version)]
    pub revision: WireU64,

    pub label: String<'a, 32>,
    pub reviewers: Vec<'a, Address, 4>,
    pub body: TailStr<'a>,
}

#[hopper::account(discriminator = 22, version = 1)]
pub struct TailBlob<'a> {
    #[role(authority)]
    pub authority: Address,

    #[role(version)]
    pub revision: WireU64,

    pub tag: u8,
    pub payload: TailBytes<'a>,
}

hopper::hopper_error! {
    base = 6700;
    EmptyBody,
    EmptyPayload
}

#[derive(Accounts)]
pub struct InitializeNote<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = TailNote::space_for_tail(NOTE_BODY_MAX))]
    pub note: InitAccount<'info, TailNote>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateNote<'info> {
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub note: Account<'info, TailNote>,
}

#[derive(Accounts)]
pub struct InitializeBlob<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = TailBlob::space_for_tail(BLOB_BYTES_MAX))]
    pub blob: InitAccount<'info, TailBlob>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateBlob<'info> {
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub blob: Account<'info, TailBlob>,
}

#[program(max_accounts = 8)]
mod tail_lab_program {
    use super::*;

    #[instruction(0)]
    pub fn init_note(
        ctx: Ctx<InitializeNote>,
        label: HopperString<32>,
        body: HopperString<NOTE_BODY_MAX>,
    ) -> ProgramResult {
        ctx.init_note()?;
        ctx.accounts.initialize(label, body)
    }

    #[instruction(1)]
    pub fn rewrite_note(
        ctx: Ctx<UpdateNote>,
        label: HopperString<32>,
        body: HopperString<NOTE_BODY_MAX>,
    ) -> ProgramResult {
        ctx.accounts.rewrite(label, body)
    }

    #[instruction(2)]
    pub fn add_reviewer(ctx: Ctx<UpdateNote>, reviewer: Address) -> ProgramResult {
        ctx.accounts.add_reviewer(reviewer)
    }

    #[instruction(3)]
    pub fn init_blob(
        ctx: Ctx<InitializeBlob>,
        tag: u8,
        payload: HopperVec<u8, BLOB_BYTES_MAX>,
    ) -> ProgramResult {
        ctx.init_blob()?;
        ctx.accounts.initialize(tag, payload)
    }

    #[instruction(4)]
    pub fn write_blob(
        ctx: Ctx<UpdateBlob>,
        tag: u8,
        payload: HopperVec<u8, BLOB_BYTES_MAX>,
    ) -> ProgramResult {
        ctx.accounts.write(tag, payload)
    }
}

impl<'info> InitializeNote<'info> {
    pub fn initialize(
        &self,
        label: HopperString<32>,
        body: HopperString<NOTE_BODY_MAX>,
    ) -> ProgramResult {
        hopper::hopper_require!(!body.is_empty(), EmptyBody);

        {
            let mut note = self.note.get_mut_after_init()?;
            note.set_inner(*self.authority.key(), 0)?;
        }

        let reviewers = HopperVec::<Address, 4>::from_slice(&[*self.authority.key()])?;
        write_note_tail(
            &mut self.note.as_account().try_borrow_mut()?,
            label,
            reviewers,
            body.as_bytes(),
        )
        .map(|_| ())
    }
}

impl<'info> UpdateNote<'info> {
    pub fn rewrite(
        &self,
        label: HopperString<32>,
        body: HopperString<NOTE_BODY_MAX>,
    ) -> ProgramResult {
        hopper::hopper_require!(!body.is_empty(), EmptyBody);
        bump_note_revision(&self.note)?;

        let mut data = self.note.as_account().try_borrow_mut()?;
        let mut editor = TailNote::tail_editor(&mut data)?;
        editor.set_label(label.as_str()?)?;
        editor.commit_with_raw(body.as_bytes())
    }

    pub fn add_reviewer(&self, reviewer: Address) -> ProgramResult {
        let body = read_note_body::<NOTE_BODY_MAX>(self.note.as_account())?;
        bump_note_revision(&self.note)?;

        let mut data = self.note.as_account().try_borrow_mut()?;
        let mut editor = TailNote::tail_editor(&mut data)?;
        let _inserted = editor.push_unique_reviewer(reviewer)?;
        editor.commit_with_raw(body.as_bytes())
    }
}

impl<'info> InitializeBlob<'info> {
    pub fn initialize(&self, tag: u8, payload: HopperVec<u8, BLOB_BYTES_MAX>) -> ProgramResult {
        hopper::hopper_require!(!payload.is_empty(), EmptyPayload);

        {
            let mut blob = self.blob.get_mut_after_init()?;
            blob.set_inner(*self.authority.key(), 0, tag)?;
        }

        TailBlob::tail_write(
            &mut self.blob.as_account().try_borrow_mut()?,
            &TailBlobTail {
                payload: TailBytes::new(payload.as_slice()),
            },
        )
        .map(|_| ())
    }
}

impl<'info> UpdateBlob<'info> {
    pub fn write(&self, tag: u8, payload: HopperVec<u8, BLOB_BYTES_MAX>) -> ProgramResult {
        hopper::hopper_require!(!payload.is_empty(), EmptyPayload);

        {
            let mut blob = self.blob.get_mut()?;
            blob.revision.checked_add_assign(1)?;
            blob.tag = tag;
        }

        TailBlob::tail_write(
            &mut self.blob.as_account().try_borrow_mut()?,
            &TailBlobTail {
                payload: TailBytes::new(payload.as_slice()),
            },
        )
        .map(|_| ())
    }
}

pub fn initialize_note_data(
    data: &mut [u8],
    authority: Address,
    label: &str,
    body: &str,
    reviewers: &[Address],
) -> ProgramResult {
    if body.is_empty() {
        return Err(EmptyBody.into());
    }
    init_header::<TailNote>(data)?;
    let note = TailNote::overlay_mut(&mut data[HopperHeader::SIZE..TailNote::TAIL_PREFIX_OFFSET])?;
    *note = TailNote::new(authority, 0.into());
    write_note_tail(
        data,
        HopperString::from_str(label)?,
        HopperVec::from_slice(reviewers)?,
        body.as_bytes(),
    )
    .map(|_| ())
}

pub fn rewrite_note_data(data: &mut [u8], label: &str, body: &str) -> ProgramResult {
    if body.is_empty() {
        return Err(EmptyBody.into());
    }
    let fixed = TailNote::overlay_mut(&mut data[HopperHeader::SIZE..TailNote::TAIL_PREFIX_OFFSET])?;
    fixed.revision.checked_add_assign(1)?;

    let mut editor = TailNote::tail_editor(data)?;
    editor.set_label(label)?;
    editor.commit_with_raw(body.as_bytes())
}

pub fn add_reviewer_data(data: &mut [u8], reviewer: Address) -> ProgramResult {
    let body = TailNote::body(data)?.as_str()?;
    let body = HopperString::<NOTE_BODY_MAX>::from_str(body)?;
    let fixed = TailNote::overlay_mut(&mut data[HopperHeader::SIZE..TailNote::TAIL_PREFIX_OFFSET])?;
    fixed.revision.checked_add_assign(1)?;

    let mut editor = TailNote::tail_editor(data)?;
    let _inserted = editor.push_unique_reviewer(reviewer)?;
    editor.commit_with_raw(body.as_bytes())
}

pub fn initialize_blob_data(
    data: &mut [u8],
    authority: Address,
    tag: u8,
    payload: &[u8],
) -> ProgramResult {
    if payload.is_empty() {
        return Err(EmptyPayload.into());
    }
    init_header::<TailBlob>(data)?;
    let blob = TailBlob::overlay_mut(&mut data[HopperHeader::SIZE..TailBlob::TAIL_PREFIX_OFFSET])?;
    *blob = TailBlob::new(authority, 0.into(), tag);
    TailBlob::tail_write(
        data,
        &TailBlobTail {
            payload: TailBytes::new(payload),
        },
    )
    .map(|_| ())
}

pub fn write_blob_data(data: &mut [u8], tag: u8, payload: &[u8]) -> ProgramResult {
    if payload.is_empty() {
        return Err(EmptyPayload.into());
    }
    let fixed = TailBlob::overlay_mut(&mut data[HopperHeader::SIZE..TailBlob::TAIL_PREFIX_OFFSET])?;
    fixed.revision.checked_add_assign(1)?;
    fixed.tag = tag;
    TailBlob::tail_write(
        data,
        &TailBlobTail {
            payload: TailBytes::new(payload),
        },
    )
    .map(|_| ())
}

fn write_note_tail(
    data: &mut [u8],
    label: HopperString<32>,
    reviewers: HopperVec<Address, 4>,
    body: &[u8],
) -> Result<usize, ProgramError> {
    TailNote::tail_write(
        data,
        &TailNoteTail {
            label,
            reviewers,
            body: TailStr::new(body),
        },
    )
}

fn read_note_body<const N: usize>(account: &AccountView) -> Result<HopperString<N>, ProgramError> {
    let data = account.try_borrow()?;
    let body = TailNote::body(&data)?.as_str()?;
    HopperString::<N>::from_str(body)
}

fn bump_note_revision(note: &Account<'_, TailNote>) -> ProgramResult {
    let mut fixed = note.get_mut()?;
    fixed.revision.checked_add_assign(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_roundtrips_bounded_fields_and_final_text_tail() {
        let authority = Address::new([1u8; 32]);
        let reviewer = Address::new([2u8; 32]);
        let mut data = [0u8; TailNote::space_for_tail(NOTE_BODY_MAX)];

        initialize_note_data(&mut data, authority, "audit", "ship the tail", &[authority]).unwrap();
        assert_eq!(TailNote::label(&data).unwrap(), "audit");
        assert_eq!(TailNote::reviewers(&data).unwrap(), &[authority]);
        assert_eq!(
            TailNote::body(&data).unwrap().as_str().unwrap(),
            "ship the tail"
        );

        add_reviewer_data(&mut data, reviewer).unwrap();
        rewrite_note_data(&mut data, "ops", "edited body").unwrap();

        assert_eq!(TailNote::label(&data).unwrap(), "ops");
        assert_eq!(TailNote::reviewers(&data).unwrap(), &[authority, reviewer]);
        assert_eq!(
            TailNote::body(&data).unwrap().as_str().unwrap(),
            "edited body"
        );
        assert_eq!(
            TailNote::tail_read(&data)
                .unwrap()
                .reviewers
                .as_slice()
                .len(),
            2
        );
    }

    #[test]
    fn blob_roundtrips_binary_tail() {
        let authority = Address::new([3u8; 32]);
        let mut data = [0u8; TailBlob::space_for_tail(BLOB_BYTES_MAX)];

        initialize_blob_data(&mut data, authority, 7, &[0, 1, 2, 0xFF]).unwrap();
        assert_eq!(
            TailBlob::payload(&data).unwrap().as_bytes(),
            &[0, 1, 2, 0xFF]
        );

        write_blob_data(&mut data, 9, &[9, 8, 7]).unwrap();
        assert_eq!(TailBlob::payload(&data).unwrap().as_bytes(), &[9, 8, 7]);
        assert_eq!(
            TailBlob::overlay(&data[HopperHeader::SIZE..TailBlob::TAIL_PREFIX_OFFSET])
                .unwrap()
                .tag(),
            9
        );
    }
}
