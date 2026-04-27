//! Vote program — `BuiltinProgram` impl for
//! `Vote111111111111111111111111111111111111111`.
//!
//! ## Scope
//!
//! On mainnet the Vote program backs the validator gossip layer
//! and is operationally complex (vote-state versions, lockouts,
//! root slots, timestamp rewriting). Almost no application
//! program touches it directly — but it's still needed for
//! parity with `quasar-svm`'s test surface, which lets users
//! seed pre-vote accounts before delegating stake.
//!
//! Hopper's simulator covers the **administrative slice** —
//! account-shape ops that don't require modeling the full vote
//! lifecycle:
//!
//! - `InitializeAccount` (0)  — set node identity + auths
//! - `Authorize` (1)          — rotate voter or withdrawer
//! - `Withdraw` (3)           — pull lamports out
//! - `UpdateValidatorIdentity` (4) — change node pubkey
//! - `UpdateCommission` (5)   — change commission percent
//!
//! Vote-emitting variants (`Vote`, `VoteSwitch`,
//! `UpdateVoteState`, `UpdateVoteStateSwitch`,
//! `CompactUpdateVoteState`, `TowerSync`, `TowerSyncSwitch`,
//! `AuthorizeChecked`, `AuthorizeWithSeed`) return a clear
//! "unsupported variant" error — Phase 1 doesn't simulate the
//! TowerBFT lockout machinery.
//!
//! ## State layout
//!
//! Mainnet vote accounts use VoteStateVersion (currently V3).
//! The full state is 3,762 bytes including the per-slot lockout
//! ring buffer. Hopper models the **header slice** — the bytes
//! the administrative ops mutate — and zero-pads the rest:
//!
//! ```text
//! 0..4     version (u32 LE — 0=V0, 1=V1_14_11, 2=V2, 3=V3 = current)
//! 4..36    node_pubkey
//! 36..68   authorized_withdrawer
//! 68       commission (u8)
//! 69..71   pad to align next field
//! 71..    (versioned tail — Hopper leaves zeroed)
//! ```
//!
//! `authorized_voter` lives inside the V3 versioned tail (it's a
//! `BTreeMap<Epoch, Pubkey>` in upstream); since Phase 1 doesn't
//! tick epochs we model authorized_voter as a fixed slot at the
//! 80-byte mark for round-trip simplicity. This diverges from
//! upstream's exact bincode but is consistent within the
//! simulator and round-trips cleanly across Hopper-only flows.
//!
//! ## CU cost
//!
//! Mainnet vote operations are heterogeneous (votes are ~2,100
//! CU, admin ops are ~3,000). We pin 3,000 as the conservative
//! constant for the administrative slice.

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_sdk::pubkey::Pubkey;

/// Vote program ID.
pub const VOTE_PROGRAM_ID: Pubkey = solana_sdk::vote::program::id();

/// CU baseline for administrative ops.
const VOTE_INSTRUCTION_CU: u64 = 3_000;

/// Vote-state version 3 (the current mainnet shape).
const VOTE_STATE_VERSION_V3: u32 = 3;

/// Hopper-modeled minimum vote-account size. Mainnet uses 3,762;
/// we accept any size ≥ this since the administrative ops only
/// touch the header. Authors who need bit-exact mainnet sizing
/// can pre-allocate 3,762 bytes themselves.
pub const VOTE_ACCOUNT_HEADER_SIZE: usize = 112;

/// Offset of `authorized_voter` within the Hopper header. See
/// module docs for the divergence note.
const AUTHORIZED_VOTER_OFFSET: usize = 80;

/// Vote program — register via
/// [`crate::HopperSvm::with_vote_program`].
pub struct VoteProgramSimulator;

impl BuiltinProgram for VoteProgramSimulator {
    fn name(&self) -> &'static str {
        "vote"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        VOTE_INSTRUCTION_CU
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
                message: format!(
                    "vote: instruction data too short ({} bytes, need ≥ 4 for tag)",
                    data.len()
                ),
            });
        }
        let tag = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let body = &data[4..];
        match tag {
            0 => initialize_account(body, accounts, ctx),
            1 => authorize(body, accounts, ctx),
            3 => withdraw(body, accounts, ctx),
            4 => update_validator_identity(body, accounts, ctx),
            5 => update_commission(body, accounts, ctx),
            other => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "vote: variant tag {other} not supported by Hopper simulator \
                     (supported: 0/InitializeAccount, 1/Authorize, 3/Withdraw, \
                     4/UpdateValidatorIdentity, 5/UpdateCommission)"
                ),
            }),
        }
    }
}

// ────────── instruction handlers ──────────

/// `InitializeAccount` body: `VoteInit { node(32) | voter(32) |
/// withdrawer(32) | commission(u8) }`.
/// Accounts: `[vote (writable), rent (readonly), clock (readonly),
/// node (signer)]`.
fn initialize_account(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    let vote_addr = accounts[0].address;
    ctx.require_writable(&vote_addr)?;
    if accounts[0].data.len() < VOTE_ACCOUNT_HEADER_SIZE {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "vote::InitializeAccount: vote account is {} bytes, \
                 need ≥ {VOTE_ACCOUNT_HEADER_SIZE}",
                accounts[0].data.len()
            ),
        });
    }
    if read_version(&accounts[0].data) != 0 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "vote::InitializeAccount: account already initialized".into(),
        });
    }
    if body.len() < 32 + 32 + 32 + 1 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "vote::InitializeAccount: body has {} bytes, need 97 (VoteInit)",
                body.len()
            ),
        });
    }
    let node = read_pubkey(body, 0);
    let voter = read_pubkey(body, 32);
    let withdrawer = read_pubkey(body, 64);
    let commission = body[96];
    // The node identity must sign — mirrors mainnet.
    ctx.require_signer(&node)?;

    let rent_exempt = ctx.sysvars.rent.minimum_balance(accounts[0].data.len());
    if accounts[0].lamports < rent_exempt {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "vote::InitializeAccount: lamports {} < rent-exempt minimum {rent_exempt}",
                accounts[0].lamports
            ),
        });
    }

    // Wipe the header bytes and write the V3 layout. Tail bytes
    // (lockouts etc.) stay zeroed — Hopper doesn't simulate
    // TowerBFT.
    for b in accounts[0].data[..VOTE_ACCOUNT_HEADER_SIZE].iter_mut() {
        *b = 0;
    }
    write_version(&mut accounts[0].data, VOTE_STATE_VERSION_V3);
    accounts[0].data[4..36].copy_from_slice(node.as_ref());
    accounts[0].data[36..68].copy_from_slice(withdrawer.as_ref());
    accounts[0].data[68] = commission;
    accounts[0].data[AUTHORIZED_VOTER_OFFSET..AUTHORIZED_VOTER_OFFSET + 32]
        .copy_from_slice(voter.as_ref());

    ctx.log(format!(
        "vote::InitializeAccount: {vote_addr} (node={node}, voter={voter}, \
         withdrawer={withdrawer}, commission={commission})"
    ));
    Ok(())
}

/// `Authorize` body: `new_authority(32) | role(u32 — 0=Voter,
/// 1=Withdrawer)`.
/// Accounts: `[vote (writable), clock (readonly),
/// current_authority (signer)]`.
fn authorize(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    let vote_addr = accounts[0].address;
    ctx.require_writable(&vote_addr)?;
    require_initialized(&accounts[0].data, ctx)?;
    if body.len() < 32 + 4 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("vote::Authorize: body has {} bytes, need 36", body.len()),
        });
    }
    let new_auth = read_pubkey(body, 0);
    let role = u32::from_le_bytes(body[32..36].try_into().unwrap());
    match role {
        0 => {
            let current = read_pubkey(&accounts[0].data, AUTHORIZED_VOTER_OFFSET);
            ctx.require_signer(&current)?;
            accounts[0].data[AUTHORIZED_VOTER_OFFSET..AUTHORIZED_VOTER_OFFSET + 32]
                .copy_from_slice(new_auth.as_ref());
        }
        1 => {
            let current = read_pubkey(&accounts[0].data, 36);
            ctx.require_signer(&current)?;
            accounts[0].data[36..68].copy_from_slice(new_auth.as_ref());
        }
        other => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "vote::Authorize: role {other} unrecognised (0=Voter, 1=Withdrawer)"
                ),
            });
        }
    }
    ctx.log(format!(
        "vote::Authorize: {vote_addr} role={role} -> {new_auth}"
    ));
    Ok(())
}

/// `Withdraw` body: `lamports(u64)`.
/// Accounts: `[vote (writable), recipient (writable),
/// withdrawer (signer)]`.
fn withdraw(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    require_account(accounts, 1, ctx)?;
    require_initialized(&accounts[0].data, ctx)?;
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("vote::Withdraw: body has {} bytes, need 8", body.len()),
        });
    }
    let lamports = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let vote_addr = accounts[0].address;
    let recipient_addr = accounts[1].address;
    ctx.require_writable(&vote_addr)?;
    ctx.require_writable(&recipient_addr)?;
    let withdrawer = read_pubkey(&accounts[0].data, 36);
    ctx.require_signer(&withdrawer)?;
    let rent_exempt = ctx.sysvars.rent.minimum_balance(accounts[0].data.len());
    let available = accounts[0].lamports.saturating_sub(rent_exempt);
    if lamports > available {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "vote::Withdraw: requested {lamports} > available {available} \
                 (lamports={}, rent_exempt={rent_exempt})",
                accounts[0].lamports
            ),
        });
    }
    accounts[0].lamports -= lamports;
    accounts[1].lamports = accounts[1].lamports.saturating_add(lamports);
    if accounts[0].lamports == 0 {
        // Closing — wipe header so the account looks fresh.
        for b in accounts[0].data[..VOTE_ACCOUNT_HEADER_SIZE].iter_mut() {
            *b = 0;
        }
    }
    ctx.log(format!(
        "vote::Withdraw: {vote_addr} -> {recipient_addr} ({lamports} lamports)"
    ));
    Ok(())
}

/// `UpdateValidatorIdentity` body: empty.
/// Accounts: `[vote (writable), new_node (signer),
/// withdrawer (signer)]`.
fn update_validator_identity(
    _body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    require_account(accounts, 1, ctx)?;
    let vote_addr = accounts[0].address;
    ctx.require_writable(&vote_addr)?;
    require_initialized(&accounts[0].data, ctx)?;
    let new_node = accounts[1].address;
    ctx.require_signer(&new_node)?;
    let current_withdrawer = read_pubkey(&accounts[0].data, 36);
    ctx.require_signer(&current_withdrawer)?;
    accounts[0].data[4..36].copy_from_slice(new_node.as_ref());
    ctx.log(format!(
        "vote::UpdateValidatorIdentity: {vote_addr} -> {new_node}"
    ));
    Ok(())
}

/// `UpdateCommission` body: `commission(u8)`.
/// Accounts: `[vote (writable), withdrawer (signer)]`.
fn update_commission(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    let vote_addr = accounts[0].address;
    ctx.require_writable(&vote_addr)?;
    require_initialized(&accounts[0].data, ctx)?;
    if body.is_empty() {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "vote::UpdateCommission: body empty (need u8 commission)".into(),
        });
    }
    let new_commission = body[0];
    if new_commission > 100 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("vote::UpdateCommission: commission {new_commission} > 100"),
        });
    }
    let withdrawer = read_pubkey(&accounts[0].data, 36);
    ctx.require_signer(&withdrawer)?;
    accounts[0].data[68] = new_commission;
    ctx.log(format!(
        "vote::UpdateCommission: {vote_addr} -> {new_commission}%"
    ));
    Ok(())
}

// ────────── helpers ──────────

fn require_account(
    accounts: &[KeyedAccount],
    index: usize,
    _ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.len() <= index {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index,
            len: accounts.len(),
        });
    }
    Ok(())
}

fn read_pubkey(buf: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(buf[offset..offset + 32].try_into().unwrap())
}

fn read_version(data: &[u8]) -> u32 {
    if data.len() < 4 {
        return 0;
    }
    u32::from_le_bytes(data[0..4].try_into().unwrap())
}

fn write_version(data: &mut [u8], version: u32) {
    data[0..4].copy_from_slice(&version.to_le_bytes());
}

fn require_initialized(data: &[u8], ctx: &mut InvokeContext<'_>) -> Result<(), HopperSvmError> {
    if read_version(data) == 0 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "vote: account not initialized".into(),
        });
    }
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
    ) -> Result<(), HopperSvmError> {
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = VOTE_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        VoteProgramSimulator.invoke(&data, accounts, &mut ctx)
    }

    fn initialize_data(node: Pubkey, voter: Pubkey, withdrawer: Pubkey, commission: u8) -> Vec<u8> {
        let mut buf = vec![];
        buf.extend_from_slice(&0u32.to_le_bytes()); // tag
        buf.extend_from_slice(node.as_ref());
        buf.extend_from_slice(voter.as_ref());
        buf.extend_from_slice(withdrawer.as_ref());
        buf.push(commission);
        buf
    }

    #[test]
    fn initialize_writes_header() {
        let vote_addr = Pubkey::new_unique();
        let node = Pubkey::new_unique();
        let voter = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            vote_addr,
            10_000_000_000,
            VOTE_PROGRAM_ID,
            vec![0u8; VOTE_ACCOUNT_HEADER_SIZE],
            false,
        )];
        invoke(
            initialize_data(node, voter, withdrawer, 7),
            &mut accounts,
            metas(&[(vote_addr, false, true), (node, true, false)]),
        )
        .expect("InitializeAccount");
        assert_eq!(read_pubkey(&accounts[0].data, 4), node);
        assert_eq!(read_pubkey(&accounts[0].data, 36), withdrawer);
        assert_eq!(accounts[0].data[68], 7);
        assert_eq!(
            read_pubkey(&accounts[0].data, AUTHORIZED_VOTER_OFFSET),
            voter
        );
    }

    #[test]
    fn authorize_changes_voter() {
        let vote_addr = Pubkey::new_unique();
        let node = Pubkey::new_unique();
        let voter = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let new_voter = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            vote_addr,
            10_000_000_000,
            VOTE_PROGRAM_ID,
            vec![0u8; VOTE_ACCOUNT_HEADER_SIZE],
            false,
        )];
        invoke(
            initialize_data(node, voter, withdrawer, 0),
            &mut accounts,
            metas(&[(vote_addr, false, true), (node, true, false)]),
        )
        .expect("init");
        let mut data = vec![];
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(new_voter.as_ref());
        data.extend_from_slice(&0u32.to_le_bytes()); // role 0 = voter
        invoke(
            data,
            &mut accounts,
            metas(&[(vote_addr, false, true), (voter, true, false)]),
        )
        .expect("Authorize");
        assert_eq!(
            read_pubkey(&accounts[0].data, AUTHORIZED_VOTER_OFFSET),
            new_voter
        );
    }

    #[test]
    fn withdraw_respects_rent_exempt_floor() {
        let vote_addr = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let node = Pubkey::new_unique();
        let voter = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(
                vote_addr,
                10_000_000_000,
                VOTE_PROGRAM_ID,
                vec![0u8; VOTE_ACCOUNT_HEADER_SIZE],
                false,
            ),
            KeyedAccount::new(recipient, 0, Pubkey::default(), vec![], false),
        ];
        invoke(
            initialize_data(node, voter, withdrawer, 0),
            &mut accounts,
            metas(&[(vote_addr, false, true), (node, true, false)]),
        )
        .expect("init");
        // Try draining the whole vote account.
        let mut data = vec![];
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&10_000_000_000u64.to_le_bytes());
        let err = invoke(
            data,
            &mut accounts,
            metas(&[
                (vote_addr, false, true),
                (recipient, false, true),
                (withdrawer, true, false),
            ]),
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("requested"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    #[test]
    fn update_commission_clamped_to_100() {
        let vote_addr = Pubkey::new_unique();
        let node = Pubkey::new_unique();
        let voter = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            vote_addr,
            10_000_000_000,
            VOTE_PROGRAM_ID,
            vec![0u8; VOTE_ACCOUNT_HEADER_SIZE],
            false,
        )];
        invoke(
            initialize_data(node, voter, withdrawer, 0),
            &mut accounts,
            metas(&[(vote_addr, false, true), (node, true, false)]),
        )
        .expect("init");
        let mut data = vec![];
        data.extend_from_slice(&5u32.to_le_bytes());
        data.push(150); // > 100
        let err = invoke(
            data,
            &mut accounts,
            metas(&[(vote_addr, false, true), (withdrawer, true, false)]),
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("> 100"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    #[test]
    fn unsupported_variant_errors() {
        let vote_addr = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            vote_addr,
            1,
            VOTE_PROGRAM_ID,
            vec![0u8; VOTE_ACCOUNT_HEADER_SIZE],
            false,
        )];
        let mut data = vec![];
        data.extend_from_slice(&2u32.to_le_bytes()); // Vote — not supported
        let err = invoke(data, &mut accounts, metas(&[(vote_addr, false, true)])).unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("not supported"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }
}
