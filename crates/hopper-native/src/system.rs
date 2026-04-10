//! System program CPI instructions.
//!
//! Provides CreateAccount, Transfer, Assign, and Allocate builders
//! that invoke the system program via `sol_invoke_signed_c`.

use crate::account_view::AccountView;
use crate::address::Address;
use crate::instruction::{CpiAccount, Signer};
use crate::ProgramResult;

/// System program address: 11111111111111111111111111111111
pub const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

// ── CreateAccount ────────────────────────────────────────────────────

/// Builder for the system program's CreateAccount instruction.
pub struct CreateAccount<'a, 'b> {
    pub from: &'a AccountView,
    pub to: &'a AccountView,
    pub lamports: u64,
    pub space: u64,
    pub owner: &'b Address,
}

impl CreateAccount<'_, '_> {
    /// Invoke the CreateAccount instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the CreateAccount instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        // Instruction data: u32(0) + u64(lamports) + u64(space) + [u8;32](owner)
        let mut data = [0u8; 52];
        // index 0 = CreateAccount (already zero)
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());
        data[12..20].copy_from_slice(&self.space.to_le_bytes());
        data[20..52].copy_from_slice(self.owner.as_array());

        let accounts = [
            CpiAccount::from(self.from),
            CpiAccount::from(self.to),
        ];

        invoke_system(&data, &accounts, signers)
    }
}

// ── Transfer ─────────────────────────────────────────────────────────

/// Builder for the system program's Transfer instruction.
pub struct Transfer<'a> {
    pub from: &'a AccountView,
    pub to: &'a AccountView,
    pub lamports: u64,
}

impl Transfer<'_> {
    /// Invoke the Transfer instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the Transfer instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        // Instruction data: u32(2) + u64(lamports)
        let mut data = [0u8; 12];
        data[0] = 2;
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.from),
            CpiAccount::from(self.to),
        ];

        invoke_system(&data, &accounts, signers)
    }
}

// ── Assign ───────────────────────────────────────────────────────────

/// Builder for the system program's Assign instruction.
pub struct Assign<'a, 'b> {
    pub account: &'a AccountView,
    pub owner: &'b Address,
}

impl Assign<'_, '_> {
    /// Invoke the Assign instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the Assign instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        // Instruction data: u32(1) + [u8;32](owner)
        let mut data = [0u8; 36];
        data[0] = 1;
        data[4..36].copy_from_slice(self.owner.as_array());

        let accounts = [
            CpiAccount::from(self.account),
        ];

        invoke_system(&data, &accounts, signers)
    }
}

// ── Allocate ─────────────────────────────────────────────────────────

/// Builder for the system program's Allocate instruction.
pub struct Allocate<'a> {
    pub account: &'a AccountView,
    pub space: u64,
}

impl Allocate<'_> {
    /// Invoke the Allocate instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the Allocate instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        // Instruction data: u32(8) + u64(space)
        let mut data = [0u8; 12];
        data[0] = 8;
        data[4..12].copy_from_slice(&self.space.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.account),
        ];

        invoke_system(&data, &accounts, signers)
    }
}

// ── Internal helper ──────────────────────────────────────────────────

/// Build an InstructionView to the system program and invoke.
#[inline]
fn invoke_system(
    data: &[u8],
    accounts: &[CpiAccount],
    signers: &[Signer],
) -> ProgramResult {
    // Build an InstructionView to the system program and invoke via C ABI.
    #[cfg(target_os = "solana")]
    {
        let ix = crate::instruction::InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data,
            accounts: &[], // Not used by the C ABI path
        };
        let result = unsafe {
            crate::syscalls::sol_invoke_signed_c(
                &ix as *const _ as *const u8,
                accounts.as_ptr() as *const u8,
                accounts.len() as u64,
                signers.as_ptr() as *const u8,
                signers.len() as u64,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(crate::ProgramError::from(result))
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (data, accounts, signers);
        Ok(())
    }
}

/// Compatibility re-exports matching `pinocchio_system::instructions::*`.
pub mod instructions {
    pub use super::{CreateAccount, Transfer, Assign, Allocate};
}
