use hopper::prelude::*;
use hopper::systems::{init_header, HopperHeader};

#[hopper::account(discriminator = 8, version = 1)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Payout {
    pub multisig: Address,
    pub destination: Address,
    pub policy_epoch: WireU64,
    pub amount: WireU64,
    pub not_before: WireU64,
    pub expires: WireU64,
    pub executed: WireBool,
}

#[hopper::account(discriminator = 7, version = 2)]
pub struct Multisig<'a> {
    #[role(threshold)]
    pub threshold: u64,
    /// Incremented whenever membership or outstanding payout policy changes.
    pub policy_epoch: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
}

pub(crate) fn validate_members(threshold: u64, members: &[Address]) -> ProgramResult {
    if threshold == 0
        || threshold > members.len() as u64
        || members.len() > 10
        || members.iter().any(|key| *key == Address::new([0; 32]))
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    require_unique_members(members)
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
    validate_members(threshold, signers)?;

    // Validate all fallible input conversions before changing the buffer.
    let tail = MultisigTail {
        label: HopperString::from_str(label)?,
        signers: HopperVec::from_slice(signers)?,
    };

    init_header::<Multisig>(data)?;
    let body = Multisig::overlay_mut(&mut data[HopperHeader::SIZE..Multisig::TAIL_PREFIX_OFFSET])?;
    *body = Multisig::new(threshold, 0);

    Multisig::tail_write(data, &tail)?;
    Ok(())
}

/// Count distinct configured identities. The caller must authenticate approvals;
/// passing public keys to this data helper does not prove transaction signatures.
pub fn threshold_met(data: &[u8], approvals: &[Address]) -> Result<bool, ProgramError> {
    if data.len() < Multisig::TAIL_PREFIX_OFFSET {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let body = Multisig::overlay(&data[HopperHeader::SIZE..Multisig::TAIL_PREFIX_OFFSET])?;
    let needed = body.threshold();
    if needed == 0 {
        return Ok(false);
    }

    let signers = Multisig::signers(data)?;
    require_unique_members(signers)?;
    let mut approved = 0usize;
    for signer in signers {
        if approvals.iter().any(|candidate| candidate == signer) {
            approved += 1;
            if approved as u64 >= needed {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn require_unique_members(signers: &[Address]) -> ProgramResult {
    for (index, signer) in signers.iter().enumerate() {
        if signers[..index].contains(signer) {
            return Err(ProgramError::InvalidAccountData);
        }
    }
    Ok(())
}

pub fn rename_multisig_data(data: &mut [u8], label: &str) -> ProgramResult {
    Multisig::set_label(data, label)
}

pub fn add_signer_data(data: &mut [u8], signer: Address) -> ProgramResult {
    Multisig::push_unique_signer(data, signer).map(|_| ())
}

pub fn remove_signer_data(data: &mut [u8], signer: &Address) -> Result<bool, ProgramError> {
    let members = Multisig::signers(data)?;
    require_unique_members(members)?;
    if !members.contains(signer) {
        return Ok(false);
    }
    let body = Multisig::overlay(&data[HopperHeader::SIZE..Multisig::TAIL_PREFIX_OFFSET])?;
    if body.threshold() == 0 || body.threshold() > (members.len() - 1) as u64 {
        return Err(ProgramError::InvalidInstructionData);
    }
    Multisig::remove_signer(data, signer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_members_cannot_satisfy_a_threshold() {
        let a = Address::new([1; 32]);
        let b = Address::new([2; 32]);
        let mut data = [0u8; Multisig::ALLOC_SPACE];
        let before = data;
        assert!(initialize_multisig_data(&mut data, 2, "ops", &[a, a]).is_err());
        assert_eq!(data, before);
        initialize_multisig_data(&mut data, 2, "ops", &[a, b]).unwrap();
        assert!(!threshold_met(&data, &[a, a]).unwrap());
        let before = data;
        assert!(remove_signer_data(&mut data, &b).is_err());
        assert_eq!(data, before);
        // Reject old or malformed storage containing duplicate members too.
        Multisig::tail_write(
            &mut data,
            &MultisigTail {
                label: HopperString::from_str("ops").unwrap(),
                signers: HopperVec::from_slice(&[a, a]).unwrap(),
            },
        )
        .unwrap();
        assert!(threshold_met(&data, &[a]).is_err());
    }

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
