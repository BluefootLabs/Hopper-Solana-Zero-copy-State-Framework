//! Address Lookup Table program — `BuiltinProgram` impl for
//! `AddressLookupTab1e1111111111111111111111111111`.
//!
//! The ALT program is a built-in (not BPF) on mainnet. Hopper
//! ships a Rust simulator covering the 5 standard instructions:
//!
//! | Tag | Instruction              |
//! |-----|--------------------------|
//! |  0  | `CreateLookupTable`      |
//! |  1  | `FreezeLookupTable`      |
//! |  2  | `ExtendLookupTable`      |
//! |  3  | `DeactivateLookupTable`  |
//! |  4  | `CloseLookupTable`       |
//!
//! ## Authority semantics
//!
//! - `Create` sets the table's authority to the supplied signer.
//! - `Freeze` clears the authority — the table becomes
//!   immutable (no Extend / Deactivate / Close after).
//! - `Extend`, `Deactivate`, `Close` require the stored
//!   authority to sign.
//!
//! ## Deactivation cooldown
//!
//! `Close` requires the table to have been `Deactivate`d AND
//! the cooldown to have elapsed
//! ([`crate::alt::DEACTIVATION_COOLDOWN_SLOTS`] = 513 slots).
//! Programs can advance the harness clock via
//! `HopperSvm::warp_to_slot(N)` to test the cooldown path.

use crate::account::KeyedAccount;
use crate::alt;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_sdk::pubkey::Pubkey;

/// Address Lookup Table program ID. Bound to the canonical
/// mainnet address.
pub const ALT_PROGRAM_ID: Pubkey = solana_sdk::address_lookup_table::program::id();

/// CU baseline. The ALT program is built-in on mainnet and
/// charges a fixed amount per instruction; we use 750 to
/// match the runtime's average.
const ALT_INSTRUCTION_CU: u64 = 750;

/// Address Lookup Table program reference simulator. Register
/// against [`ALT_PROGRAM_ID`] via
/// [`crate::HopperSvm::with_alt_program`].
pub struct AltProgramSimulator;

impl BuiltinProgram for AltProgramSimulator {
    fn name(&self) -> &'static str {
        "address-lookup-table"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        ALT_INSTRUCTION_CU
    }

    fn invoke(
        &self,
        data: &[u8],
        accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError> {
        if data.len() < 4 {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "alt: instruction data too short (need 4-byte tag)".to_string(),
            });
        }
        let tag = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let body = &data[4..];
        match tag {
            0 => create_lookup_table(body, accounts, ctx),
            1 => freeze_lookup_table(accounts, ctx),
            2 => extend_lookup_table(body, accounts, ctx),
            3 => deactivate_lookup_table(accounts, ctx),
            4 => close_lookup_table(accounts, ctx),
            other => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "alt: variant tag {other} not recognised \
                     (supported: 0/Create, 1/Freeze, 2/Extend, 3/Deactivate, 4/Close)"
                ),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tag 0 — CreateLookupTable { recent_slot, bump_seed }
// ---------------------------------------------------------------------------
//
// Body: recent_slot(u64 LE) + bump_seed(u8)
//
// The lookup-table address must equal
//   PDA(["authority", recent_slot.to_le_bytes()], ALT_PROGRAM_ID)
// derived with the supplied bump_seed.
//
// Accounts:
//   0: lookup_table (writable; address must match derivation)
//   1: authority (signer, read-only)
//   2: payer (signer, writable)
//   3: system_program (read-only)
fn create_lookup_table(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 9 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "alt::Create: body < 9 bytes (need recent_slot u64 + bump_seed u8)"
                .to_string(),
        });
    }
    let _recent_slot = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let bump_seed = body[8];
    if accounts.len() < 4 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 3,
            len: accounts.len(),
        });
    }
    let table_addr = accounts[0].address;
    let authority_addr = accounts[1].address;
    let payer_addr = accounts[2].address;
    ctx.require_writable(&table_addr)?;
    ctx.require_signer(&authority_addr)?;
    ctx.require_signer(&payer_addr)?;
    ctx.require_writable(&payer_addr)?;

    // Validate the address derivation. Mainnet uses the recent
    // slot's bytes as one of the PDA seeds; we mirror that
    // exactly via Pubkey::create_program_address.
    let seed_bytes_authority = authority_addr.to_bytes();
    let seed_bytes_slot = _recent_slot.to_le_bytes();
    let seed_bump = [bump_seed];
    let seeds: &[&[u8]] = &[&seed_bytes_authority, &seed_bytes_slot, &seed_bump];
    let derived = Pubkey::create_program_address(seeds, ctx.program_id).map_err(|err| {
        HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Create: PDA derivation failed: {err:?}"),
        }
    })?;
    if derived != table_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Create: address mismatch (got {table_addr}, derived {derived})"),
        });
    }

    // Account must be empty (system-owned, no data).
    if !accounts[0].data.is_empty() || accounts[0].lamports != 0 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Create: {table_addr} already initialised"),
        });
    }

    // Allocate the meta-only account; payer funds it. Mainnet
    // charges roughly 0.00203928 SOL for the rent-exempt
    // 56-byte header; we hard-code the value rather than
    // looking up rent here.
    let initial_lamports: u64 = 1_281_360;
    if accounts[2].lamports < initial_lamports {
        return Err(HopperSvmError::InsufficientFunds {
            account: payer_addr,
            balance: accounts[2].lamports,
            requested: initial_lamports,
        });
    }
    accounts[2].lamports -= initial_lamports;
    accounts[0].lamports = initial_lamports;
    accounts[0].data = vec![0u8; alt::LOOKUP_TABLE_META_SIZE];
    accounts[0].owner = *ctx.program_id;
    accounts[0].executable = false;

    let meta = alt::LookupTableMeta::new(authority_addr);
    alt::write_meta(&mut accounts[0].data, &meta);
    ctx.log(format!(
        "alt::CreateLookupTable: {table_addr} authority={authority_addr} (slot={})",
        _recent_slot
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 1 — FreezeLookupTable
// ---------------------------------------------------------------------------
//
// Accounts:
//   0: lookup_table (writable)
//   1: authority (signer)
fn freeze_lookup_table(
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }
    let table_addr = accounts[0].address;
    let auth_addr = accounts[1].address;
    ctx.require_writable(&table_addr)?;
    ctx.require_signer(&auth_addr)?;
    let mut meta =
        alt::read_meta(&accounts[0].data).ok_or_else(|| HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Freeze: {table_addr} not a valid lookup table"),
        })?;
    match meta.authority {
        Some(stored) if stored == auth_addr => {}
        Some(stored) => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("alt::Freeze: signer {auth_addr} != stored authority {stored}"),
            });
        }
        None => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("alt::Freeze: {table_addr} already frozen"),
            });
        }
    }
    meta.authority = None;
    alt::write_meta(&mut accounts[0].data, &meta);
    ctx.log(format!("alt::FreezeLookupTable: {table_addr}"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 2 — ExtendLookupTable { new_addresses: Vec<Pubkey> }
// ---------------------------------------------------------------------------
//
// Body: u64 LE address-count + N × 32-byte pubkeys
//
// Accounts:
//   0: lookup_table (writable)
//   1: authority (signer)
//   2: payer (signer, writable) — pays additional rent
//   3: system_program
fn extend_lookup_table(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "alt::Extend: body too short (need address count u64)".to_string(),
        });
    }
    let count = u64::from_le_bytes(body[0..8].try_into().unwrap()) as usize;
    let needed = 8 + count * 32;
    if body.len() < needed {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "alt::Extend: body has {} bytes, need {needed} for {count} addresses",
                body.len()
            ),
        });
    }
    let mut new_addresses: Vec<Pubkey> = Vec::with_capacity(count);
    for i in 0..count {
        let off = 8 + i * 32;
        new_addresses.push(Pubkey::new_from_array(
            body[off..off + 32].try_into().unwrap(),
        ));
    }

    if accounts.len() < 4 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 3,
            len: accounts.len(),
        });
    }
    let table_addr = accounts[0].address;
    let auth_addr = accounts[1].address;
    ctx.require_writable(&table_addr)?;
    ctx.require_signer(&auth_addr)?;

    let mut meta =
        alt::read_meta(&accounts[0].data).ok_or_else(|| HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Extend: {table_addr} not a valid lookup table"),
        })?;
    match meta.authority {
        Some(stored) if stored == auth_addr => {}
        Some(stored) => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("alt::Extend: signer {auth_addr} != stored authority {stored}"),
            });
        }
        None => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("alt::Extend: {table_addr} is frozen"),
            });
        }
    }
    if meta.is_deactivated() {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Extend: {table_addr} is deactivated"),
        });
    }

    let prev_count = alt::address_count(&accounts[0].data);
    alt::append_addresses(&mut accounts[0].data, &new_addresses).map_err(|err| {
        HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Extend: {err}"),
        }
    })?;
    meta.last_extended_slot = ctx.sysvars.clock.slot;
    meta.last_extended_slot_start_index = prev_count.min(255) as u8;
    alt::write_meta(&mut accounts[0].data, &meta);
    ctx.log(format!(
        "alt::ExtendLookupTable: {table_addr} +{count} addresses (now {})",
        prev_count + count
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 3 — DeactivateLookupTable
// ---------------------------------------------------------------------------
//
// Accounts:
//   0: lookup_table (writable)
//   1: authority (signer)
fn deactivate_lookup_table(
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }
    let table_addr = accounts[0].address;
    let auth_addr = accounts[1].address;
    ctx.require_writable(&table_addr)?;
    ctx.require_signer(&auth_addr)?;

    let mut meta =
        alt::read_meta(&accounts[0].data).ok_or_else(|| HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Deactivate: {table_addr} not a valid lookup table"),
        })?;
    match meta.authority {
        Some(stored) if stored == auth_addr => {}
        Some(stored) => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "alt::Deactivate: signer {auth_addr} != stored authority {stored}"
                ),
            });
        }
        None => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("alt::Deactivate: {table_addr} is frozen"),
            });
        }
    }
    if meta.is_deactivated() {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Deactivate: {table_addr} already deactivated"),
        });
    }
    meta.deactivation_slot = ctx.sysvars.clock.slot;
    alt::write_meta(&mut accounts[0].data, &meta);
    ctx.log(format!(
        "alt::DeactivateLookupTable: {table_addr} (slot {})",
        ctx.sysvars.clock.slot
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 4 — CloseLookupTable
// ---------------------------------------------------------------------------
//
// Accounts:
//   0: lookup_table (writable)
//   1: authority (signer)
//   2: recipient (writable; receives the table's lamports)
fn close_lookup_table(
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let table_addr = accounts[0].address;
    let auth_addr = accounts[1].address;
    let recipient_addr = accounts[2].address;
    ctx.require_writable(&table_addr)?;
    ctx.require_signer(&auth_addr)?;
    ctx.require_writable(&recipient_addr)?;

    let meta = alt::read_meta(&accounts[0].data).ok_or_else(|| HopperSvmError::BuiltinError {
        program_id: *ctx.program_id,
        message: format!("alt::Close: {table_addr} not a valid lookup table"),
    })?;
    match meta.authority {
        Some(stored) if stored == auth_addr => {}
        Some(stored) => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("alt::Close: signer {auth_addr} != stored authority {stored}"),
            });
        }
        None => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("alt::Close: {table_addr} is frozen and cannot be closed"),
            });
        }
    }
    if !meta.is_closeable(ctx.sysvars.clock.slot) {
        let reason = if !meta.is_deactivated() {
            "table is still active; call Deactivate first"
        } else {
            "deactivation cooldown has not elapsed"
        };
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("alt::Close: {table_addr} not closeable ({reason})"),
        });
    }

    // Move lamports out, zero the account, reassign to system.
    let lamports = accounts[0].lamports;
    accounts[2].lamports = accounts[2].lamports.saturating_add(lamports);
    accounts[0].lamports = 0;
    accounts[0].data = vec![];
    accounts[0].owner = solana_sdk::system_program::id();
    ctx.log(format!(
        "alt::CloseLookupTable: {table_addr} -> {recipient_addr} ({lamports} lamports)"
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogCapture;
    use crate::sysvar::Sysvars;
    use solana_sdk::instruction::AccountMeta;

    fn metas(addrs: &[(Pubkey, bool, bool)]) -> Vec<AccountMeta> {
        addrs
            .iter()
            .map(|(pk, signer, writable)| AccountMeta {
                pubkey: *pk,
                is_signer: *signer,
                is_writable: *writable,
            })
            .collect()
    }

    fn invoke(
        data: Vec<u8>,
        accounts: &mut Vec<KeyedAccount>,
        metas: Vec<AccountMeta>,
        slot: u64,
    ) -> Result<(), HopperSvmError> {
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let mut sysvars = Sysvars::default();
        sysvars.clock.slot = slot;
        let pid = ALT_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        AltProgramSimulator.invoke(&data, accounts, &mut ctx)
    }

    /// End-to-end happy path: create → extend → deactivate →
    /// (warp slot past cooldown) → close. Pin the full ALT
    /// state machine.
    #[test]
    fn full_alt_lifecycle() {
        let authority = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let recent_slot: u64 = 100;
        // Find a valid PDA + bump for (authority, recent_slot)
        // under the ALT program ID.
        let (table_addr, bump) = Pubkey::find_program_address(
            &[authority.as_ref(), &recent_slot.to_le_bytes()],
            &ALT_PROGRAM_ID,
        );

        let mut accounts = vec![
            KeyedAccount::new(
                table_addr,
                0,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                authority,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                payer,
                5_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                solana_sdk::system_program::id(),
                0,
                solana_sdk::system_program::id(),
                vec![],
                true,
            ),
        ];

        // Create.
        let mut data = vec![0u8, 0, 0, 0]; // tag 0
        data.extend_from_slice(&recent_slot.to_le_bytes());
        data.push(bump);
        invoke(
            data,
            &mut accounts,
            metas(&[
                (table_addr, false, true),
                (authority, true, false),
                (payer, true, true),
                (solana_sdk::system_program::id(), false, false),
            ]),
            recent_slot,
        )
        .expect("Create");
        assert_eq!(accounts[0].owner, ALT_PROGRAM_ID);
        let meta = alt::read_meta(&accounts[0].data).unwrap();
        assert_eq!(meta.authority, Some(authority));

        // Extend with 3 addresses.
        let new_addrs: Vec<Pubkey> = (0..3).map(|_| Pubkey::new_unique()).collect();
        let mut data = vec![2u8, 0, 0, 0];
        data.extend_from_slice(&3u64.to_le_bytes());
        for a in &new_addrs {
            data.extend_from_slice(a.as_ref());
        }
        invoke(
            data,
            &mut accounts,
            metas(&[
                (table_addr, false, true),
                (authority, true, false),
                (payer, true, true),
                (solana_sdk::system_program::id(), false, false),
            ]),
            recent_slot + 1,
        )
        .expect("Extend");
        assert_eq!(alt::address_count(&accounts[0].data), 3);
        assert_eq!(alt::read_address(&accounts[0].data, 0), Some(new_addrs[0]));

        // Deactivate at slot 200.
        invoke(
            vec![3u8, 0, 0, 0],
            &mut accounts,
            metas(&[(table_addr, false, true), (authority, true, false)]),
            200,
        )
        .expect("Deactivate");
        let meta = alt::read_meta(&accounts[0].data).unwrap();
        assert_eq!(meta.deactivation_slot, 200);

        // Close before cooldown — must fail.
        let mut closing_accounts = accounts.clone();
        closing_accounts.push(KeyedAccount::new(
            recipient,
            0,
            solana_sdk::system_program::id(),
            vec![],
            false,
        ));
        // The Close instruction's account list is [table, authority, recipient]
        // — slot 1 is authority, slot 2 is recipient. Use only those three.
        let mut close_only = vec![
            accounts[0].clone(),
            accounts[1].clone(),
            closing_accounts.last().unwrap().clone(),
        ];
        let err = invoke(
            vec![4u8, 0, 0, 0],
            &mut close_only,
            metas(&[
                (table_addr, false, true),
                (authority, true, false),
                (recipient, false, true),
            ]),
            200 + 100, // way before cooldown
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("not closeable"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }

        // Close after cooldown — succeeds.
        let close_slot = 200 + alt::DEACTIVATION_COOLDOWN_SLOTS + 1;
        invoke(
            vec![4u8, 0, 0, 0],
            &mut close_only,
            metas(&[
                (table_addr, false, true),
                (authority, true, false),
                (recipient, false, true),
            ]),
            close_slot,
        )
        .expect("Close");
        assert_eq!(close_only[0].lamports, 0);
        assert_eq!(close_only[0].owner, solana_sdk::system_program::id());
        assert!(close_only[2].lamports > 0); // recipient got the lamports
    }

    /// Freeze + Extend → Extend rejected (frozen tables can't
    /// be extended).
    #[test]
    fn freeze_blocks_extend() {
        let authority = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let recent_slot: u64 = 50;
        let (table_addr, bump) = Pubkey::find_program_address(
            &[authority.as_ref(), &recent_slot.to_le_bytes()],
            &ALT_PROGRAM_ID,
        );
        let mut accounts = vec![
            KeyedAccount::new(
                table_addr,
                0,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                authority,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                payer,
                5_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                solana_sdk::system_program::id(),
                0,
                solana_sdk::system_program::id(),
                vec![],
                true,
            ),
        ];
        let mut data = vec![0u8, 0, 0, 0];
        data.extend_from_slice(&recent_slot.to_le_bytes());
        data.push(bump);
        invoke(
            data,
            &mut accounts,
            metas(&[
                (table_addr, false, true),
                (authority, true, false),
                (payer, true, true),
                (solana_sdk::system_program::id(), false, false),
            ]),
            recent_slot,
        )
        .expect("Create");

        // Freeze.
        invoke(
            vec![1u8, 0, 0, 0],
            &mut accounts,
            metas(&[(table_addr, false, true), (authority, true, false)]),
            recent_slot + 1,
        )
        .expect("Freeze");
        let meta = alt::read_meta(&accounts[0].data).unwrap();
        assert!(meta.authority.is_none());

        // Try to Extend — must fail.
        let mut data = vec![2u8, 0, 0, 0];
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(Pubkey::new_unique().as_ref());
        let err = invoke(
            data,
            &mut accounts,
            metas(&[
                (table_addr, false, true),
                (authority, true, false),
                (payer, true, true),
                (solana_sdk::system_program::id(), false, false),
            ]),
            recent_slot + 2,
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("frozen"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }
}
