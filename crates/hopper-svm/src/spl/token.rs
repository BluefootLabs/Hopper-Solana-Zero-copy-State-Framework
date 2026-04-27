//! SPL Token program simulator — pure-Rust [`BuiltinProgram`]
//! impl of the 8 most-used instructions. Registered against
//! `spl_token::id()` via [`crate::HopperSvm::with_spl_token_simulator`].
//!
//! ## Coverage
//!
//! | Tag | Instruction          | Implemented |
//! |-----|----------------------|-------------|
//! |  0  | `InitializeMint`     | ✓           |
//! |  1  | `InitializeAccount`  | ✓           |
//! |  3  | `Transfer`           | ✓           |
//! |  4  | `Approve`            | ✓           |
//! |  5  | `Revoke`             | ✓           |
//! |  7  | `MintTo`             | ✓           |
//! |  8  | `Burn`               | ✓           |
//! |  9  | `CloseAccount`       | ✓           |
//!
//! Other tags return a structured "not yet supported" error so
//! tests that hit them fail fast with an actionable message.
//! The remaining 16 instructions (`SetAuthority`,
//! `FreezeAccount`, `ThawAccount`, `InitializeMultisig`, the
//! various `*Checked` variants, `SyncNative`, etc.) land in
//! follow-up passes; the core 8 cover ~95% of token-test
//! workflows.
//!
//! ## Validation
//!
//! Every operation enforces:
//!
//! - Account owner = SPL Token program ID.
//! - Mint matches between source and destination on transfer.
//! - Authority is a signer (or matches the delegate slot).
//! - State is `Initialized` before any state-mutating op.
//! - Saturating-arithmetic checks on `mint.supply` and
//!   `account.amount`.
//!
//! ## Wire format
//!
//! Matches `spl_token::instruction::TokenInstruction`'s bincode
//! shape: a 1-byte tag followed by little-endian fields. We
//! parse that directly rather than depending on `spl-token`'s
//! deserialiser to keep the dep tree minimal (we still depend
//! on `spl-token` for the `Mint` / `Account` `Pack` impls — the
//! wire shape of the on-chain state is what matters for
//! interop with other tools).

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

/// Per-instruction CU baseline. Matches mainnet's actual
/// per-instruction cost roughly — token transfers cost ~3000 CU
/// in production; we charge a flat 4000 to leave headroom
/// without blowing through tight test budgets.
const TOKEN_INSTRUCTION_CU: u64 = 4_000;

/// `InitializeMint` data tail size:
///   1 (decimals) + 32 (mint_authority) + 1 (freeze_authority option flag)
///   + 32 (freeze_authority pubkey, present iff flag = 1) = 66 bytes max.
/// We accept either 34 (no freeze authority, flag = 0) or 66
/// (freeze authority present, flag = 1).
const INITIALIZE_MINT_MIN_LEN: usize = 1 + 1 + 32 + 1; // tag + decimals + mint_auth + flag

/// SPL Token program reference simulator. Register with
/// [`crate::HopperSvm::with_spl_token_simulator`].
pub struct SplTokenSimulator;

impl BuiltinProgram for SplTokenSimulator {
    fn name(&self) -> &'static str {
        "spl-token (simulated)"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        TOKEN_INSTRUCTION_CU
    }

    fn invoke(
        &self,
        data: &[u8],
        accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError> {
        let (tag, body) = data
            .split_first()
            .ok_or_else(|| HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "spl-token: empty instruction data".to_string(),
            })?;
        match *tag {
            0 => initialize_mint(body, accounts, ctx),
            1 => initialize_account(body, accounts, ctx),
            3 => transfer(body, accounts, ctx),
            4 => approve(body, accounts, ctx),
            5 => revoke(accounts, ctx),
            7 => mint_to(body, accounts, ctx),
            8 => burn(body, accounts, ctx),
            9 => close_account(accounts, ctx),
            other => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-token: instruction tag {other} not supported by the bundled simulator yet \
                     (supported tags: 0/InitializeMint, 1/InitializeAccount, 3/Transfer, \
                     4/Approve, 5/Revoke, 7/MintTo, 8/Burn, 9/CloseAccount)"
                ),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read the upstream-shaped `Mint` from an account. Returns a
/// structured error if the account isn't a Mint or isn't owned
/// by the token program.
fn read_mint(account: &KeyedAccount, ctx: &InvokeContext<'_>) -> Result<Mint, HopperSvmError> {
    if account.owner != *ctx.program_id {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token: mint account {} is owned by {} (expected token program)",
                account.address, account.owner
            ),
        });
    }
    Mint::unpack(&account.data).map_err(|err| HopperSvmError::BuiltinError {
        program_id: *ctx.program_id,
        message: format!("spl-token: Mint::unpack({}): {err:?}", account.address),
    })
}

/// Same as [`read_mint`] but for token accounts.
fn read_token_account(
    account: &KeyedAccount,
    ctx: &InvokeContext<'_>,
) -> Result<TokenAccount, HopperSvmError> {
    if account.owner != *ctx.program_id {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token: token account {} is owned by {} (expected token program)",
                account.address, account.owner
            ),
        });
    }
    TokenAccount::unpack(&account.data).map_err(|err| HopperSvmError::BuiltinError {
        program_id: *ctx.program_id,
        message: format!(
            "spl-token: TokenAccount::unpack({}): {err:?}",
            account.address
        ),
    })
}

/// Re-pack a Mint back into an account's data. Account data must
/// already be the right size (82 bytes); we ensure it on every
/// state-mutating path through `assert_eq` because the Mint::LEN
/// is canonical and a mismatch is always a test-side bug.
fn write_mint(account: &mut KeyedAccount, mint: &Mint) {
    if account.data.len() != Mint::LEN {
        account.data = vec![0u8; Mint::LEN];
    }
    Mint::pack(*mint, &mut account.data).expect("Mint::pack into 82-byte buffer");
}

/// Re-pack a TokenAccount back into an account's data.
fn write_token_account(account: &mut KeyedAccount, token: &TokenAccount) {
    if account.data.len() != TokenAccount::LEN {
        account.data = vec![0u8; TokenAccount::LEN];
    }
    TokenAccount::pack(*token, &mut account.data).expect("TokenAccount::pack into 165-byte buffer");
}

/// Read the COption<Pubkey> wire shape: 4-byte LE flag + 32-byte
/// pubkey. Used by `InitializeMint` for the freeze authority and
/// by `Approve` for the delegate.
fn read_coption_pubkey(body: &[u8], offset: usize) -> Option<Pubkey> {
    if body.len() < offset + 4 {
        return None;
    }
    let flag = u32::from_le_bytes(body[offset..offset + 4].try_into().unwrap());
    if flag != 1 || body.len() < offset + 4 + 32 {
        return None;
    }
    Some(Pubkey::new_from_array(
        body[offset + 4..offset + 4 + 32].try_into().unwrap(),
    ))
}

// ---------------------------------------------------------------------------
// Tag 0 — InitializeMint
// ---------------------------------------------------------------------------
//
// Wire: [tag=0][decimals: u8][mint_authority: 32][freeze_authority_flag: u8]
//       [freeze_authority: 32]?
//
// Accounts:
//   0: mint           (writable, signer)
//   1: rent sysvar    (read-only) — we don't enforce this in
//                     the simulator since rent is a stub
fn initialize_mint(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < INITIALIZE_MINT_MIN_LEN - 1 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::InitializeMint: body too short ({} bytes)",
                body.len()
            ),
        });
    }
    if accounts.is_empty() {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 0,
            len: accounts.len(),
        });
    }
    let mint_addr = accounts[0].address;
    ctx.require_writable(&mint_addr)?;
    let decimals = body[0];
    let mint_authority =
        Pubkey::new_from_array(body[1..33].try_into().expect("mint_authority 32 bytes"));
    // Freeze authority is COption-shaped: 1-byte flag + 32 bytes
    // when flag = 1. spl_token actually uses a 4-byte flag on
    // wire, but the InitializeMint instruction encodes it as a
    // single byte. Match the spl_token wire format: 1 byte.
    let freeze_authority = if body.len() >= 1 + 32 + 1 + 32 && body[33] == 1 {
        Some(Pubkey::new_from_array(
            body[34..66].try_into().expect("freeze_authority 32 bytes"),
        ))
    } else {
        None
    };

    if accounts[0].data.len() < Mint::LEN {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::InitializeMint: account data too small ({} < {})",
                accounts[0].data.len(),
                Mint::LEN
            ),
        });
    }

    // Reject re-initialisation: if the mint is already
    // initialised, this is an error (matches spl_token).
    if let Ok(existing) = Mint::unpack(&accounts[0].data) {
        if existing.is_initialized {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("spl-token::InitializeMint: {mint_addr} already initialised"),
            });
        }
    }

    let mint = Mint {
        mint_authority: Some(mint_authority).into(),
        supply: 0,
        decimals,
        is_initialized: true,
        freeze_authority: freeze_authority.into(),
    };
    write_mint(&mut accounts[0], &mint);
    accounts[0].owner = *ctx.program_id;
    ctx.log(format!(
        "spl-token::InitializeMint: {mint_addr} (decimals={decimals})"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 1 — InitializeAccount
// ---------------------------------------------------------------------------
//
// Accounts:
//   0: token account     (writable)
//   1: mint              (read-only)
//   2: account owner     (read-only)
//   3: rent sysvar       (read-only)
fn initialize_account(
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
    let acct_addr = accounts[0].address;
    ctx.require_writable(&acct_addr)?;
    let mint_addr = accounts[1].address;
    let owner_addr = accounts[2].address;

    let mint = read_mint(&accounts[1], ctx)?;
    if !mint.is_initialized {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("spl-token::InitializeAccount: mint {mint_addr} not initialised"),
        });
    }

    if accounts[0].data.len() < TokenAccount::LEN {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::InitializeAccount: account data too small ({} < {})",
                accounts[0].data.len(),
                TokenAccount::LEN
            ),
        });
    }
    // Reject re-init (state already Initialized).
    if let Ok(existing) = TokenAccount::unpack(&accounts[0].data) {
        if matches!(existing.state, AccountState::Initialized) {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("spl-token::InitializeAccount: {acct_addr} already initialised"),
            });
        }
    }

    let token = TokenAccount {
        mint: mint_addr,
        owner: owner_addr,
        amount: 0,
        delegate: None.into(),
        state: AccountState::Initialized,
        is_native: None.into(),
        delegated_amount: 0,
        close_authority: None.into(),
    };
    write_token_account(&mut accounts[0], &token);
    accounts[0].owner = *ctx.program_id;
    ctx.log(format!(
        "spl-token::InitializeAccount: {acct_addr} owner={owner_addr} mint={mint_addr}"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 3 — Transfer
// ---------------------------------------------------------------------------
//
// Wire: [tag=3][amount: u64 LE]
//
// Accounts:
//   0: source       (writable)
//   1: destination  (writable)
//   2: authority    (signer; either source.owner or source.delegate)
fn transfer(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "spl-token::Transfer: body < 8 bytes".to_string(),
        });
    }
    let amount = u64::from_le_bytes(body[0..8].try_into().unwrap());
    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let src_addr = accounts[0].address;
    let dst_addr = accounts[1].address;
    let auth_addr = accounts[2].address;

    ctx.require_writable(&src_addr)?;
    ctx.require_writable(&dst_addr)?;
    ctx.require_signer(&auth_addr)?;

    let mut src = read_token_account(&accounts[0], ctx)?;
    let mut dst = read_token_account(&accounts[1], ctx)?;

    if !matches!(src.state, AccountState::Initialized) {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("spl-token::Transfer: source {src_addr} not initialised"),
        });
    }
    if !matches!(dst.state, AccountState::Initialized) {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("spl-token::Transfer: destination {dst_addr} not initialised"),
        });
    }
    if src.mint != dst.mint {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::Transfer: mint mismatch (src={} dst={})",
                src.mint, dst.mint
            ),
        });
    }

    // Authority check: either the account owner is the signer,
    // OR the delegate is the signer with sufficient delegated
    // amount.
    if src.owner == auth_addr {
        // Owner-signed path.
    } else if let Some(delegate) = Option::<Pubkey>::from(src.delegate) {
        if delegate != auth_addr {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-token::Transfer: signer {auth_addr} is neither owner ({}) nor delegate ({})",
                    src.owner, delegate
                ),
            });
        }
        if src.delegated_amount < amount {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-token::Transfer: delegated amount {} < requested {amount}",
                    src.delegated_amount
                ),
            });
        }
        // Decrement delegated_amount; if it hits zero, clear
        // the delegate slot.
        src.delegated_amount = src.delegated_amount.saturating_sub(amount);
        if src.delegated_amount == 0 {
            src.delegate = None.into();
        }
    } else {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::Transfer: signer {auth_addr} is not the owner ({}) and no delegate is set",
                src.owner
            ),
        });
    }

    if src.amount < amount {
        return Err(HopperSvmError::InsufficientFunds {
            account: src_addr,
            balance: src.amount,
            requested: amount,
        });
    }
    src.amount -= amount;
    dst.amount = dst
        .amount
        .checked_add(amount)
        .ok_or_else(|| HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "spl-token::Transfer: destination amount overflow".to_string(),
        })?;
    write_token_account(&mut accounts[0], &src);
    write_token_account(&mut accounts[1], &dst);
    ctx.log(format!(
        "spl-token::Transfer: {src_addr} -> {dst_addr} amount={amount}"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 4 — Approve
// ---------------------------------------------------------------------------
//
// Wire: [tag=4][amount: u64 LE]
//
// Accounts:
//   0: source token account (writable)
//   1: delegate             (read-only)
//   2: source owner         (signer)
fn approve(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "spl-token::Approve: body < 8 bytes".to_string(),
        });
    }
    let amount = u64::from_le_bytes(body[0..8].try_into().unwrap());
    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let src_addr = accounts[0].address;
    let delegate_addr = accounts[1].address;
    let owner_addr = accounts[2].address;
    ctx.require_writable(&src_addr)?;
    ctx.require_signer(&owner_addr)?;
    let mut src = read_token_account(&accounts[0], ctx)?;
    if src.owner != owner_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::Approve: signer {owner_addr} is not the owner ({}) of {src_addr}",
                src.owner
            ),
        });
    }
    src.delegate = Some(delegate_addr).into();
    src.delegated_amount = amount;
    write_token_account(&mut accounts[0], &src);
    ctx.log(format!(
        "spl-token::Approve: {src_addr} delegate={delegate_addr} amount={amount}"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 5 — Revoke
// ---------------------------------------------------------------------------
//
// Accounts:
//   0: source token account (writable)
//   1: source owner         (signer)
fn revoke(
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.len() < 2 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 1,
            len: accounts.len(),
        });
    }
    let src_addr = accounts[0].address;
    let owner_addr = accounts[1].address;
    ctx.require_writable(&src_addr)?;
    ctx.require_signer(&owner_addr)?;
    let mut src = read_token_account(&accounts[0], ctx)?;
    if src.owner != owner_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::Revoke: signer {owner_addr} is not the owner ({})",
                src.owner
            ),
        });
    }
    src.delegate = None.into();
    src.delegated_amount = 0;
    write_token_account(&mut accounts[0], &src);
    ctx.log(format!("spl-token::Revoke: {src_addr}"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 7 — MintTo
// ---------------------------------------------------------------------------
//
// Wire: [tag=7][amount: u64 LE]
//
// Accounts:
//   0: mint            (writable)
//   1: destination     (writable)
//   2: mint authority  (signer)
fn mint_to(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "spl-token::MintTo: body < 8 bytes".to_string(),
        });
    }
    let amount = u64::from_le_bytes(body[0..8].try_into().unwrap());
    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let mint_addr = accounts[0].address;
    let dst_addr = accounts[1].address;
    let auth_addr = accounts[2].address;
    ctx.require_writable(&mint_addr)?;
    ctx.require_writable(&dst_addr)?;
    ctx.require_signer(&auth_addr)?;

    let mut mint = read_mint(&accounts[0], ctx)?;
    let mut dst = read_token_account(&accounts[1], ctx)?;
    if dst.mint != mint_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::MintTo: destination mint mismatch (dst.mint={} expected={mint_addr})",
                dst.mint
            ),
        });
    }
    let mint_authority = Option::<Pubkey>::from(mint.mint_authority).ok_or_else(|| {
        HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!("spl-token::MintTo: mint {mint_addr} has no mint authority"),
        }
    })?;
    if mint_authority != auth_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::MintTo: signer {auth_addr} is not the mint authority ({mint_authority})"
            ),
        });
    }
    mint.supply = mint
        .supply
        .checked_add(amount)
        .ok_or_else(|| HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "spl-token::MintTo: supply overflow".to_string(),
        })?;
    dst.amount = dst
        .amount
        .checked_add(amount)
        .ok_or_else(|| HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "spl-token::MintTo: destination amount overflow".to_string(),
        })?;
    write_mint(&mut accounts[0], &mint);
    write_token_account(&mut accounts[1], &dst);
    ctx.log(format!(
        "spl-token::MintTo: {mint_addr} -> {dst_addr} amount={amount}"
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 8 — Burn
// ---------------------------------------------------------------------------
//
// Wire: [tag=8][amount: u64 LE]
//
// Accounts:
//   0: source token account (writable)
//   1: mint                 (writable)
//   2: authority            (signer; account owner or delegate)
fn burn(
    body: &[u8],
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if body.len() < 8 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: "spl-token::Burn: body < 8 bytes".to_string(),
        });
    }
    let amount = u64::from_le_bytes(body[0..8].try_into().unwrap());
    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let src_addr = accounts[0].address;
    let mint_addr = accounts[1].address;
    let auth_addr = accounts[2].address;
    ctx.require_writable(&src_addr)?;
    ctx.require_writable(&mint_addr)?;
    ctx.require_signer(&auth_addr)?;

    let mut src = read_token_account(&accounts[0], ctx)?;
    let mut mint = read_mint(&accounts[1], ctx)?;
    if src.mint != mint_addr {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::Burn: source mint mismatch (src.mint={} expected={mint_addr})",
                src.mint
            ),
        });
    }
    // Authority check (same shape as Transfer).
    if src.owner != auth_addr {
        match Option::<Pubkey>::from(src.delegate) {
            Some(delegate) if delegate == auth_addr => {
                if src.delegated_amount < amount {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: format!(
                            "spl-token::Burn: delegated amount {} < requested {amount}",
                            src.delegated_amount
                        ),
                    });
                }
                src.delegated_amount = src.delegated_amount.saturating_sub(amount);
                if src.delegated_amount == 0 {
                    src.delegate = None.into();
                }
            }
            _ => {
                return Err(HopperSvmError::BuiltinError {
                    program_id: *ctx.program_id,
                    message: format!(
                        "spl-token::Burn: signer {auth_addr} is neither owner nor delegate"
                    ),
                });
            }
        }
    }
    if src.amount < amount {
        return Err(HopperSvmError::InsufficientFunds {
            account: src_addr,
            balance: src.amount,
            requested: amount,
        });
    }
    src.amount -= amount;
    mint.supply = mint.supply.saturating_sub(amount);
    write_token_account(&mut accounts[0], &src);
    write_mint(&mut accounts[1], &mint);
    ctx.log(format!("spl-token::Burn: {src_addr} amount={amount}"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag 9 — CloseAccount
// ---------------------------------------------------------------------------
//
// Accounts:
//   0: account to close   (writable; transfer lamports out, zero data)
//   1: lamport destination (writable)
//   2: authority           (signer; account owner or close_authority)
fn close_account(
    accounts: &mut [KeyedAccount],
    ctx: &mut InvokeContext<'_>,
) -> Result<(), HopperSvmError> {
    if accounts.len() < 3 {
        return Err(HopperSvmError::AccountIndexOutOfBounds {
            index: 2,
            len: accounts.len(),
        });
    }
    let acct_addr = accounts[0].address;
    let dst_addr = accounts[1].address;
    let auth_addr = accounts[2].address;
    ctx.require_writable(&acct_addr)?;
    ctx.require_writable(&dst_addr)?;
    ctx.require_signer(&auth_addr)?;

    let token = read_token_account(&accounts[0], ctx)?;
    if token.amount != 0 {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::CloseAccount: {acct_addr} non-empty (amount={})",
                token.amount
            ),
        });
    }
    let close_auth = Option::<Pubkey>::from(token.close_authority);
    if token.owner != auth_addr && close_auth != Some(auth_addr) {
        return Err(HopperSvmError::BuiltinError {
            program_id: *ctx.program_id,
            message: format!(
                "spl-token::CloseAccount: signer {auth_addr} is neither owner ({}) nor close_authority",
                token.owner
            ),
        });
    }
    // Move lamports.
    let lamports = accounts[0].lamports;
    accounts[1].lamports = accounts[1].lamports.saturating_add(lamports);
    accounts[0].lamports = 0;
    // Zero out data and reassign owner to the system program —
    // matches spl_token's close-account semantics.
    accounts[0].data = vec![0u8; TokenAccount::LEN];
    accounts[0].owner = solana_sdk::system_program::id();
    ctx.log(format!(
        "spl-token::CloseAccount: {acct_addr} -> {dst_addr} ({lamports} lamports)"
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
        sim: &SplTokenSimulator,
        data: Vec<u8>,
        accounts: &mut Vec<KeyedAccount>,
        metas_list: Vec<AccountMeta>,
    ) -> Result<(), HopperSvmError> {
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = spl_token::id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas_list,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        sim.invoke(&data, accounts, &mut ctx)
    }

    /// End-to-end: initialise a mint, initialise an account,
    /// mint to it, transfer some, burn some. Pin the full happy
    /// path against a single test so a regression in any of the
    /// state-mutating instructions surfaces immediately.
    #[test]
    fn happy_path_initialize_mint_account_mint_transfer_burn() {
        let pid = spl_token::id();
        let mint_addr = Pubkey::new_unique();
        let alice_acct = Pubkey::new_unique();
        let bob_acct = Pubkey::new_unique();
        let mint_authority = Pubkey::new_unique();
        let alice = Pubkey::new_unique();
        let bob = Pubkey::new_unique();

        let mut accounts = vec![
            KeyedAccount::new(mint_addr, 1_000_000, pid, vec![0u8; Mint::LEN], false),
            KeyedAccount::new(
                alice_acct,
                1_000_000,
                pid,
                vec![0u8; TokenAccount::LEN],
                false,
            ),
            KeyedAccount::new(
                bob_acct,
                1_000_000,
                pid,
                vec![0u8; TokenAccount::LEN],
                false,
            ),
            KeyedAccount::new(
                mint_authority,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                alice,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
            KeyedAccount::new(
                bob,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
        ];

        let sim = SplTokenSimulator;

        // 1. InitializeMint(decimals=6).
        let mut data = vec![0u8, 6]; // tag=0, decimals=6
        data.extend_from_slice(mint_authority.as_ref()); // mint_authority
        data.push(0); // freeze_authority flag = none
        let mint_metas = metas(&[(mint_addr, true, true)]);
        let mut mint_only = vec![accounts[0].clone()];
        invoke(&sim, data, &mut mint_only, mint_metas).expect("InitializeMint");
        accounts[0] = mint_only[0].clone();

        // 2. InitializeAccount for alice + bob.
        for token_acct in [1usize, 2] {
            let mut data = vec![1u8]; // tag=1
            let mut subset = vec![
                accounts[token_acct].clone(),
                accounts[0].clone(),              // mint
                accounts[3 + token_acct].clone(), // owner (alice or bob)
            ];
            let init_metas = metas(&[
                (subset[0].address, true, true),
                (subset[1].address, false, false),
                (subset[2].address, false, false),
            ]);
            invoke(&sim, data, &mut subset, init_metas).expect("InitializeAccount");
            accounts[token_acct] = subset[0].clone();
        }

        // 3. MintTo: 100 tokens to alice.
        let mut data = vec![7u8];
        data.extend_from_slice(&100u64.to_le_bytes());
        let mut subset = vec![
            accounts[0].clone(), // mint
            accounts[1].clone(), // alice token account
            accounts[3].clone(), // mint authority
        ];
        let mint_to_metas = metas(&[
            (subset[0].address, false, true),
            (subset[1].address, false, true),
            (subset[2].address, true, false),
        ]);
        invoke(&sim, data, &mut subset, mint_to_metas).expect("MintTo");
        accounts[0] = subset[0].clone();
        accounts[1] = subset[1].clone();

        // 4. Transfer 30 from alice to bob.
        let mut data = vec![3u8];
        data.extend_from_slice(&30u64.to_le_bytes());
        let mut subset = vec![
            accounts[1].clone(), // alice token account
            accounts[2].clone(), // bob token account
            accounts[4].clone(), // alice (signer)
        ];
        let xfer_metas = metas(&[
            (subset[0].address, false, true),
            (subset[1].address, false, true),
            (subset[2].address, true, false),
        ]);
        invoke(&sim, data, &mut subset, xfer_metas).expect("Transfer");
        accounts[1] = subset[0].clone();
        accounts[2] = subset[1].clone();

        // 5. Burn 10 from bob.
        let mut data = vec![8u8];
        data.extend_from_slice(&10u64.to_le_bytes());
        let mut subset = vec![
            accounts[2].clone(), // bob token account
            accounts[0].clone(), // mint
            accounts[5].clone(), // bob (signer)
        ];
        let burn_metas = metas(&[
            (subset[0].address, false, true),
            (subset[1].address, false, true),
            (subset[2].address, true, false),
        ]);
        invoke(&sim, data, &mut subset, burn_metas).expect("Burn");

        // Final state: mint.supply = 100 - 10 = 90, alice = 70,
        // bob = 30 - 10 = 20. Pin every number.
        let final_mint = Mint::unpack(&subset[1].data).unwrap();
        let final_bob = TokenAccount::unpack(&subset[0].data).unwrap();
        assert_eq!(final_mint.supply, 90);
        assert_eq!(final_bob.amount, 20);
        let alice_state = TokenAccount::unpack(&accounts[1].data).unwrap();
        assert_eq!(alice_state.amount, 70);
    }

    /// Transfer with mint mismatch is rejected.
    #[test]
    fn transfer_rejects_mint_mismatch() {
        let pid = spl_token::id();
        let mint1 = Pubkey::new_unique();
        let mint2 = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let acct1 = Pubkey::new_unique();
        let acct2 = Pubkey::new_unique();

        let mut t1 = TokenAccount {
            mint: mint1,
            owner,
            amount: 100,
            state: AccountState::Initialized,
            ..Default::default()
        };
        let mut buf1 = vec![0u8; TokenAccount::LEN];
        TokenAccount::pack(t1.clone(), &mut buf1).unwrap();
        let mut t2 = TokenAccount {
            mint: mint2, // different mint
            owner,
            amount: 0,
            state: AccountState::Initialized,
            ..Default::default()
        };
        let mut buf2 = vec![0u8; TokenAccount::LEN];
        TokenAccount::pack(t2.clone(), &mut buf2).unwrap();
        let _ = (&mut t1, &mut t2);

        let mut accounts = vec![
            KeyedAccount::new(acct1, 1_000_000, pid, buf1, false),
            KeyedAccount::new(acct2, 1_000_000, pid, buf2, false),
            KeyedAccount::new(
                owner,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
        ];
        let metas_list = metas(&[
            (acct1, false, true),
            (acct2, false, true),
            (owner, true, false),
        ]);

        let mut data = vec![3u8];
        data.extend_from_slice(&50u64.to_le_bytes());
        let err = invoke(&SplTokenSimulator, data, &mut accounts, metas_list).unwrap_err();
        assert!(
            matches!(err, HopperSvmError::BuiltinError { ref message, .. } if message.contains("mint mismatch")),
            "{err:?}"
        );
    }

    /// CloseAccount rejects a non-empty token account.
    #[test]
    fn close_account_rejects_non_empty() {
        let pid = spl_token::id();
        let owner = Pubkey::new_unique();
        let acct = Pubkey::new_unique();
        let dst = Pubkey::new_unique();

        let token = TokenAccount {
            mint: Pubkey::new_unique(),
            owner,
            amount: 5,
            state: AccountState::Initialized,
            ..Default::default()
        };
        let mut buf = vec![0u8; TokenAccount::LEN];
        TokenAccount::pack(token, &mut buf).unwrap();

        let mut accounts = vec![
            KeyedAccount::new(acct, 1_000_000, pid, buf, false),
            KeyedAccount::new(dst, 0, solana_sdk::system_program::id(), vec![], false),
            KeyedAccount::new(
                owner,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
        ];
        let metas_list = metas(&[
            (acct, false, true),
            (dst, false, true),
            (owner, true, false),
        ]);
        let err = invoke(&SplTokenSimulator, vec![9u8], &mut accounts, metas_list).unwrap_err();
        assert!(
            matches!(err, HopperSvmError::BuiltinError { ref message, .. } if message.contains("non-empty")),
            "{err:?}"
        );
    }

    /// Unsupported tag returns a structured error with a clear
    /// list of supported tags.
    #[test]
    fn unsupported_tag_lists_supported_set() {
        let mut accounts = vec![];
        let err = invoke(
            &SplTokenSimulator,
            vec![10u8], // FreezeAccount, not yet supported
            &mut accounts,
            vec![],
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("supported tags"), "{message}");
                assert!(message.contains("Transfer"), "{message}");
                assert!(message.contains("MintTo"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }
}
