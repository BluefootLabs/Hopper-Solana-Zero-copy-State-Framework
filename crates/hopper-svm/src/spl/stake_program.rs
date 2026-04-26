//! Stake program — `BuiltinProgram` impl for
//! `Stake11111111111111111111111111111111111111`.
//!
//! ## Scope
//!
//! The on-chain Stake program has 16+ instruction variants spread
//! across years of validator-side feature work (split, merge,
//! redelegate, lockup, deactivating-delinquent, …). The Hopper
//! simulator covers the **lifecycle slice** — the variants
//! application programs realistically touch:
//!
//! - `Initialize` (0)        — set the staker / withdrawer / lockup
//! - `Authorize` (1)         — change the staker or withdrawer
//! - `DelegateStake` (2)     — bind the stake to a vote account
//! - `Withdraw` (4)          — pull lamports out (rent-exempt floor)
//! - `Deactivate` (5)        — start the cooldown
//!
//! Variants outside that slice (`Split`, `Merge`,
//! `AuthorizeWithSeed`, `InitializeChecked`, `AuthorizeChecked`,
//! `AuthorizeCheckedWithSeed`, `SetLockup`,
//! `SetLockupChecked`, `Redelegate`, `MoveStake`, `MoveLamports`,
//! `DeactivateDelinquent`) return a clear "unsupported variant"
//! error so tests that hit them fail fast with an actionable
//! message.
//!
//! ## State layout
//!
//! Solana's stake account is 200 bytes:
//!
//! ```text
//! offset  size  field
//! 0       4     state discriminator (u32 LE — 0=Uninitialized,
//!                                              1=Initialized,
//!                                              2=Stake,
//!                                              3=RewardsPool)
//! 4       16    Meta::rent_exempt_reserve (u64 + 8 bytes pad)
//!                  + plus Meta::authorized + Meta::lockup ……
//! ```
//!
//! Solana defines the on-chain shape via bincode-serialised enums.
//! Because Hopper does NOT depend on `solana-stake-program` (we
//! avoid pulling validator crates into a test harness), we model
//! the layout as a hand-coded `StakeAccountState` struct and
//! serialise/deserialise it by hand. The encoding matches the
//! upstream `solana_program::stake::state::StakeStateV2` bincode
//! shape so accounts authored by Hopper round-trip cleanly into
//! mainnet tooling.
//!
//! ## CU cost
//!
//! Mainnet charges ~750 CU per stake instruction (verified via
//! recent-block traces). We pin that as the constant cost since
//! the simulator's per-instruction logic is roughly equivalent to
//! upstream's BPF-free path.

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_sdk::pubkey::Pubkey;

/// Stake program ID.
pub const STAKE_PROGRAM_ID: Pubkey = solana_sdk::stake::program::id();

/// Mainnet pins ~750 CU for stake-program ops. Hold to that for
/// snapshot stability.
const STAKE_INSTRUCTION_CU: u64 = 750;

/// Stake account size (mainnet-canonical).
pub const STAKE_ACCOUNT_SIZE: usize = 200;

/// Discriminator: account hasn't been initialized yet. Reading the
/// state from the on-chain bincode encoding yields 0 here.
const DISCRIMINATOR_UNINITIALIZED: u32 = 0;
/// Discriminator: `Initialize` has been called, but no
/// `DelegateStake` yet — the account holds Meta only.
const DISCRIMINATOR_INITIALIZED: u32 = 1;
/// Discriminator: `DelegateStake` has been called — the account
/// holds Meta + Stake (delegation info).
const DISCRIMINATOR_STAKE: u32 = 2;

/// On-chain stake authority pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StakeAuthorized {
    /// May call `DelegateStake` and `Deactivate`.
    pub staker: Pubkey,
    /// May call `Withdraw` and rotate authorities.
    pub withdrawer: Pubkey,
}

/// On-chain lockup. Hopper supports the field for round-tripping;
/// the simulator never actively enforces lockup since Phase 1
/// targets unit tests over time-dependent contracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StakeLockup {
    /// Earliest unix-ts a withdrawer can move lamports out.
    pub unix_timestamp: i64,
    /// Earliest epoch a withdrawer can move lamports out.
    pub epoch: u64,
    /// Custodian who can override the lockup.
    pub custodian: Pubkey,
}

/// On-chain stake meta — present in every initialized state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StakeMeta {
    /// Lamports below this number are reserved for rent and can't
    /// be withdrawn.
    pub rent_exempt_reserve: u64,
    /// Staker / withdrawer.
    pub authorized: StakeAuthorized,
    /// Lockup info (rarely active in tests).
    pub lockup: StakeLockup,
}

/// Active delegation, added to Meta when the stake is actually
/// staking to a vote account. Mirrors upstream `Delegation`.
///
/// `warmup_cooldown_rate` is `f64` to match the upstream Solana
/// shape, so this struct is `PartialEq` only (not `Eq`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StakeDelegation {
    /// Vote account this stake is bound to.
    pub voter_pubkey: Pubkey,
    /// Number of lamports delegated.
    pub stake: u64,
    /// Activation epoch.
    pub activation_epoch: u64,
    /// Deactivation epoch (`u64::MAX` while active — mirrors
    /// upstream).
    pub deactivation_epoch: u64,
    /// Warmup-cooldown rate (legacy field — pinned to `0.25` as
    /// `f64`).
    pub warmup_cooldown_rate: f64,
}

impl Default for StakeDelegation {
    fn default() -> Self {
        Self {
            voter_pubkey: Pubkey::default(),
            stake: 0,
            activation_epoch: 0,
            deactivation_epoch: u64::MAX,
            warmup_cooldown_rate: 0.25,
        }
    }
}

/// Logical state of a stake account.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StakeAccountState {
    /// Discriminator 0.
    Uninitialized,
    /// Discriminator 1: meta but no delegation.
    Initialized(StakeMeta),
    /// Discriminator 2: meta + delegation. Solana's representation
    /// also has a `credits_observed` counter; we surface it for
    /// round-tripping but the simulator never increments it.
    Stake(StakeMeta, StakeDelegation, u64),
}

/// Stake program — register via
/// [`crate::HopperSvm::with_stake_program`].
pub struct StakeProgramSimulator;

impl BuiltinProgram for StakeProgramSimulator {
    fn name(&self) -> &'static str {
        "stake"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        STAKE_INSTRUCTION_CU
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
                    "stake: instruction data too short ({} bytes, need ≥ 4 for tag)",
                    data.len()
                ),
            });
        }
        let tag = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let body = &data[4..];
        match tag {
            0 => initialize(body, accounts, ctx),
            1 => authorize(body, accounts, ctx),
            2 => delegate_stake(body, accounts, ctx),
            4 => withdraw(body, accounts, ctx),
            5 => deactivate(body, accounts, ctx),
            other => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "stake: variant tag {other} not supported by Hopper simulator \
                     (supported: 0/Initialize, 1/Authorize, 2/DelegateStake, \
                     4/Withdraw, 5/Deactivate)"
                ),
            }),
        }
    }
}

// ────────── instruction handlers ──────────

/// `Initialize` body: `Authorized { staker(32) | withdrawer(32) } |
/// Lockup { ts(i64) | epoch(u64) | custodian(32) }`.
/// Accounts: `[stake (writable), rent-sysvar (readonly)]`.
fn initialize(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    let stake_addr = accounts[0].address;
    ctx.require_writable(&stake_addr)?;
    if accounts[0].data.len() != STAKE_ACCOUNT_SIZE {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "stake::Initialize: stake account is {} bytes, need {STAKE_ACCOUNT_SIZE}",
                accounts[0].data.len()
            ),
        });
    }
    if read_discriminator(&accounts[0].data) != DISCRIMINATOR_UNINITIALIZED {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "stake::Initialize: account already initialized".into(),
        });
    }
    if body.len() < 32 + 32 + 8 + 8 + 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "stake::Initialize: body has {} bytes, need 112 (Authorized + Lockup)",
                body.len()
            ),
        });
    }
    let staker = read_pubkey(body, 0);
    let withdrawer = read_pubkey(body, 32);
    let lockup_ts = i64::from_le_bytes(body[64..72].try_into().unwrap());
    let lockup_epoch = u64::from_le_bytes(body[72..80].try_into().unwrap());
    let custodian = read_pubkey(body, 80);

    let rent_exempt = ctx.sysvars.rent.minimum_balance(STAKE_ACCOUNT_SIZE);
    if accounts[0].lamports < rent_exempt {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "stake::Initialize: stake account lamports {} < rent-exempt minimum {rent_exempt}",
                accounts[0].lamports
            ),
        });
    }

    let meta = StakeMeta {
        rent_exempt_reserve: rent_exempt,
        authorized: StakeAuthorized { staker, withdrawer },
        lockup: StakeLockup {
            unix_timestamp: lockup_ts,
            epoch: lockup_epoch,
            custodian,
        },
    };
    write_state(&mut accounts[0].data, &StakeAccountState::Initialized(meta));
    ctx.log(format!(
        "stake::Initialize: {stake_addr} (staker={staker}, withdrawer={withdrawer})"
    ));
    Ok(())
}

/// `Authorize` body: `new_authority(32) | role(u32 — 0=Staker,
/// 1=Withdrawer)`.
/// Accounts: `[stake (writable), clock-sysvar (readonly),
/// current_authority (signer)]`.
fn authorize(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    let stake_addr = accounts[0].address;
    ctx.require_writable(&stake_addr)?;
    if body.len() < 32 + 4 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "stake::Authorize: body has {} bytes, need 36",
                body.len()
            ),
        });
    }
    let new_auth = read_pubkey(body, 0);
    let role = u32::from_le_bytes(body[32..36].try_into().unwrap());
    let mut state = read_state(&accounts[0].data, ctx)?;
    let meta_mut = match &mut state {
        StakeAccountState::Initialized(meta) => meta,
        StakeAccountState::Stake(meta, _, _) => meta,
        StakeAccountState::Uninitialized => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "stake::Authorize: account uninitialized".into(),
            });
        }
    };
    match role {
        0 => {
            ctx.require_signer(&meta_mut.authorized.staker)?;
            meta_mut.authorized.staker = new_auth;
        }
        1 => {
            ctx.require_signer(&meta_mut.authorized.withdrawer)?;
            meta_mut.authorized.withdrawer = new_auth;
        }
        other => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "stake::Authorize: role {other} unrecognised (0=Staker, 1=Withdrawer)"
                ),
            });
        }
    }
    write_state(&mut accounts[0].data, &state);
    ctx.log(format!(
        "stake::Authorize: {stake_addr} role={role} -> {new_auth}"
    ));
    Ok(())
}

/// `DelegateStake` body: empty (everything comes from accounts).
/// Accounts: `[stake (writable), vote (readonly), clock (readonly),
/// stake_history (readonly), config (readonly), staker (signer)]`.
fn delegate_stake(
    _body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    require_account(accounts, 1, ctx)?;
    let stake_addr = accounts[0].address;
    let vote_addr = accounts[1].address;
    ctx.require_writable(&stake_addr)?;
    let stake_lamports = accounts[0].lamports;
    let activation_epoch = ctx.sysvars.clock.epoch;
    let new_state = match read_state(&accounts[0].data, ctx)? {
        StakeAccountState::Initialized(meta) => {
            ctx.require_signer(&meta.authorized.staker)?;
            let stakeable = stake_lamports.saturating_sub(meta.rent_exempt_reserve);
            let delegation = StakeDelegation {
                voter_pubkey: vote_addr,
                stake: stakeable,
                activation_epoch,
                deactivation_epoch: u64::MAX,
                warmup_cooldown_rate: 0.25,
            };
            StakeAccountState::Stake(meta, delegation, 0)
        }
        StakeAccountState::Stake(meta, mut delegation, credits) => {
            // Re-delegating: simulator accepts the rebind. The
            // mainnet validator imposes a cooldown check; Hopper
            // skips that since unit tests typically don't run
            // multiple epochs.
            ctx.require_signer(&meta.authorized.staker)?;
            let stakeable = stake_lamports.saturating_sub(meta.rent_exempt_reserve);
            delegation.voter_pubkey = vote_addr;
            delegation.stake = stakeable;
            delegation.activation_epoch = activation_epoch;
            delegation.deactivation_epoch = u64::MAX;
            StakeAccountState::Stake(meta, delegation, credits)
        }
        StakeAccountState::Uninitialized => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "stake::DelegateStake: account uninitialized".into(),
            });
        }
    };
    write_state(&mut accounts[0].data, &new_state);
    ctx.log(format!(
        "stake::DelegateStake: {stake_addr} -> vote {vote_addr} epoch={activation_epoch}"
    ));
    Ok(())
}

/// `Withdraw` body: `lamports(u64)`.
/// Accounts: `[stake (writable), recipient (writable),
/// clock (readonly), stake_history (readonly), withdrawer
/// (signer), … optional custodian]`.
fn withdraw(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    require_account(accounts, 1, ctx)?;
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("stake::Withdraw: body has {} bytes, need 8", body.len()),
        });
    }
    let lamports = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let stake_addr = accounts[0].address;
    let recipient_addr = accounts[1].address;
    ctx.require_writable(&stake_addr)?;
    ctx.require_writable(&recipient_addr)?;
    let state = read_state(&accounts[0].data, ctx)?;
    let withdrawer = match &state {
        StakeAccountState::Initialized(meta) => meta.authorized.withdrawer,
        StakeAccountState::Stake(meta, _, _) => meta.authorized.withdrawer,
        StakeAccountState::Uninitialized => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "stake::Withdraw: account uninitialized".into(),
            });
        }
    };
    ctx.require_signer(&withdrawer)?;
    let rent_exempt_reserve = match &state {
        StakeAccountState::Initialized(meta) => meta.rent_exempt_reserve,
        StakeAccountState::Stake(meta, _, _) => meta.rent_exempt_reserve,
        StakeAccountState::Uninitialized => 0,
    };
    // Active-stake check: while delegated and not fully cooled, can
    // only withdraw above rent-exempt reserve + active stake.
    let locked_floor = match &state {
        StakeAccountState::Stake(_, delegation, _) => {
            // Crude: a fully-deactivated stake has
            // `deactivation_epoch <= clock.epoch` AND has cooled
            // for one epoch. Mirror upstream by requiring the
            // current epoch to be strictly past deactivation.
            if delegation.deactivation_epoch == u64::MAX
                || ctx.sysvars.clock.epoch <= delegation.deactivation_epoch
            {
                rent_exempt_reserve.saturating_add(delegation.stake)
            } else {
                rent_exempt_reserve
            }
        }
        _ => rent_exempt_reserve,
    };
    let available = accounts[0].lamports.saturating_sub(locked_floor);
    if lamports > available {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "stake::Withdraw: requested {lamports} > available {available} \
                 (lamports={}, locked_floor={locked_floor})",
                accounts[0].lamports
            ),
        });
    }
    accounts[0].lamports -= lamports;
    accounts[1].lamports = accounts[1].lamports.saturating_add(lamports);
    // If the withdraw drained the stake to zero, reset to
    // Uninitialized — matches upstream cleanup.
    if accounts[0].lamports == 0 {
        accounts[0].data.fill(0);
    }
    ctx.log(format!(
        "stake::Withdraw: {stake_addr} -> {recipient_addr} ({lamports} lamports)"
    ));
    Ok(())
}

/// `Deactivate` body: empty.
/// Accounts: `[stake (writable), clock (readonly), staker (signer)]`.
fn deactivate(
    _body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    require_account(accounts, 0, ctx)?;
    let stake_addr = accounts[0].address;
    ctx.require_writable(&stake_addr)?;
    let mut state = read_state(&accounts[0].data, ctx)?;
    match &mut state {
        StakeAccountState::Stake(meta, delegation, _) => {
            ctx.require_signer(&meta.authorized.staker)?;
            if delegation.deactivation_epoch != u64::MAX {
                return Err(HopperSvmError::BuiltinError {
                    program_id: *ctx.program_id,
                    message: format!(
                        "stake::Deactivate: stake already deactivating at epoch {}",
                        delegation.deactivation_epoch
                    ),
                });
            }
            delegation.deactivation_epoch = ctx.sysvars.clock.epoch;
        }
        StakeAccountState::Initialized(_) | StakeAccountState::Uninitialized => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "stake::Deactivate: account is not delegated".into(),
            });
        }
    }
    write_state(&mut accounts[0].data, &state);
    ctx.log(format!(
        "stake::Deactivate: {stake_addr} epoch={}",
        ctx.sysvars.clock.epoch
    ));
    Ok(())
}

// ────────── encode / decode helpers ──────────

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

fn read_discriminator(data: &[u8]) -> u32 {
    if data.len() < 4 {
        return DISCRIMINATOR_UNINITIALIZED;
    }
    u32::from_le_bytes(data[0..4].try_into().unwrap())
}

/// Decode a stake account.
///
/// Layout (mirrors `solana_program::stake::state::StakeStateV2`'s
/// bincode encoding):
///
/// ```text
/// 0..4    discriminator (u32 LE)
/// 4..12   meta.rent_exempt_reserve (u64)
/// 12..44  meta.authorized.staker
/// 44..76  meta.authorized.withdrawer
/// 76..84  meta.lockup.unix_timestamp (i64)
/// 84..92  meta.lockup.epoch (u64)
/// 92..124 meta.lockup.custodian
/// 124..156 delegation.voter_pubkey
/// 156..164 delegation.stake (u64)
/// 164..172 delegation.activation_epoch (u64)
/// 172..180 delegation.deactivation_epoch (u64)
/// 180..188 delegation.warmup_cooldown_rate (f64 LE)
/// 188..196 credits_observed (u64)
/// 196..200 padding to 200
/// ```
pub fn read_state(
    data: &[u8],
    ctx: &mut InvokeContext<'_>,
) -> Result<StakeAccountState, HopperSvmError> {
    if data.len() < STAKE_ACCOUNT_SIZE {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "stake: account data is {} bytes, need {STAKE_ACCOUNT_SIZE}",
                data.len()
            ),
        });
    }
    match read_discriminator(data) {
        DISCRIMINATOR_UNINITIALIZED => Ok(StakeAccountState::Uninitialized),
        DISCRIMINATOR_INITIALIZED => {
            let meta = read_meta(data);
            Ok(StakeAccountState::Initialized(meta))
        }
        DISCRIMINATOR_STAKE => {
            let meta = read_meta(data);
            let voter = read_pubkey(data, 124);
            let stake = u64::from_le_bytes(data[156..164].try_into().unwrap());
            let activation_epoch = u64::from_le_bytes(data[164..172].try_into().unwrap());
            let deactivation_epoch = u64::from_le_bytes(data[172..180].try_into().unwrap());
            let warmup_cooldown_rate =
                f64::from_le_bytes(data[180..188].try_into().unwrap());
            let credits = u64::from_le_bytes(data[188..196].try_into().unwrap());
            Ok(StakeAccountState::Stake(
                meta,
                StakeDelegation {
                    voter_pubkey: voter,
                    stake,
                    activation_epoch,
                    deactivation_epoch,
                    warmup_cooldown_rate,
                },
                credits,
            ))
        }
        other => Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("stake: unknown discriminator {other}"),
        }),
    }
}

fn read_meta(data: &[u8]) -> StakeMeta {
    let rent_exempt_reserve = u64::from_le_bytes(data[4..12].try_into().unwrap());
    let staker = read_pubkey(data, 12);
    let withdrawer = read_pubkey(data, 44);
    let unix_timestamp = i64::from_le_bytes(data[76..84].try_into().unwrap());
    let epoch = u64::from_le_bytes(data[84..92].try_into().unwrap());
    let custodian = read_pubkey(data, 92);
    StakeMeta {
        rent_exempt_reserve,
        authorized: StakeAuthorized { staker, withdrawer },
        lockup: StakeLockup {
            unix_timestamp,
            epoch,
            custodian,
        },
    }
}

/// Encode a stake account back into its 200-byte representation.
/// Zero-pads any tail bytes to keep the account size invariant.
pub fn write_state(data: &mut [u8], state: &StakeAccountState) {
    debug_assert!(data.len() >= STAKE_ACCOUNT_SIZE);
    // Zero everything first so unused fields don't leak across
    // state transitions.
    for b in data.iter_mut().take(STAKE_ACCOUNT_SIZE) {
        *b = 0;
    }
    match state {
        StakeAccountState::Uninitialized => {
            // Discriminator already zero from the fill above.
        }
        StakeAccountState::Initialized(meta) => {
            data[0..4].copy_from_slice(&DISCRIMINATOR_INITIALIZED.to_le_bytes());
            write_meta(data, meta);
        }
        StakeAccountState::Stake(meta, delegation, credits) => {
            data[0..4].copy_from_slice(&DISCRIMINATOR_STAKE.to_le_bytes());
            write_meta(data, meta);
            data[124..156].copy_from_slice(delegation.voter_pubkey.as_ref());
            data[156..164].copy_from_slice(&delegation.stake.to_le_bytes());
            data[164..172].copy_from_slice(&delegation.activation_epoch.to_le_bytes());
            data[172..180].copy_from_slice(&delegation.deactivation_epoch.to_le_bytes());
            data[180..188].copy_from_slice(&delegation.warmup_cooldown_rate.to_le_bytes());
            data[188..196].copy_from_slice(&credits.to_le_bytes());
        }
    }
}

fn write_meta(data: &mut [u8], meta: &StakeMeta) {
    data[4..12].copy_from_slice(&meta.rent_exempt_reserve.to_le_bytes());
    data[12..44].copy_from_slice(meta.authorized.staker.as_ref());
    data[44..76].copy_from_slice(meta.authorized.withdrawer.as_ref());
    data[76..84].copy_from_slice(&meta.lockup.unix_timestamp.to_le_bytes());
    data[84..92].copy_from_slice(&meta.lockup.epoch.to_le_bytes());
    data[92..124].copy_from_slice(meta.lockup.custodian.as_ref());
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

    fn invoke_with_sysvars(
        data: Vec<u8>,
        accounts: &mut Vec<KeyedAccount>,
        metas: Vec<AccountMeta>,
        sysvars: Sysvars,
    ) -> Result<(), HopperSvmError> {
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let pid = STAKE_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        StakeProgramSimulator.invoke(&data, accounts, &mut ctx)
    }

    fn invoke(
        data: Vec<u8>,
        accounts: &mut Vec<KeyedAccount>,
        metas: Vec<AccountMeta>,
    ) -> Result<(), HopperSvmError> {
        invoke_with_sysvars(data, accounts, metas, Sysvars::default())
    }

    fn build_initialize(
        staker: Pubkey,
        withdrawer: Pubkey,
        custodian: Pubkey,
    ) -> Vec<u8> {
        let mut buf = vec![];
        buf.extend_from_slice(&0u32.to_le_bytes()); // tag
        buf.extend_from_slice(staker.as_ref());
        buf.extend_from_slice(withdrawer.as_ref());
        buf.extend_from_slice(&0i64.to_le_bytes()); // lockup ts
        buf.extend_from_slice(&0u64.to_le_bytes()); // lockup epoch
        buf.extend_from_slice(custodian.as_ref());
        buf
    }

    #[test]
    fn initialize_writes_meta() {
        let stake_addr = Pubkey::new_unique();
        let staker = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            stake_addr,
            10_000_000_000,
            STAKE_PROGRAM_ID,
            vec![0u8; STAKE_ACCOUNT_SIZE],
            false,
        )];
        let data = build_initialize(staker, withdrawer, Pubkey::default());
        invoke(data, &mut accounts, metas(&[(stake_addr, false, true)])).expect("Initialize");
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = STAKE_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &[],
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        match read_state(&accounts[0].data, &mut ctx).unwrap() {
            StakeAccountState::Initialized(meta) => {
                assert_eq!(meta.authorized.staker, staker);
                assert_eq!(meta.authorized.withdrawer, withdrawer);
            }
            other => panic!("expected Initialized, got {other:?}"),
        }
    }

    #[test]
    fn delegate_then_deactivate() {
        let stake_addr = Pubkey::new_unique();
        let vote_addr = Pubkey::new_unique();
        let staker = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(
                stake_addr,
                10_000_000_000,
                STAKE_PROGRAM_ID,
                vec![0u8; STAKE_ACCOUNT_SIZE],
                false,
            ),
            KeyedAccount::new(vote_addr, 1_000_000, Pubkey::default(), vec![], false),
        ];
        let init = build_initialize(staker, withdrawer, Pubkey::default());
        invoke(init, &mut accounts, metas(&[(stake_addr, false, true)]))
            .expect("Initialize");
        // Delegate
        let mut delegate = vec![];
        delegate.extend_from_slice(&2u32.to_le_bytes());
        invoke(
            delegate,
            &mut accounts,
            metas(&[
                (stake_addr, false, true),
                (vote_addr, false, false),
                (staker, true, false),
            ]),
        )
        .expect("DelegateStake");
        // Deactivate
        let mut deactivate = vec![];
        deactivate.extend_from_slice(&5u32.to_le_bytes());
        let mut sysvars = Sysvars::default();
        sysvars.clock.epoch = 5;
        invoke_with_sysvars(
            deactivate,
            &mut accounts,
            metas(&[(stake_addr, false, true), (staker, true, false)]),
            sysvars,
        )
        .expect("Deactivate");
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = STAKE_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &[],
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        match read_state(&accounts[0].data, &mut ctx).unwrap() {
            StakeAccountState::Stake(meta, delegation, _) => {
                assert_eq!(meta.authorized.staker, staker);
                assert_eq!(delegation.voter_pubkey, vote_addr);
                assert_eq!(delegation.deactivation_epoch, 5);
            }
            other => panic!("expected Stake, got {other:?}"),
        }
    }

    #[test]
    fn authorize_changes_role() {
        let stake_addr = Pubkey::new_unique();
        let staker = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let new_staker = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            stake_addr,
            10_000_000_000,
            STAKE_PROGRAM_ID,
            vec![0u8; STAKE_ACCOUNT_SIZE],
            false,
        )];
        invoke(
            build_initialize(staker, withdrawer, Pubkey::default()),
            &mut accounts,
            metas(&[(stake_addr, false, true)]),
        )
        .expect("Initialize");
        let mut data = vec![];
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(new_staker.as_ref());
        data.extend_from_slice(&0u32.to_le_bytes()); // role 0 = staker
        invoke(
            data,
            &mut accounts,
            metas(&[(stake_addr, false, true), (staker, true, false)]),
        )
        .expect("Authorize");
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = STAKE_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &[],
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        match read_state(&accounts[0].data, &mut ctx).unwrap() {
            StakeAccountState::Initialized(meta) => {
                assert_eq!(meta.authorized.staker, new_staker);
                assert_eq!(meta.authorized.withdrawer, withdrawer);
            }
            other => panic!("wrong state: {other:?}"),
        }
    }

    #[test]
    fn withdraw_respects_locked_floor() {
        let stake_addr = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let staker = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(
                stake_addr,
                10_000_000_000,
                STAKE_PROGRAM_ID,
                vec![0u8; STAKE_ACCOUNT_SIZE],
                false,
            ),
            KeyedAccount::new(recipient, 0, Pubkey::default(), vec![], false),
        ];
        invoke(
            build_initialize(staker, withdrawer, Pubkey::default()),
            &mut accounts,
            metas(&[(stake_addr, false, true)]),
        )
        .expect("Initialize");
        // Try to withdraw more than (lamports - rent_exempt_reserve)
        let mut data = vec![];
        data.extend_from_slice(&4u32.to_le_bytes());
        data.extend_from_slice(&10_000_000_000u64.to_le_bytes());
        let err = invoke(
            data,
            &mut accounts,
            metas(&[
                (stake_addr, false, true),
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
    fn unsupported_variant_errors() {
        let stake_addr = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            stake_addr,
            1,
            STAKE_PROGRAM_ID,
            vec![0u8; STAKE_ACCOUNT_SIZE],
            false,
        )];
        let mut data = vec![];
        data.extend_from_slice(&3u32.to_le_bytes()); // Split — not supported
        let err =
            invoke(data, &mut accounts, metas(&[(stake_addr, false, true)])).unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("not supported"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }
}
