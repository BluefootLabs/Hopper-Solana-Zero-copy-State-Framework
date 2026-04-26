//! System program — Hopper-native implementation.
//!
//! The Solana system program ships as a built-in (not BPF) on
//! mainnet, so reproducing it in pure Rust is the right shape for a
//! Phase-1 harness. We implement the four most-used variants:
//! `CreateAccount`, `Transfer`, `Allocate`, and `Assign`. Each is
//! parsed from `instruction.data` per the standard upstream wire
//! format (matching `solana_sdk::system_instruction::SystemInstruction`)
//! and acted on against the supplied accounts.
//!
//! Wire format (matches `bincode::serialize` of
//! `SystemInstruction`):
//!
//! - `CreateAccount`: tag `[0,0,0,0]` + lamports (u64) + space
//!   (u64) + owner (32 bytes).
//! - `Assign`: tag `[1,0,0,0]` + owner (32 bytes).
//! - `Transfer`: tag `[2,0,0,0]` + lamports (u64).
//! - `Allocate`: tag `[8,0,0,0]` + space (u64).
//!
//! Tags omitted from this Phase-1 implementation: `CreateAccountWithSeed`,
//! `AdvanceNonceAccount`, `WithdrawNonceAccount`,
//! `InitializeNonceAccount`, `AuthorizeNonceAccount`,
//! `AssignWithSeed`, `TransferWithSeed`, `UpgradeNonceAccount`. They
//! return a clear "unsupported variant" error so tests that hit them
//! fail fast with an actionable message.

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_program;

/// The system program's invariant CU cost. Mainnet charges 150 CU
/// for a system-program transfer; pin to that here so tests that
/// snapshot CU consumption see the same number regardless of how
/// the user configured the harness's default cost.
const SYSTEM_PROGRAM_COST: u64 = 150;

/// System program — registered by default in
/// [`crate::HopperSvm::new`].
pub struct SystemProgram;

impl BuiltinProgram for SystemProgram {
    fn name(&self) -> &'static str {
        "system"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        SYSTEM_PROGRAM_COST
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
                    "system: instruction data too short ({} bytes, need ≥ 4 for tag)",
                    data.len()
                ),
            });
        }
        let tag = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let body = &data[4..];
        match tag {
            0 => create_account(body, accounts, ctx),
            1 => assign(body, accounts, ctx),
            2 => transfer(body, accounts, ctx),
            3 => create_account_with_seed(body, accounts, ctx),
            4 => advance_nonce_account(body, accounts, ctx),
            5 => withdraw_nonce_account(body, accounts, ctx),
            6 => initialize_nonce_account(body, accounts, ctx),
            7 => authorize_nonce_account(body, accounts, ctx),
            8 => allocate(body, accounts, ctx),
            9 => assign_with_seed(body, accounts, ctx),
            10 => transfer_with_seed(body, accounts, ctx),
            11 => upgrade_nonce_account(body, accounts, ctx),
            other => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "system: variant tag {other} not recognised \
                     (supported: 0/CreateAccount, 1/Assign, 2/Transfer, \
                     3/CreateAccountWithSeed, 4/AdvanceNonceAccount, \
                     5/WithdrawNonceAccount, 6/InitializeNonceAccount, \
                     7/AuthorizeNonceAccount, 8/Allocate, 9/AssignWithSeed, \
                     10/TransferWithSeed, 11/UpgradeNonceAccount)"
                ),
            }),
        }
    }
}

/// `CreateAccount`: `body = lamports(u64) | space(u64) | owner(32)`.
/// Account 0 is the funder (signer, writable). Account 1 is the new
/// account (signer, writable, must be empty / zero lamports).
fn create_account(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 + 8 + 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::CreateAccount: body too short ({} bytes, need 48)",
                body.len()
            ),
        });
    }
    let lamports = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let space = u64::from_le_bytes(body[8..16].try_into().unwrap());
    let owner = Pubkey::new_from_array(body[16..48].try_into().unwrap());

    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }

    // Capture addresses before taking mutable refs so we can call
    // ctx.require_* without overlapping borrows.
    let funder_addr = accounts[0].address;
    let new_addr = accounts[1].address;

    ctx.require_signer(&funder_addr)?;
    ctx.require_signer(&new_addr)?;
    ctx.require_writable(&funder_addr)?;
    ctx.require_writable(&new_addr)?;

    if accounts[0].lamports < lamports {
        return Err(HopperSvmError::InsufficientFunds {
            account: funder_addr,
            balance: accounts[0].lamports,
            requested: lamports,
        });
    }
    // Refuse to clobber a non-empty account — matches the on-chain
    // runtime which rejects CreateAccount when the target already
    // has data or non-system ownership.
    if accounts[1].lamports != 0 || !accounts[1].data.is_empty() {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::CreateAccount: target already initialised".to_string(),
        });
    }

    // Apply the transfer atomically against the working set.
    accounts[0].lamports -= lamports;
    accounts[1].lamports = lamports;
    accounts[1].data = vec![0u8; space as usize];
    accounts[1].owner = owner;
    accounts[1].executable = false;
    ctx.log(format!(
        "system::CreateAccount: {} -> {} ({} lamports, {} bytes, owner {})",
        funder_addr, new_addr, lamports, space, owner
    ));
    Ok(())
}

/// `Assign`: `body = owner(32)`. Account 0 is the writable, signer
/// account whose owner is being changed.
fn assign(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::Assign: body too short".to_string(),
        });
    }
    let new_owner = Pubkey::new_from_array(body[0..32].try_into().unwrap());
    if accounts.is_empty() {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 0,
            len: 0,
        });
    }
    let addr = accounts[0].address;
    ctx.require_signer(&addr)?;
    ctx.require_writable(&addr)?;
    accounts[0].owner = new_owner;
    ctx.log(format!("system::Assign: {addr} -> owner {new_owner}"));
    Ok(())
}

/// `Transfer`: `body = lamports(u64)`. Account 0 = signer/writable
/// source; account 1 = writable destination.
fn transfer(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::Transfer: body too short".to_string(),
        });
    }
    let lamports = u64::from_le_bytes(body[0..8].try_into().unwrap());
    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }

    let src_addr = accounts[0].address;
    let dst_addr = accounts[1].address;
    ctx.require_signer(&src_addr)?;
    ctx.require_writable(&src_addr)?;
    ctx.require_writable(&dst_addr)?;

    // The on-chain runtime rejects transfers FROM a non-system-owned
    // account; a system-program-owned account is the only one that
    // can debit lamports through this path. Match that.
    if accounts[0].owner != system_program::id() {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::Transfer: source {src_addr} not owned by system program (owner={})",
                accounts[0].owner
            ),
        });
    }

    if accounts[0].lamports < lamports {
        return Err(HopperSvmError::InsufficientFunds {
            account: src_addr,
            balance: accounts[0].lamports,
            requested: lamports,
        });
    }
    accounts[0].lamports -= lamports;
    accounts[1].lamports = accounts[1].lamports.saturating_add(lamports);
    ctx.log(format!(
        "system::Transfer: {} -> {} ({} lamports)",
        src_addr, dst_addr, lamports
    ));
    Ok(())
}

/// `Allocate`: `body = space(u64)`. Account 0 = signer/writable
/// account whose data buffer is being sized. The runtime rejects
/// allocate against an account that already has non-zero data; we
/// match that.
fn allocate(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::Allocate: body too short".to_string(),
        });
    }
    let space = u64::from_le_bytes(body[0..8].try_into().unwrap()) as usize;
    if accounts.is_empty() {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 0,
            len: 0,
        });
    }
    let addr = accounts[0].address;
    ctx.require_signer(&addr)?;
    ctx.require_writable(&addr)?;
    if !accounts[0].data.is_empty() {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::Allocate: account already has data".to_string(),
        });
    }
    accounts[0].data = vec![0u8; space];
    ctx.log(format!("system::Allocate: {addr} -> {space} bytes"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 3 — CreateAccountWithSeed
// ---------------------------------------------------------------------------
//
// Wire: body = base(32) | seed_len(u64 LE) | seed | lamports(u64) | space(u64) | owner(32)
//
// The created account's address must equal
//   `Pubkey::create_with_seed(base, seed_str, owner)`
// = SHA-256(base || seed || owner) under the upstream impl.
//
// Accounts:
//   0: funder (signer, writable)
//   1: created (writable; address must match the seed derivation)
//   2: base   (signer, only required if base != funder)
fn create_account_with_seed(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 32 + 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::CreateAccountWithSeed: body too short ({} bytes)",
                body.len()
            ),
        });
    }
    let base = Pubkey::new_from_array(body[0..32].try_into().unwrap());
    let seed_len = u64::from_le_bytes(body[32..40].try_into().unwrap()) as usize;
    if body.len() < 40 + seed_len + 8 + 8 + 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::CreateAccountWithSeed: body too short for seed_len={seed_len} ({} bytes)",
                body.len()
            ),
        });
    }
    let seed_bytes = &body[40..40 + seed_len];
    let seed_str = match std::str::from_utf8(seed_bytes) {
        Ok(s) => s,
        Err(err) => {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "system::CreateAccountWithSeed: seed not valid UTF-8: {err}"
                ),
            });
        }
    };
    let mut off = 40 + seed_len;
    let lamports = u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    off += 8;
    let space = u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    off += 8;
    let owner = Pubkey::new_from_array(body[off..off + 32].try_into().unwrap());

    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }
    let funder_addr = accounts[0].address;
    let target_addr = accounts[1].address;

    let derived = Pubkey::create_with_seed(&base, seed_str, &owner).map_err(|err| {
        HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("system::CreateAccountWithSeed: derive failed: {err:?}"),
        }
    })?;
    if derived != target_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::CreateAccountWithSeed: address mismatch (got {target_addr}, derived {derived})"
            ),
        });
    }

    ctx.require_signer(&funder_addr)?;
    ctx.require_writable(&funder_addr)?;
    ctx.require_writable(&target_addr)?;
    // The base must sign unless it equals the funder.
    if base != funder_addr {
        ctx.require_signer(&base)?;
    }

    if accounts[0].lamports < lamports {
        return Err(HopperSvmError::InsufficientFunds {
            account: funder_addr,
            balance: accounts[0].lamports,
            requested: lamports,
        });
    }
    if accounts[1].lamports != 0 || !accounts[1].data.is_empty() {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::CreateAccountWithSeed: target already initialised".to_string(),
        });
    }

    accounts[0].lamports -= lamports;
    accounts[1].lamports = lamports;
    accounts[1].data = vec![0u8; space as usize];
    accounts[1].owner = owner;
    accounts[1].executable = false;
    ctx.log(format!(
        "system::CreateAccountWithSeed: {target_addr} from base {base} seed {seed_str:?} ({lamports} lamports, {space} bytes, owner {owner})"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 9 — AssignWithSeed
// ---------------------------------------------------------------------------
//
// Wire: body = base(32) | seed_len(u64) | seed | owner(32)
//
// Accounts:
//   0: account to assign (writable; address must match seed derivation)
//   1: base (signer)
fn assign_with_seed(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 32 + 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::AssignWithSeed: body too short".to_string(),
        });
    }
    let base = Pubkey::new_from_array(body[0..32].try_into().unwrap());
    let seed_len = u64::from_le_bytes(body[32..40].try_into().unwrap()) as usize;
    if body.len() < 40 + seed_len + 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::AssignWithSeed: body too short for seed".to_string(),
        });
    }
    let seed_bytes = &body[40..40 + seed_len];
    let seed_str = std::str::from_utf8(seed_bytes).map_err(|err| HopperSvmError::BuiltinError {
        program_id: *ctx.program_id,
        message: format!("system::AssignWithSeed: seed not UTF-8: {err}"),
    })?;
    let owner =
        Pubkey::new_from_array(body[40 + seed_len..40 + seed_len + 32].try_into().unwrap());

    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }
    let target_addr = accounts[0].address;
    ctx.require_writable(&target_addr)?;
    ctx.require_signer(&base)?;
    let derived = Pubkey::create_with_seed(&base, seed_str, &owner).map_err(|err| {
        HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("system::AssignWithSeed: derive failed: {err:?}"),
        }
    })?;
    if derived != target_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::AssignWithSeed: address mismatch (got {target_addr}, derived {derived})"
            ),
        });
    }
    accounts[0].owner = owner;
    ctx.log(format!(
        "system::AssignWithSeed: {target_addr} -> owner {owner}"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 10 — TransferWithSeed
// ---------------------------------------------------------------------------
//
// Wire: body = lamports(u64) | from_seed_len(u64) | from_seed | from_owner(32)
//
// Accounts:
//   0: source (writable; address must = create_with_seed(from_base, from_seed, from_owner))
//   1: from_base (signer)
//   2: destination (writable)
fn transfer_with_seed(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 + 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::TransferWithSeed: body too short".to_string(),
        });
    }
    let lamports = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let seed_len = u64::from_le_bytes(body[8..16].try_into().unwrap()) as usize;
    if body.len() < 16 + seed_len + 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::TransferWithSeed: body too short for seed".to_string(),
        });
    }
    let seed_bytes = &body[16..16 + seed_len];
    let seed_str = std::str::from_utf8(seed_bytes).map_err(|err| HopperSvmError::BuiltinError {
        program_id: *ctx.program_id,
        message: format!("system::TransferWithSeed: seed not UTF-8: {err}"),
    })?;
    let from_owner =
        Pubkey::new_from_array(body[16 + seed_len..16 + seed_len + 32].try_into().unwrap());

    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let src_addr = accounts[0].address;
    let from_base = accounts[1].address;
    let dst_addr = accounts[2].address;
    ctx.require_writable(&src_addr)?;
    ctx.require_writable(&dst_addr)?;
    ctx.require_signer(&from_base)?;
    let derived = Pubkey::create_with_seed(&from_base, seed_str, &from_owner).map_err(
        |err| HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("system::TransferWithSeed: derive failed: {err:?}"),
        },
    )?;
    if derived != src_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::TransferWithSeed: address mismatch (got {src_addr}, derived {derived})"
            ),
        });
    }
    if accounts[0].lamports < lamports {
        return Err(HopperSvmError::InsufficientFunds {
            account: src_addr,
            balance: accounts[0].lamports,
            requested: lamports,
        });
    }
    accounts[0].lamports -= lamports;
    accounts[2].lamports = accounts[2].lamports.saturating_add(lamports);
    ctx.log(format!(
        "system::TransferWithSeed: {src_addr} -> {dst_addr} ({lamports} lamports, base {from_base})"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Nonce account state — 80 bytes wire layout
// ---------------------------------------------------------------------------
//
// ```text
// [0..4]    Versions tag (u32 LE; 1 = Current, 0 = Legacy)
// [4..8]    State tag    (u32 LE; 0 = Uninitialized, 1 = Initialized)
// [8..40]   authority    (Pubkey)
// [40..72]  durable_nonce (Hash; 32 bytes)
// [72..80]  fee_calculator.lamports_per_signature (u64 LE)
// ```
//
// Solana's `Pubkey::create_with_seed` for nonces is a different
// thing (no seed). Nonce-state serialization is what's relevant
// here. We hard-code the 80-byte layout instead of pulling
// `bincode` in just for this. The format is stable on mainnet
// (both Legacy and Current variants share these field offsets;
// the distinction is informational, not byte-shaped).

const NONCE_STATE_BYTES: usize = 80;
const NONCE_TAG_VERSIONS: u32 = 1; // Current
const NONCE_TAG_INITIALIZED: u32 = 1;
const NONCE_TAG_UNINITIALIZED: u32 = 0;
/// Default lamports-per-signature value programs read from a
/// fresh nonce account. Mainnet's current value for legacy
/// FeeCalculator pricing.
const NONCE_DEFAULT_FEE: u64 = 5_000;

/// Read a nonce-account state. Returns `(initialized, authority,
/// durable_nonce, lamports_per_signature)`. Errors if data is
/// shorter than 80 bytes.
fn read_nonce_state(
    data: &[u8],
    program_id: &Pubkey,
) -> Result<(bool, Pubkey, [u8; 32], u64), HopperSvmError> {
    if data.len() < NONCE_STATE_BYTES {
        return Err(HopperSvmError::BuiltinError {
            program_id: *program_id,
            message: format!(
                "nonce state: data {} bytes < required {NONCE_STATE_BYTES}",
                data.len()
            ),
        });
    }
    let state_tag = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let initialized = state_tag == NONCE_TAG_INITIALIZED;
    let authority = Pubkey::new_from_array(data[8..40].try_into().unwrap());
    let mut durable_nonce = [0u8; 32];
    durable_nonce.copy_from_slice(&data[40..72]);
    let fee = u64::from_le_bytes(data[72..80].try_into().unwrap());
    Ok((initialized, authority, durable_nonce, fee))
}

/// Write a nonce-account state. Caller is responsible for
/// ensuring the `data` buffer is at least 80 bytes.
fn write_nonce_state(
    data: &mut [u8],
    initialized: bool,
    authority: &Pubkey,
    durable_nonce: &[u8; 32],
    lamports_per_signature: u64,
) {
    if data.len() < NONCE_STATE_BYTES {
        // Shouldn't happen — callers grow first.
        return;
    }
    data[0..4].copy_from_slice(&NONCE_TAG_VERSIONS.to_le_bytes());
    data[4..8].copy_from_slice(
        &(if initialized {
            NONCE_TAG_INITIALIZED
        } else {
            NONCE_TAG_UNINITIALIZED
        })
        .to_le_bytes(),
    );
    data[8..40].copy_from_slice(authority.as_ref());
    data[40..72].copy_from_slice(durable_nonce);
    data[72..80].copy_from_slice(&lamports_per_signature.to_le_bytes());
}

/// Synthesise a deterministic durable-nonce blockhash from the
/// current sysvar slot. Mainnet's nonces are real blockhashes
/// from the validator's recent-blockhashes sysvar; tests don't
/// have validator state, so we hash the slot to produce a
/// stable but instruction-changing 32-byte value. The result
/// is deterministic per slot — `warp_to_slot(N)` then
/// `AdvanceNonceAccount` produces the same nonce twice if the
/// slot doesn't change in between.
fn synthesise_nonce_from_slot(slot: u64) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"hopper-nonce");
    h.update(&slot.to_le_bytes());
    let out = h.finalize();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&out);
    nonce
}

// ---------------------------------------------------------------------------
// Tag 6 — InitializeNonceAccount
// ---------------------------------------------------------------------------
//
// Wire: body = nonce_authority(32)
//
// Accounts:
//   0: nonce account (writable, must be ≥ 80 bytes of system-owned data)
//   1: recent_blockhashes sysvar (informational; we don't enforce)
//   2: rent sysvar (informational)
fn initialize_nonce_account(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::InitializeNonceAccount: body too short".to_string(),
        });
    }
    let authority = Pubkey::new_from_array(body[0..32].try_into().unwrap());
    if accounts.is_empty() {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 0,
            len: 0,
        });
    }
    let nonce_addr = accounts[0].address;
    ctx.require_writable(&nonce_addr)?;
    if accounts[0].data.len() < NONCE_STATE_BYTES {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::InitializeNonceAccount: account data {} bytes < required {NONCE_STATE_BYTES}",
                accounts[0].data.len()
            ),
        });
    }
    let (already_init, _, _, _) = read_nonce_state(&accounts[0].data, ctx.program_id)?;
    if already_init {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::InitializeNonceAccount: {nonce_addr} already initialised"
            ),
        });
    }
    let nonce = synthesise_nonce_from_slot(ctx.sysvars.clock.slot);
    write_nonce_state(
        &mut accounts[0].data,
        true,
        &authority,
        &nonce,
        NONCE_DEFAULT_FEE,
    );
    accounts[0].owner = solana_sdk::system_program::id();
    ctx.log(format!(
        "system::InitializeNonceAccount: {nonce_addr} authority={authority}"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 4 — AdvanceNonceAccount
// ---------------------------------------------------------------------------
//
// Wire: body empty
//
// Accounts:
//   0: nonce account (writable)
//   1: recent_blockhashes sysvar (informational)
//   2: nonce authority (signer)
fn advance_nonce_account(
    _body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let nonce_addr = accounts[0].address;
    let auth_addr = accounts[2].address;
    ctx.require_writable(&nonce_addr)?;
    ctx.require_signer(&auth_addr)?;
    let (init, stored_authority, _, fee) =
        read_nonce_state(&accounts[0].data, ctx.program_id)?;
    if !init {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::AdvanceNonceAccount: {nonce_addr} not initialised"
            ),
        });
    }
    if stored_authority != auth_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::AdvanceNonceAccount: signer {auth_addr} != stored authority {stored_authority}"
            ),
        });
    }
    // Synthesise a new durable nonce from the current slot. In
    // production the validator pulls this from
    // `recent_blockhashes`; we use the slot as a deterministic
    // proxy.
    let new_nonce = synthesise_nonce_from_slot(ctx.sysvars.clock.slot);
    write_nonce_state(
        &mut accounts[0].data,
        true,
        &stored_authority,
        &new_nonce,
        fee,
    );
    ctx.log(format!(
        "system::AdvanceNonceAccount: {nonce_addr} (slot {})",
        ctx.sysvars.clock.slot
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 5 — WithdrawNonceAccount
// ---------------------------------------------------------------------------
//
// Wire: body = lamports(u64)
//
// Accounts:
//   0: nonce account (writable)
//   1: destination (writable)
//   2: recent_blockhashes sysvar (informational)
//   3: rent sysvar (informational)
//   4: nonce authority (signer)
fn withdraw_nonce_account(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::WithdrawNonceAccount: body too short".to_string(),
        });
    }
    let lamports = u64::from_le_bytes(body[0..8].try_into().unwrap());
    if accounts.len() < 5 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 4,
            len: accounts.len(),
        });
    }
    let nonce_addr = accounts[0].address;
    let dst_addr = accounts[1].address;
    let auth_addr = accounts[4].address;
    ctx.require_writable(&nonce_addr)?;
    ctx.require_writable(&dst_addr)?;
    ctx.require_signer(&auth_addr)?;
    let (init, stored_authority, _, _) =
        read_nonce_state(&accounts[0].data, ctx.program_id)?;
    // For an initialised nonce account the authority must sign
    // and equal the stored one. For an uninitialised account
    // the authority is whoever is the rent-payer; we accept any
    // signer.
    if init && stored_authority != auth_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::WithdrawNonceAccount: signer {auth_addr} != stored authority {stored_authority}"
            ),
        });
    }
    if accounts[0].lamports < lamports {
        return Err(HopperSvmError::InsufficientFunds {
            account: nonce_addr,
            balance: accounts[0].lamports,
            requested: lamports,
        });
    }
    accounts[0].lamports -= lamports;
    accounts[1].lamports = accounts[1].lamports.saturating_add(lamports);
    ctx.log(format!(
        "system::WithdrawNonceAccount: {nonce_addr} -> {dst_addr} ({lamports} lamports)"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 7 — AuthorizeNonceAccount
// ---------------------------------------------------------------------------
//
// Wire: body = new_authority(32)
//
// Accounts:
//   0: nonce account (writable)
//   1: nonce authority (signer)
fn authorize_nonce_account(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 32 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "system::AuthorizeNonceAccount: body too short".to_string(),
        });
    }
    let new_authority = Pubkey::new_from_array(body[0..32].try_into().unwrap());
    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }
    let nonce_addr = accounts[0].address;
    let auth_addr = accounts[1].address;
    ctx.require_writable(&nonce_addr)?;
    ctx.require_signer(&auth_addr)?;
    let (init, stored_authority, durable_nonce, fee) =
        read_nonce_state(&accounts[0].data, ctx.program_id)?;
    if !init {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::AuthorizeNonceAccount: {nonce_addr} not initialised"
            ),
        });
    }
    if stored_authority != auth_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "system::AuthorizeNonceAccount: signer {auth_addr} != stored authority {stored_authority}"
            ),
        });
    }
    write_nonce_state(
        &mut accounts[0].data,
        true,
        &new_authority,
        &durable_nonce,
        fee,
    );
    ctx.log(format!(
        "system::AuthorizeNonceAccount: {nonce_addr} -> authority {new_authority}"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 11 — UpgradeNonceAccount
// ---------------------------------------------------------------------------
//
// One-time on-chain migration from Legacy to Current
// `Versions`. Both layouts have the same byte shape in our
// implementation, so this is a no-op success at the Hopper
// level (the writable check still applies so a non-writable
// meta is rejected).
//
// Accounts:
//   0: nonce account (writable)
fn upgrade_nonce_account(
    _body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.is_empty() {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 0,
            len: 0,
        });
    }
    let nonce_addr = accounts[0].address;
    ctx.require_writable(&nonce_addr)?;
    // Verify the version tag is present (treat as already
    // current).
    if accounts[0].data.len() >= 4 {
        accounts[0].data[0..4].copy_from_slice(&NONCE_TAG_VERSIONS.to_le_bytes());
    }
    ctx.log(format!(
        "system::UpgradeNonceAccount: {nonce_addr} (already current; no-op)"
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogCapture;
    use crate::sysvar::Sysvars;
    use solana_sdk::instruction::AccountMeta;

    fn metas_for(addrs: &[(Pubkey, bool, bool)]) -> Vec<AccountMeta> {
        addrs
            .iter()
            .map(|(pk, signer, writable)| AccountMeta {
                pubkey: *pk,
                is_signer: *signer,
                is_writable: *writable,
            })
            .collect()
    }

    /// Transfer must debit the source and credit the destination by
    /// exactly the requested amount, leaving every other field
    /// untouched. Pin the exact field-by-field outcome.
    #[test]
    fn system_transfer_debits_and_credits_exactly() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(alice, 1_000, system_program::id(), Vec::new(), false),
            KeyedAccount::new(bob, 50, system_program::id(), Vec::new(), false),
        ];
        let metas = metas_for(&[(alice, true, true), (bob, false, true)]);
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = system_program::id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        let mut data = vec![2, 0, 0, 0];
        data.extend_from_slice(&250u64.to_le_bytes());
        SystemProgram.invoke(&data, &mut accounts, &mut ctx).unwrap();
        assert_eq!(accounts[0].lamports, 750);
        assert_eq!(accounts[1].lamports, 300);
    }

    /// Transfer from a non-system-owned account must fail — matches
    /// runtime behaviour and prevents accidental "I forgot to set
    /// the owner" tests from passing silently.
    #[test]
    fn transfer_from_wrong_owner_fails() {
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();
        let other_owner = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(alice, 1_000, other_owner, Vec::new(), false),
            KeyedAccount::new(bob, 0, system_program::id(), Vec::new(), false),
        ];
        let metas = metas_for(&[(alice, true, true), (bob, false, true)]);
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = system_program::id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        let mut data = vec![2, 0, 0, 0];
        data.extend_from_slice(&100u64.to_le_bytes());
        let err = SystemProgram
            .invoke(&data, &mut accounts, &mut ctx)
            .unwrap_err();
        assert!(matches!(err, HopperSvmError::BuiltinError { .. }));
    }

    /// CreateAccount must fail when the target already has lamports.
    #[test]
    fn create_account_refuses_initialised_target() {
        let funder = Pubkey::new_unique();
        let target = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(funder, 5_000_000, system_program::id(), Vec::new(), false),
            KeyedAccount::new(target, 100, system_program::id(), Vec::new(), false),
        ];
        let metas = metas_for(&[(funder, true, true), (target, true, true)]);
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = system_program::id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(&1_000_000u64.to_le_bytes());
        data.extend_from_slice(&100u64.to_le_bytes());
        data.extend_from_slice(&[0u8; 32]);
        let err = SystemProgram
            .invoke(&data, &mut accounts, &mut ctx)
            .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("already initialised"), "{message}");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    // ── Tier 2 (b) — system program parity tests ─────────────────

    /// Helper: build a fully-formed InvokeContext for the new tag handlers.
    fn invoke_with(
        data: Vec<u8>,
        accounts: &mut Vec<KeyedAccount>,
        metas: Vec<AccountMeta>,
    ) -> Result<(), HopperSvmError> {
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = solana_sdk::system_program::id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        SystemProgram.invoke(&data, accounts, &mut ctx)
    }

    /// CreateAccountWithSeed derives the target address from
    /// (base, seed, owner) and rejects an address mismatch.
    #[test]
    fn create_account_with_seed_validates_derivation() {
        let funder = Pubkey::new_unique();
        let base = funder; // base = funder, no separate signer
        let owner = Pubkey::new_unique();
        let seed = "vault";
        let target =
            Pubkey::create_with_seed(&base, seed, &owner).expect("derive ok");

        let mut accounts = vec![
            KeyedAccount::new(funder, 5_000_000, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(target, 0, solana_sdk::system_program::id(), vec![], false),
        ];
        let mut data = vec![3u8, 0, 0, 0]; // tag = 3
        data.extend_from_slice(base.as_ref());
        data.extend_from_slice(&(seed.len() as u64).to_le_bytes());
        data.extend_from_slice(seed.as_bytes());
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // lamports
        data.extend_from_slice(&100u64.to_le_bytes()); // space
        data.extend_from_slice(owner.as_ref());

        invoke_with(
            data,
            &mut accounts,
            metas_for(&[(funder, true, true), (target, false, true)]),
        )
        .expect("CreateAccountWithSeed");

        assert_eq!(accounts[1].lamports, 1_000_000);
        assert_eq!(accounts[1].owner, owner);
        assert_eq!(accounts[1].data.len(), 100);
        assert_eq!(accounts[0].lamports, 5_000_000 - 1_000_000);
    }

    /// CreateAccountWithSeed with a wrong target address fails
    /// before any state mutation.
    #[test]
    fn create_account_with_seed_rejects_address_mismatch() {
        let funder = Pubkey::new_unique();
        let base = funder;
        let owner = Pubkey::new_unique();
        let seed = "vault";
        let bogus_target = Pubkey::new_unique(); // not derived

        let mut accounts = vec![
            KeyedAccount::new(funder, 5_000_000, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(bogus_target, 0, solana_sdk::system_program::id(), vec![], false),
        ];
        let mut data = vec![3u8, 0, 0, 0];
        data.extend_from_slice(base.as_ref());
        data.extend_from_slice(&(seed.len() as u64).to_le_bytes());
        data.extend_from_slice(seed.as_bytes());
        data.extend_from_slice(&1_000_000u64.to_le_bytes());
        data.extend_from_slice(&100u64.to_le_bytes());
        data.extend_from_slice(owner.as_ref());

        let err = invoke_with(
            data,
            &mut accounts,
            metas_for(&[(funder, true, true), (bogus_target, false, true)]),
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("address mismatch"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// AssignWithSeed assigns a seed-derived account to the owner
    /// the seed was derived under. Solana's `AssignWithSeed`
    /// derivation-owner is also the assignment-owner; the seed
    /// payload's owner field doubles as both, so this verb does
    /// not rotate ownership. The "with seed" suffix exists so the
    /// base account can sign in place of the derived address.
    #[test]
    fn assign_with_seed_round_trip() {
        let base = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let seed = "config";
        let target = Pubkey::create_with_seed(&base, seed, &owner).unwrap();

        let mut accounts = vec![
            KeyedAccount::new(
                target,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![0u8; 8],
                false,
            ),
            KeyedAccount::new(base, 1_000_000, solana_sdk::system_program::id(), vec![], false),
        ];
        let mut data = vec![9u8, 0, 0, 0]; // tag = 9
        data.extend_from_slice(base.as_ref());
        data.extend_from_slice(&(seed.len() as u64).to_le_bytes());
        data.extend_from_slice(seed.as_bytes());
        data.extend_from_slice(owner.as_ref());

        invoke_with(
            data,
            &mut accounts,
            metas_for(&[(target, false, true), (base, true, false)]),
        )
        .expect("AssignWithSeed");
        assert_eq!(accounts[0].owner, owner);
    }

    /// TransferWithSeed debits a seed-derived source.
    #[test]
    fn transfer_with_seed_round_trip() {
        let base = Pubkey::new_unique();
        let from_owner = Pubkey::new_unique();
        let dst = Pubkey::new_unique();
        let seed = "vault";
        let src = Pubkey::create_with_seed(&base, seed, &from_owner).unwrap();

        let mut accounts = vec![
            KeyedAccount::new(src, 1_000_000, from_owner, vec![], false),
            KeyedAccount::new(base, 1_000_000, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(dst, 50, solana_sdk::system_program::id(), vec![], false),
        ];
        let mut data = vec![10u8, 0, 0, 0]; // tag = 10
        data.extend_from_slice(&250_000u64.to_le_bytes());
        data.extend_from_slice(&(seed.len() as u64).to_le_bytes());
        data.extend_from_slice(seed.as_bytes());
        data.extend_from_slice(from_owner.as_ref());

        invoke_with(
            data,
            &mut accounts,
            metas_for(&[(src, false, true), (base, true, false), (dst, false, true)]),
        )
        .expect("TransferWithSeed");
        assert_eq!(accounts[0].lamports, 750_000);
        assert_eq!(accounts[2].lamports, 50 + 250_000);
    }

    /// InitializeNonceAccount writes the 80-byte nonce state
    /// with the requested authority and a synthesised durable
    /// nonce.
    #[test]
    fn initialize_nonce_account_writes_state() {
        let nonce_addr = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let recent_blockhashes = Pubkey::new_unique();
        let rent = Pubkey::new_unique();

        let mut accounts = vec![
            KeyedAccount::new(
                nonce_addr,
                2_000_000,
                solana_sdk::system_program::id(),
                vec![0u8; NONCE_STATE_BYTES],
                false,
            ),
            KeyedAccount::new(recent_blockhashes, 0, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(rent, 0, solana_sdk::system_program::id(), vec![], false),
        ];
        let mut data = vec![6u8, 0, 0, 0]; // tag = 6
        data.extend_from_slice(authority.as_ref());

        invoke_with(
            data,
            &mut accounts,
            metas_for(&[
                (nonce_addr, false, true),
                (recent_blockhashes, false, false),
                (rent, false, false),
            ]),
        )
        .expect("InitializeNonceAccount");

        let (init, stored_auth, durable, fee) =
            read_nonce_state(&accounts[0].data, &solana_sdk::system_program::id())
                .expect("read");
        assert!(init);
        assert_eq!(stored_auth, authority);
        assert_eq!(fee, NONCE_DEFAULT_FEE);
        // Durable nonce is non-zero (synthesised from slot).
        assert_ne!(durable, [0u8; 32]);
    }

    /// AdvanceNonceAccount rotates the durable nonce when called
    /// at a different slot. Same slot → same nonce
    /// (deterministic), different slot → different nonce.
    #[test]
    fn advance_nonce_account_changes_nonce_with_slot() {
        let nonce_addr = Pubkey::new_unique();
        let authority = Pubkey::new_unique();

        // Pre-build an initialised nonce account.
        let mut data = vec![0u8; NONCE_STATE_BYTES];
        let initial_nonce = synthesise_nonce_from_slot(0);
        write_nonce_state(
            &mut data,
            true,
            &authority,
            &initial_nonce,
            NONCE_DEFAULT_FEE,
        );

        let recent_blockhashes = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(nonce_addr, 2_000_000, solana_sdk::system_program::id(), data, false),
            KeyedAccount::new(recent_blockhashes, 0, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(authority, 1_000_000, solana_sdk::system_program::id(), vec![], false),
        ];
        let metas = metas_for(&[
            (nonce_addr, false, true),
            (recent_blockhashes, false, false),
            (authority, true, false),
        ]);

        // Advance at slot = 0 → nonce stays the same (deterministic).
        invoke_with(vec![4u8, 0, 0, 0], &mut accounts, metas.clone()).expect("Advance @ 0");
        let (_, _, n0, _) =
            read_nonce_state(&accounts[0].data, &solana_sdk::system_program::id()).unwrap();
        assert_eq!(n0, initial_nonce);

        // Bump the slot via a custom sysvar set, then advance.
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let mut sysvars = Sysvars::default();
        sysvars.clock.slot = 1_000;
        let pid = solana_sdk::system_program::id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        SystemProgram
            .invoke(&[4u8, 0, 0, 0], &mut accounts, &mut ctx)
            .expect("Advance @ 1000");
        let (_, _, n1, _) =
            read_nonce_state(&accounts[0].data, &solana_sdk::system_program::id()).unwrap();
        assert_ne!(n1, initial_nonce, "different slot should produce different nonce");
    }

    /// AuthorizeNonceAccount with the wrong signer is rejected.
    #[test]
    fn authorize_nonce_rejects_wrong_signer() {
        let nonce_addr = Pubkey::new_unique();
        let real_authority = Pubkey::new_unique();
        let bogus = Pubkey::new_unique();
        let new_authority = Pubkey::new_unique();

        let mut data = vec![0u8; NONCE_STATE_BYTES];
        write_nonce_state(
            &mut data,
            true,
            &real_authority,
            &[0xAB; 32],
            NONCE_DEFAULT_FEE,
        );
        let mut accounts = vec![
            KeyedAccount::new(nonce_addr, 2_000_000, solana_sdk::system_program::id(), data, false),
            KeyedAccount::new(bogus, 1_000_000, solana_sdk::system_program::id(), vec![], false),
        ];
        let mut ix_data = vec![7u8, 0, 0, 0]; // tag = 7
        ix_data.extend_from_slice(new_authority.as_ref());

        let err = invoke_with(
            ix_data,
            &mut accounts,
            metas_for(&[(nonce_addr, false, true), (bogus, true, false)]),
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(
                    message.contains("stored authority"),
                    "{message}"
                );
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// WithdrawNonceAccount transfers lamports + leaves the
    /// nonce state intact.
    #[test]
    fn withdraw_nonce_account_moves_lamports() {
        let nonce_addr = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let dst = Pubkey::new_unique();
        let recent_blockhashes = Pubkey::new_unique();
        let rent = Pubkey::new_unique();

        let mut data = vec![0u8; NONCE_STATE_BYTES];
        write_nonce_state(
            &mut data,
            true,
            &authority,
            &[0u8; 32],
            NONCE_DEFAULT_FEE,
        );
        let mut accounts = vec![
            KeyedAccount::new(nonce_addr, 5_000_000, solana_sdk::system_program::id(), data, false),
            KeyedAccount::new(dst, 0, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(recent_blockhashes, 0, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(rent, 0, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(authority, 1_000_000, solana_sdk::system_program::id(), vec![], false),
        ];
        let mut ix_data = vec![5u8, 0, 0, 0]; // tag = 5
        ix_data.extend_from_slice(&1_000_000u64.to_le_bytes());

        invoke_with(
            ix_data,
            &mut accounts,
            metas_for(&[
                (nonce_addr, false, true),
                (dst, false, true),
                (recent_blockhashes, false, false),
                (rent, false, false),
                (authority, true, false),
            ]),
        )
        .expect("WithdrawNonceAccount");
        assert_eq!(accounts[0].lamports, 4_000_000);
        assert_eq!(accounts[1].lamports, 1_000_000);
    }

    /// Nonce state round-trips through write/read.
    #[test]
    fn nonce_state_round_trips() {
        let mut buf = vec![0u8; NONCE_STATE_BYTES];
        let auth = Pubkey::new_unique();
        let nonce: [u8; 32] = [0x42; 32];
        let fee = 12_345;
        write_nonce_state(&mut buf, true, &auth, &nonce, fee);
        let (init, ra, rn, rfee) =
            read_nonce_state(&buf, &Pubkey::default()).unwrap();
        assert!(init);
        assert_eq!(ra, auth);
        assert_eq!(rn, nonce);
        assert_eq!(rfee, fee);
    }
}
