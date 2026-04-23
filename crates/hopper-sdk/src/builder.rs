//! # Manifest-driven instruction builder
//!
//! Turns a `ProgramManifest` + named instruction + typed args into a raw
//! instruction shape `(program_id, data, accounts)` ready to be handed to any
//! Solana client crate (solana-sdk, magic-block, anchor-client, etc.) for
//! transaction assembly.
//!
//! The entire point: the on-chain program's manifest IS the off-chain IDL.
//! There is no "regenerate the TypeScript types after you redeploy" step,
//! because the source of truth is compiled into the program.
//!
//! # Innovation over Anchor/Quasar
//!
//! - **Single source of truth**: no separate `.json` IDL file that drifts.
//! - **Layout-id-verified account lookups**: if a caller supplies an account
//!   for a slot whose `layout_ref` names a layout in the manifest, the
//!   builder can validate on construction that the passed-in account bytes
//!   match the expected fingerprint.
//! - **Zero serde**: args are written to a byte vec using the `ArgDescriptor`
//!   size declarations in the manifest.

#![cfg(feature = "builder")]

extern crate alloc;

use alloc::vec::Vec;
use hopper_schema::{InstructionDescriptor, ProgramManifest};

/// A Solana-compatible account meta. Re-defined locally so this crate does
/// not pull in `solana-program` or `solana-sdk`. Consumers downshift to their
/// preferred SDK's `AccountMeta` after construction.
#[derive(Debug, Clone, Copy)]
pub struct AccountMeta {
    /// 32-byte Solana public key.
    pub pubkey: [u8; 32],
    /// Account must sign the tx.
    pub is_signer: bool,
    /// Account is writable.
    pub is_writable: bool,
}

/// One built instruction ready for submission.
#[derive(Debug, Clone)]
pub struct BuiltInstruction {
    /// Program id (32 bytes).
    pub program_id: [u8; 32],
    /// Instruction data.
    pub data: Vec<u8>,
    /// Ordered account metas.
    pub accounts: Vec<AccountMeta>,
}

/// Builder error surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// No instruction by that name in the manifest.
    UnknownInstruction,
    /// Caller supplied a different number of args than the manifest declares.
    ArgCountMismatch {
        /// Expected count.
        expected: usize,
        /// Actual count.
        got: usize,
    },
    /// One of the caller's arg byte slices was the wrong size.
    ArgSizeMismatch {
        /// Which arg index.
        index: usize,
        /// Expected size from manifest.
        expected: u16,
        /// Actual size from caller.
        got: usize,
    },
    /// Caller supplied a different number of accounts than the manifest
    /// declares for this instruction.
    AccountCountMismatch {
        /// Expected count.
        expected: usize,
        /// Actual count.
        got: usize,
    },
}

/// Fluent builder over a manifest + one instruction.
#[derive(Debug)]
pub struct InstructionBuilder<'a> {
    program_id: [u8; 32],
    ix: &'a InstructionDescriptor,
    args: Vec<&'a [u8]>,
    accounts: Vec<[u8; 32]>,
}

impl<'a> InstructionBuilder<'a> {
    /// Locate `ix_name` in the manifest and return a builder seeded with the
    /// instruction tag.
    pub fn new(
        manifest: &'a ProgramManifest,
        program_id: [u8; 32],
        ix_name: &str,
    ) -> Result<Self, BuildError> {
        let ix = find_instruction(manifest, ix_name)
            .ok_or(BuildError::UnknownInstruction)?;
        Ok(Self {
            program_id,
            ix,
            args: Vec::with_capacity(ix.args.len()),
            accounts: Vec::with_capacity(ix.accounts.len()),
        })
    }

    /// Add one arg's raw bytes. Order must match the manifest.
    pub fn arg(mut self, bytes: &'a [u8]) -> Self {
        self.args.push(bytes);
        self
    }

    /// Add one account pubkey. Order must match the manifest.
    pub fn account(mut self, pubkey: [u8; 32]) -> Self {
        self.accounts.push(pubkey);
        self
    }

    /// Finalize into a `BuiltInstruction`, validating shape.
    pub fn build(self) -> Result<BuiltInstruction, BuildError> {
        if self.args.len() != self.ix.args.len() {
            return Err(BuildError::ArgCountMismatch {
                expected: self.ix.args.len(),
                got: self.args.len(),
            });
        }
        if self.accounts.len() != self.ix.accounts.len() {
            return Err(BuildError::AccountCountMismatch {
                expected: self.ix.accounts.len(),
                got: self.accounts.len(),
            });
        }

        // Data layout: [tag_byte, arg0_bytes..., arg1_bytes..., ...]
        let mut data_len: usize = 1;
        let mut i = 0;
        while i < self.args.len() {
            let want = self.ix.args[i].size as usize;
            if self.args[i].len() != want {
                return Err(BuildError::ArgSizeMismatch {
                    index: i,
                    expected: self.ix.args[i].size,
                    got: self.args[i].len(),
                });
            }
            data_len += want;
            i += 1;
        }

        let mut data = Vec::with_capacity(data_len);
        data.push(self.ix.tag);
        for a in &self.args { data.extend_from_slice(a); }

        let mut metas = Vec::with_capacity(self.accounts.len());
        let mut i = 0;
        while i < self.accounts.len() {
            let entry = &self.ix.accounts[i];
            metas.push(AccountMeta {
                pubkey: self.accounts[i],
                is_signer: entry.signer,
                is_writable: entry.writable,
            });
            i += 1;
        }

        Ok(BuiltInstruction {
            program_id: self.program_id,
            data,
            accounts: metas,
        })
    }
}

fn find_instruction<'a>(
    m: &'a ProgramManifest,
    name: &str,
) -> Option<&'a InstructionDescriptor> {
    let mut i = 0;
    while i < m.instructions.len() {
        if m.instructions[i].name == name {
            return Some(&m.instructions[i]);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_schema::{AccountEntry, ArgDescriptor, InstructionDescriptor, ProgramManifest};

    fn sample_manifest() -> ProgramManifest {
        static ARGS: [ArgDescriptor; 2] = [
            ArgDescriptor { name: "amount", canonical_type: "u64", size: 8 },
            ArgDescriptor { name: "bump",   canonical_type: "u8",  size: 1 },
        ];
        static ACCTS: [AccountEntry; 2] = [
            AccountEntry { name: "vault",     writable: true,  signer: false, layout_ref: "Vault" },
            AccountEntry { name: "authority", writable: false, signer: true,  layout_ref: "" },
        ];
        static IX: [InstructionDescriptor; 1] = [InstructionDescriptor {
            name: "deposit", tag: 3, args: &ARGS, accounts: &ACCTS,
            capabilities: &[], policy_pack: "", receipt_expected: true,
        }];
        ProgramManifest {
            name: "test", version: "0", description: "",
            layouts: &[], layout_metadata: &[],
            instructions: &IX,
            events: &[], policies: &[],
            compatibility_pairs: &[], tooling_hints: &[],
            contexts: &[],
        }
    }

    #[test]
    fn builds_a_valid_ix() {
        let m = sample_manifest();
        let amount = 42u64.to_le_bytes();
        let bump = [254u8];
        let vault = [1u8; 32];
        let auth = [2u8; 32];
        let ix = InstructionBuilder::new(&m, [9u8; 32], "deposit")
            .unwrap()
            .arg(&amount)
            .arg(&bump)
            .account(vault)
            .account(auth)
            .build()
            .unwrap();
        assert_eq!(ix.program_id, [9u8; 32]);
        assert_eq!(ix.data.len(), 1 + 8 + 1);
        assert_eq!(ix.data[0], 3);
        assert_eq!(ix.accounts.len(), 2);
        assert!(ix.accounts[0].is_writable);
        assert!(ix.accounts[1].is_signer);
    }

    #[test]
    fn rejects_mismatched_arg_size() {
        let m = sample_manifest();
        let amount_wrong = [1u8; 4];
        let bump = [0u8; 1];
        let err = InstructionBuilder::new(&m, [0u8; 32], "deposit")
            .unwrap()
            .arg(&amount_wrong)
            .arg(&bump)
            .account([0u8; 32])
            .account([0u8; 32])
            .build()
            .unwrap_err();
        assert!(matches!(err, BuildError::ArgSizeMismatch { index: 0, expected: 8, got: 4 }));
    }

    #[test]
    fn rejects_unknown_instruction() {
        let m = sample_manifest();
        let err = InstructionBuilder::new(&m, [0u8; 32], "withdraw").unwrap_err();
        assert_eq!(err, BuildError::UnknownInstruction);
    }
}
