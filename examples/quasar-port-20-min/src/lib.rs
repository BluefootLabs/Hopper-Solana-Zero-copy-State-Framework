//! Bounded dynamic-tail port example: fixed vault + bounded multisig metadata.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

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

hopper_dynamic_tail! {
    pub struct MultisigTail {
        label: BoundedString<32>,
        signers: BoundedVec<Address, 10>,
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 7, version = 1, dynamic_tail = MultisigTail)]
pub struct Multisig {
    #[role(threshold)]
    pub threshold: WireU64,
}

impl Multisig {
    pub const ALLOC_SPACE: usize = Self::INIT_SPACE + 4 + MultisigTail::MAX_ENCODED_LEN;
}

pub fn rename_multisig(multisig: &AccountView, label: &str) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    let mut tail = Multisig::tail_read(&data)?;
    tail.label.set_str(label)?;
    Multisig::tail_write(&mut data, &tail)?;
    Ok(())
}

pub fn add_signer(multisig: &AccountView, signer: Address) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    let mut tail = Multisig::tail_read(&data)?;
    tail.signers.push(signer)?;
    Multisig::tail_write(&mut data, &tail)?;
    Ok(())
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

        let back = Multisig::tail_read(&data).unwrap();
        assert_eq!(back.label.as_str().unwrap(), "ops");
        assert_eq!(back.signers.as_slice(), &[signer]);
    }
}
