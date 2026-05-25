//! Hopper port of the Styx ferry/messaging core surfaces.
//!
//! The program keeps X3DH, Double Ratchet, and payload encryption client-side.
//! On chain it verifies signers, bounded envelope sizes, monotonic counters,
//! prekey ownership, and the StyxZK verifier CPI proof boundary.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    #[cfg(not(feature = "solana-program-backend"))]
    hopper::no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    hopper::nostd_panic_handler!();
}

pub const RATCHET_CIPHERTEXT_MAX: usize = 900;
pub const STYX_ZK_PROOF_V2_LEN: usize = 513;
pub const STYX_ZK_INPUTS_OFFSET: usize = 1 + 64 + 128 + 64;
pub const STYX_ZK_PUBLIC_INPUTS: usize = 8;
pub const MAX_ONE_TIME_PREKEYS: u16 = 200;
pub const MAX_FEE_TIER: u64 = 3;
pub const ED25519_PRECOMPILE_SAME_INSTRUCTION: u16 = u16::MAX;

pub const BN254_FR_MODULUS: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

pub const EVENT_PREKEY_PUBLISH: u8 = 0x20;
pub const EVENT_PREKEY_REFRESH: u8 = 0x21;
pub const EVENT_RATCHET_MESSAGE: u8 = 0x22;
pub const EVENT_ZK_FERRY: u8 = 0x23;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 73, version = 1)]
pub struct FerryConfig {
    pub authority: Address,
    pub verifier_program: Address,
    pub domain_separator: [u8; 32],
    pub base_fee_lamports: WireU64,
    pub message_count: WireU64,
    pub prekey_update_count: WireU64,
    pub zk_ferry_count: WireU64,
}

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 74, version = 1)]
pub struct PrekeyBundle {
    pub owner: Address,
    pub domain: [u8; 32],
    pub identity_key: [u8; 32],
    pub signed_prekey_id: WireU32,
    pub signed_prekey: [u8; 32],
    pub signed_prekey_signature: [u8; 64],
    pub one_time_prekey_root: [u8; 32],
    pub one_time_prekey_count: WireU16,
    pub published_at: WireU64,
    pub refresh_count: WireU64,
}

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 75, version = 1)]
pub struct MessageThread {
    pub sender: Address,
    pub recipient: Address,
    pub last_counter: WireU64,
    pub message_count: WireU64,
    pub last_ratchet_key: [u8; 32],
    pub last_sealed_message_hash: [u8; 32],
}

hopper::hopper_error! {
    base = 6900;
    EmptyDomainSeparator,
    EmptyPrekeyDomain,
    InvalidPrekeyCount,
    EmptyCiphertext,
    CounterNotMonotonic,
    VerifierProgramMismatch,
    InvalidProofLength,
    InvalidProofVersion,
    InvalidProofPublicInputCount,
    InvalidProofDomainSeparator,
    InvalidProofBaseFee,
    InvalidProofFeeTier,
    MissingSignedPrekeySignature,
    InvalidSignedPrekeySignatureInstruction,
}

#[derive(Accounts)]
pub struct InitConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = FerryConfig::INIT_SPACE)]
    pub config: InitAccount<'info, FerryConfig>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PublishPrekeyBundle<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(mut)]
    pub config: Account<'info, FerryConfig>,

    #[account(init, payer = owner, space = PrekeyBundle::INIT_SPACE)]
    pub bundle: InitAccount<'info, PrekeyBundle>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RefreshPrekeyBundle<'info> {
    pub owner: Signer<'info>,

    #[account(mut)]
    pub config: Account<'info, FerryConfig>,

    #[account(mut, has_one = owner)]
    pub bundle: Account<'info, PrekeyBundle>,
}

#[derive(Accounts)]
pub struct InitThread<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,

    #[account(init, payer = sender, space = MessageThread::INIT_SPACE)]
    pub thread: InitAccount<'info, MessageThread>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SendRatchetMessage<'info> {
    pub sender: Signer<'info>,

    #[account(mut)]
    pub config: Account<'info, FerryConfig>,

    #[account(mut, has_one = sender)]
    pub thread: Account<'info, MessageThread>,
}

#[derive(Accounts)]
pub struct SubmitZkFerry<'info> {
    pub payer: Signer<'info>,

    #[account(mut)]
    pub config: Account<'info, FerryConfig>,

    pub verifier_program: UncheckedAccount<'info>,
}

#[program]
mod styx_ferry_program {
    use super::*;

    #[instruction(0)]
    pub fn init_config(
        ctx: Ctx<InitConfig>,
        verifier_program: Address,
        base_fee_lamports: u64,
    ) -> ProgramResult {
        ctx.init_config()?;
        let domain_separator = derive_styx_vsl_domain_separator(ctx.program_id())?;
        ctx.accounts
            .init(verifier_program, domain_separator, base_fee_lamports)
    }

    #[instruction(1)]
    pub fn publish_prekey_bundle(
        ctx: Ctx<PublishPrekeyBundle>,
        domain: [u8; 32],
        identity_key: [u8; 32],
        signed_prekey_id: u32,
        signed_prekey: [u8; 32],
        signed_prekey_signature: [u8; 64],
        one_time_prekey_root: [u8; 32],
        one_time_prekey_count: u16,
        published_at: u64,
        ed25519_sibling_index: u64,
    ) -> ProgramResult {
        ctx.init_bundle()?;
        ctx.accounts.publish(
            domain,
            identity_key,
            signed_prekey_id,
            signed_prekey,
            signed_prekey_signature,
            one_time_prekey_root,
            one_time_prekey_count,
            published_at,
            ed25519_sibling_index,
        )
    }

    #[instruction(2)]
    pub fn refresh_prekey_bundle(
        ctx: Ctx<RefreshPrekeyBundle>,
        one_time_prekey_root: [u8; 32],
        one_time_prekey_count: u16,
        refreshed_at: u64,
    ) -> ProgramResult {
        ctx.accounts
            .refresh(one_time_prekey_root, one_time_prekey_count, refreshed_at)
    }

    #[instruction(3)]
    pub fn init_thread(ctx: Ctx<InitThread>, recipient: Address) -> ProgramResult {
        ctx.init_thread()?;
        ctx.accounts.init(recipient)
    }

    #[instruction(4)]
    pub fn send_ratchet_message(
        ctx: Ctx<SendRatchetMessage>,
        counter: u64,
        ratchet_key: [u8; 32],
        sealed_message_hash: [u8; 32],
        ciphertext: HopperVec<u8, RATCHET_CIPHERTEXT_MAX>,
    ) -> ProgramResult {
        ctx.accounts
            .send(counter, ratchet_key, sealed_message_hash, ciphertext)
    }

    #[instruction(5)]
    pub fn submit_zk_ferry(
        ctx: Ctx<SubmitZkFerry>,
        proof: HopperVec<u8, STYX_ZK_PROOF_V2_LEN>,
        encrypted_outputs: [u8; 64],
    ) -> ProgramResult {
        ctx.accounts.submit(proof, encrypted_outputs)
    }
}

impl<'info> InitConfig<'info> {
    pub fn init(
        &self,
        verifier_program: Address,
        domain_separator: [u8; 32],
        base_fee_lamports: u64,
    ) -> ProgramResult {
        hopper::hopper_require!(has_any_byte(&domain_separator), EmptyDomainSeparator);

        let mut config = self.config.get_mut_after_init()?;
        config.set_inner(
            *self.authority.key(),
            verifier_program,
            domain_separator,
            base_fee_lamports,
            0,
            0,
            0,
        )
    }
}

impl<'info> PublishPrekeyBundle<'info> {
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &self,
        domain: [u8; 32],
        identity_key: [u8; 32],
        signed_prekey_id: u32,
        signed_prekey: [u8; 32],
        signed_prekey_signature: [u8; 64],
        one_time_prekey_root: [u8; 32],
        one_time_prekey_count: u16,
        published_at: u64,
        ed25519_sibling_index: u64,
    ) -> ProgramResult {
        validate_prekey_domain(&domain)?;
        validate_prekey_count(one_time_prekey_count)?;
        verify_signed_prekey_instruction(
            ed25519_sibling_index,
            self.owner.key(),
            &signed_prekey,
            &signed_prekey_signature,
        )?;

        {
            let mut bundle = self.bundle.get_mut_after_init()?;
            bundle.set_inner(
                *self.owner.key(),
                domain,
                identity_key,
                signed_prekey_id,
                signed_prekey,
                signed_prekey_signature,
                one_time_prekey_root,
                one_time_prekey_count,
                published_at,
                0,
            )?;
        }

        {
            let mut config = self.config.get_mut()?;
            config.prekey_update_count.checked_add_assign(1)?;
        }

        emit_prekey_event(
            EVENT_PREKEY_PUBLISH,
            self.owner.key(),
            &domain,
            &identity_key,
            signed_prekey_id,
            &one_time_prekey_root,
            one_time_prekey_count,
            published_at,
        );
        Ok(())
    }
}

impl<'info> RefreshPrekeyBundle<'info> {
    pub fn refresh(
        &self,
        one_time_prekey_root: [u8; 32],
        one_time_prekey_count: u16,
        refreshed_at: u64,
    ) -> ProgramResult {
        validate_prekey_count(one_time_prekey_count)?;

        let (domain, identity_key, signed_prekey_id) = {
            let mut bundle = self.bundle.get_mut()?;
            bundle.one_time_prekey_root = one_time_prekey_root;
            bundle.one_time_prekey_count = WireU16::new(one_time_prekey_count);
            bundle.published_at = WireU64::new(refreshed_at);
            bundle.refresh_count.checked_add_assign(1)?;
            (
                bundle.domain,
                bundle.identity_key,
                bundle.signed_prekey_id.get(),
            )
        };

        {
            let mut config = self.config.get_mut()?;
            config.prekey_update_count.checked_add_assign(1)?;
        }

        emit_prekey_event(
            EVENT_PREKEY_REFRESH,
            self.owner.key(),
            &domain,
            &identity_key,
            signed_prekey_id,
            &one_time_prekey_root,
            one_time_prekey_count,
            refreshed_at,
        );
        Ok(())
    }
}

impl<'info> InitThread<'info> {
    pub fn init(&self, recipient: Address) -> ProgramResult {
        let mut thread = self.thread.get_mut_after_init()?;
        thread.set_inner(*self.sender.key(), recipient, 0, 0, [0u8; 32], [0u8; 32])
    }
}

impl<'info> SendRatchetMessage<'info> {
    pub fn send(
        &self,
        counter: u64,
        ratchet_key: [u8; 32],
        sealed_message_hash: [u8; 32],
        ciphertext: HopperVec<u8, RATCHET_CIPHERTEXT_MAX>,
    ) -> ProgramResult {
        hopper::hopper_require!(!ciphertext.is_empty(), EmptyCiphertext);

        let recipient = {
            let mut thread = self.thread.get_mut()?;
            if counter <= thread.last_counter.get() {
                return Err(CounterNotMonotonic.into());
            }

            thread.last_counter = WireU64::new(counter);
            thread.message_count.checked_add_assign(1)?;
            thread.last_ratchet_key = ratchet_key;
            thread.last_sealed_message_hash = sealed_message_hash;
            thread.recipient
        };

        {
            let mut config = self.config.get_mut()?;
            config.message_count.checked_add_assign(1)?;
        }

        let counter_bytes = counter.to_le_bytes();
        let ciphertext_len = (ciphertext.len() as u16).to_le_bytes();
        hopper::events::emit_slices(&[
            &[EVENT_RATCHET_MESSAGE],
            self.sender.key().as_bytes(),
            recipient.as_bytes(),
            &counter_bytes,
            &ratchet_key,
            &sealed_message_hash,
            &ciphertext_len,
            ciphertext.as_slice(),
        ]);
        Ok(())
    }
}

impl<'info> SubmitZkFerry<'info> {
    pub fn submit(
        &self,
        proof: HopperVec<u8, STYX_ZK_PROOF_V2_LEN>,
        encrypted_outputs: [u8; 64],
    ) -> ProgramResult {
        let proof_bytes = proof.as_slice();
        hopper::hopper_require!(
            proof_bytes.len() == STYX_ZK_PROOF_V2_LEN,
            InvalidProofLength
        );
        hopper::hopper_require!(proof_bytes[0] == 2, InvalidProofVersion);
        hopper::hopper_require!(STYX_ZK_PUBLIC_INPUTS == 8, InvalidProofPublicInputCount);

        let (verifier_program, domain_separator, base_fee_lamports) = {
            let config = self.config.get()?;
            if config.verifier_program != *self.verifier_program.key() {
                return Err(VerifierProgramMismatch.into());
            }
            (
                config.verifier_program,
                config.domain_separator,
                config.base_fee_lamports.get(),
            )
        };

        let root = proof_input(proof_bytes, 0)?;
        let nullifier = proof_input(proof_bytes, 1)?;
        let out0_commitment = proof_input(proof_bytes, 2)?;
        let out1_commitment = proof_input(proof_bytes, 3)?;
        let asset_id = proof_input(proof_bytes, 4)?;
        let proof_domain_separator = proof_input(proof_bytes, 5)?;
        let fee_tier = read_bn254_field_u64(&proof_input(proof_bytes, 6)?);
        let proof_base_fee = read_bn254_field_u64(&proof_input(proof_bytes, 7)?);

        hopper::hopper_require!(
            proof_domain_separator == domain_separator,
            InvalidProofDomainSeparator
        );
        hopper::hopper_require!(proof_base_fee == base_fee_lamports, InvalidProofBaseFee);
        hopper::hopper_require!(fee_tier <= MAX_FEE_TIER, InvalidProofFeeTier);

        invoke_verifier(verifier_program, proof_bytes)?;

        {
            let mut config = self.config.get_mut()?;
            config.zk_ferry_count.checked_add_assign(1)?;
        }

        let fee_tier_bytes = fee_tier.to_le_bytes();
        let base_fee_bytes = base_fee_lamports.to_le_bytes();
        hopper::events::emit_slices(&[
            &[EVENT_ZK_FERRY],
            self.payer.key().as_bytes(),
            &root,
            &nullifier,
            &out0_commitment,
            &out1_commitment,
            &asset_id,
            &fee_tier_bytes,
            &base_fee_bytes,
            &encrypted_outputs,
        ]);
        Ok(())
    }
}

fn validate_prekey_domain(domain: &[u8; 32]) -> ProgramResult {
    hopper::hopper_require!(has_any_byte(domain), EmptyPrekeyDomain);
    Ok(())
}

fn validate_prekey_count(count: u16) -> ProgramResult {
    hopper::hopper_require!(
        count > 0 && count <= MAX_ONE_TIME_PREKEYS,
        InvalidPrekeyCount
    );
    Ok(())
}

fn has_any_byte(bytes: &[u8; 32]) -> bool {
    bytes.iter().any(|byte| *byte != 0)
}

fn derive_styx_vsl_domain_separator(program_id: &Address) -> Result<[u8; 32]> {
    let version = [2u8];
    let hash = crypto::keccak256(&[program_id.as_bytes(), &version, b"STYX_VSL"])?;
    Ok(keccak_le_to_bn254_fr_be(&hash))
}

fn keccak_le_to_bn254_fr_be(hash: &[u8; 32]) -> [u8; 32] {
    let mut be = [0u8; 32];
    for i in 0..32 {
        be[i] = hash[31 - i];
    }
    while be32_ge(&be, &BN254_FR_MODULUS) {
        be = be32_sub(&be, &BN254_FR_MODULUS);
    }
    be
}

fn be32_ge(a: &[u8; 32], b: &[u8; 32]) -> bool {
    for i in 0..32 {
        if a[i] > b[i] {
            return true;
        }
        if a[i] < b[i] {
            return false;
        }
    }
    true
}

fn be32_sub(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut result = [0u8; 32];
    let mut borrow: u16 = 0;
    for i in (0..32).rev() {
        let diff = (a[i] as u16).wrapping_sub(b[i] as u16).wrapping_sub(borrow);
        result[i] = diff as u8;
        borrow = if diff > 0xff { 1 } else { 0 };
    }
    result
}

fn verify_signed_prekey_instruction(
    sibling_index: u64,
    owner: &Address,
    signed_prekey: &[u8; 32],
    signed_prekey_signature: &[u8; 64],
) -> ProgramResult {
    let instruction = crypto::require_ed25519_instruction(sibling_index)
        .map_err(|_| MissingSignedPrekeySignature)?;
    verify_ed25519_payload(
        &instruction,
        owner.as_bytes(),
        signed_prekey,
        signed_prekey_signature,
    )
}

fn verify_ed25519_payload(
    instruction: &crypto::ProcessedInstruction,
    signer: &[u8; 32],
    message: &[u8; 32],
    signature: &[u8; 64],
) -> ProgramResult {
    const ED25519_OFFSET_TABLE_START: usize = 2;
    const ED25519_OFFSET_TABLE_LEN: usize = 14;
    const ED25519_SINGLE_SIG_HEADER_LEN: usize =
        ED25519_OFFSET_TABLE_START + ED25519_OFFSET_TABLE_LEN;

    if instruction.data_len < ED25519_SINGLE_SIG_HEADER_LEN || instruction.data[0] != 1 {
        return Err(InvalidSignedPrekeySignatureInstruction.into());
    }

    let signature_offset = read_u16_le(&instruction.data, 2)? as usize;
    let signature_instruction_index = read_u16_le(&instruction.data, 4)?;
    let public_key_offset = read_u16_le(&instruction.data, 6)? as usize;
    let public_key_instruction_index = read_u16_le(&instruction.data, 8)?;
    let message_offset = read_u16_le(&instruction.data, 10)? as usize;
    let message_size = read_u16_le(&instruction.data, 12)? as usize;
    let message_instruction_index = read_u16_le(&instruction.data, 14)?;

    if signature_instruction_index != ED25519_PRECOMPILE_SAME_INSTRUCTION
        || public_key_instruction_index != ED25519_PRECOMPILE_SAME_INSTRUCTION
        || message_instruction_index != ED25519_PRECOMPILE_SAME_INSTRUCTION
        || message_size != message.len()
    {
        return Err(InvalidSignedPrekeySignatureInstruction.into());
    }

    if !instruction_data_eq(instruction, signature_offset, signature)
        || !instruction_data_eq(instruction, public_key_offset, signer)
        || !instruction_data_eq(instruction, message_offset, message)
    {
        return Err(InvalidSignedPrekeySignatureInstruction.into());
    }

    Ok(())
}

fn instruction_data_eq(
    instruction: &crypto::ProcessedInstruction,
    offset: usize,
    expected: &[u8],
) -> bool {
    let Some(end) = offset.checked_add(expected.len()) else {
        return false;
    };
    end <= instruction.data_len && instruction.data[offset..end] == *expected
}

fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    let Some(end) = offset.checked_add(2) else {
        return Err(InvalidSignedPrekeySignatureInstruction.into());
    };
    if data.len() < end {
        return Err(InvalidSignedPrekeySignatureInstruction.into());
    }
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

#[allow(clippy::too_many_arguments)]
fn emit_prekey_event(
    tag: u8,
    owner: &Address,
    domain: &[u8; 32],
    identity_key: &[u8; 32],
    signed_prekey_id: u32,
    one_time_prekey_root: &[u8; 32],
    one_time_prekey_count: u16,
    timestamp: u64,
) {
    let signed_prekey_id_bytes = signed_prekey_id.to_le_bytes();
    let count_bytes = one_time_prekey_count.to_le_bytes();
    let timestamp_bytes = timestamp.to_le_bytes();
    hopper::events::emit_slices(&[
        &[tag],
        owner.as_bytes(),
        domain,
        identity_key,
        &signed_prekey_id_bytes,
        one_time_prekey_root,
        &count_bytes,
        &timestamp_bytes,
    ]);
}

fn proof_input(proof: &[u8], index: usize) -> Result<[u8; 32]> {
    if index >= STYX_ZK_PUBLIC_INPUTS {
        return Err(InvalidProofPublicInputCount.into());
    }
    let start = STYX_ZK_INPUTS_OFFSET + index * 32;
    copy_32(proof, start)
}

fn copy_32(data: &[u8], start: usize) -> Result<[u8; 32]> {
    let end = start
        .checked_add(32)
        .ok_or(ProgramError::InvalidInstructionData)?;
    if data.len() < end {
        return Err(InvalidProofLength.into());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&data[start..end]);
    Ok(out)
}

fn read_bn254_field_u64(field: &[u8; 32]) -> u64 {
    u64::from_be_bytes([
        field[24], field[25], field[26], field[27], field[28], field[29], field[30], field[31],
    ])
}

fn invoke_verifier(verifier_program: Address, proof: &[u8]) -> ProgramResult {
    let instruction_accounts: [cpi::InstructionAccount; 0] = [];
    let instruction = cpi::InstructionView {
        program_id: &verifier_program,
        data: proof,
        accounts: &instruction_accounts,
    };
    let account_views: [&AccountView; 0] = [];
    cpi::invoke::<0>(&instruction, &account_views)
}
