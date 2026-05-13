//! Bounded dynamic-tail port example: fixed vault + bounded multisig metadata.

#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::*;
use hopper::systems::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    #[role(authority)]
    pub authority: Address,

    #[role(balance)]
    pub balance: WireU64,

    #[role(bump)]
    pub bump: u8,
}

#[hopper::dynamic_account(disc = 7, version = 1)]
pub struct Multisig {
    #[role(threshold)]
    pub threshold: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,
}

pub fn initialize_multisig_data(
    data: &mut [u8],
    threshold: u64,
    label: &str,
    signers: &[Address],
) -> ProgramResult {
    if data.len() < Multisig::ALLOC_SPACE {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if threshold == 0 || threshold as usize > signers.len() {
        return Err(ProgramError::InvalidInstructionData);
    }

    init_header::<Multisig>(data)?;
    let body = Multisig::overlay_mut(&mut data[HopperHeader::SIZE..Multisig::TAIL_PREFIX_OFFSET])?;
    *body = Multisig::new(threshold);

    let tail = MultisigTail {
        label: HopperString::from_str(label)?,
        signers: HopperVec::from_slice(signers)?,
    };
    Multisig::tail_write(data, &tail)?;
    Ok(())
}

pub fn threshold_met(data: &[u8], approvals: &[Address]) -> Result<bool, ProgramError> {
    if data.len() < Multisig::TAIL_PREFIX_OFFSET {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let body = Multisig::overlay(&data[HopperHeader::SIZE..Multisig::TAIL_PREFIX_OFFSET])?;
    let needed = body.threshold() as usize;
    if needed == 0 {
        return Ok(false);
    }

    let signers = Multisig::signers(data)?;
    let mut approved = 0usize;
    for signer in signers {
        if approvals.iter().any(|candidate| candidate == signer) {
            approved += 1;
            if approved >= needed {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn rename_multisig_data(data: &mut [u8], label: &str) -> ProgramResult {
    Multisig::set_label(data, label)
}

pub fn add_signer_data(data: &mut [u8], signer: Address) -> ProgramResult {
    Multisig::push_unique_signer(data, signer).map(|_| ())
}

pub fn remove_signer_data(data: &mut [u8], signer: &Address) -> Result<bool, ProgramError> {
    Multisig::remove_signer(data, signer)
}

pub fn rename_multisig(multisig: &AccountView, label: &str) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    rename_multisig_data(&mut data, label)
}

pub fn add_signer(multisig: &AccountView, signer: Address) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    add_signer_data(&mut data, signer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_tail_roundtrips_label_and_signers() {
        let signer = Address::new([7u8; 32]);
        let mut tail = MultisigTail::default();
        tail.label.set_str("ops").unwrap();
        tail.signers.push(signer).unwrap();

        let mut data = [0u8; Multisig::ALLOC_SPACE];
        let written = Multisig::tail_write(&mut data, &tail).unwrap();
        assert_eq!(Multisig::tail_len(&data).unwrap(), written as u32);

        let view = Multisig::tail_view(&data).unwrap();
        assert_eq!(view.label().unwrap(), "ops");
        assert_eq!(view.signers().unwrap(), &[signer]);
    }

    #[test]
    fn multisig_data_helpers_cover_initialize_update_and_thresholds() {
        let signer_a = Address::new([1u8; 32]);
        let signer_b = Address::new([2u8; 32]);
        let signer_c = Address::new([3u8; 32]);
        let mut data = [0u8; Multisig::ALLOC_SPACE];

        initialize_multisig_data(&mut data, 2, "ops", &[signer_a, signer_b]).unwrap();
        assert!(!threshold_met(&data, &[signer_a]).unwrap());
        assert!(threshold_met(&data, &[signer_a, signer_b]).unwrap());

        rename_multisig_data(&mut data, "treasury").unwrap();
        add_signer_data(&mut data, signer_c).unwrap();
        add_signer_data(&mut data, signer_c).unwrap();

        assert_eq!(Multisig::label(&data).unwrap(), "treasury");
        assert_eq!(
            Multisig::signers(&data).unwrap(),
            &[signer_a, signer_b, signer_c]
        );

        assert!(remove_signer_data(&mut data, &signer_b).unwrap());
        assert!(!threshold_met(&data, &[signer_a, signer_b]).unwrap());
        assert!(threshold_met(&data, &[signer_a, signer_c]).unwrap());
    }
}
