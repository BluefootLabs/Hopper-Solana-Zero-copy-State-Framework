//! Associated Token Account program simulator.
//!
//! The ATA program creates the canonical `(wallet, mint, token_program)`-
//! derived token account in a single instruction. On mainnet it
//! does this by CPI'ing into the system program (CreateAccount)
//! and the token program (InitializeAccount). The Phase-1
//! simulator inlines both steps — no CPI dispatch needed
//! because the operations are deterministic and we have direct
//! access to the account state.
//!
//! ## Coverage
//!
//! | Tag | Instruction       | Implemented |
//! |-----|-------------------|-------------|
//! |  0  | `Create`          | ✓           |
//! |  1  | `CreateIdempotent`| ✓           |
//!
//! Tag 2 (`RecoverNested`) is rare and lands in a follow-up.
//!
//! ## Wire format
//!
//! ```text
//! Create:
//!   data = [tag = 0]                        (1 byte)
//!   accounts:
//!     0: funding_address  (writable, signer)
//!     1: ata              (writable; address must match derivation)
//!     2: wallet           (read-only)
//!     3: mint             (read-only; owner = token_program)
//!     4: system_program   (read-only)
//!     5: token_program    (read-only; either spl-token or spl-token-2022)
//!     6: rent sysvar      (read-only; optional in modern ATA)
//!
//! CreateIdempotent:
//!   data = [tag = 1]
//!   accounts: same as Create
//!   behaviour: succeeds without error if the ATA already exists
//!     (state = Initialized + correct mint + correct owner).
//! ```
//!
//! ## Validation
//!
//! - `accounts[1].address` must equal
//!   `get_associated_token_address_with_program_id(wallet, mint, token_program)`.
//! - `accounts[5]` must be one of `spl_token::id()` or
//!   `spl_token_2022::id()`.
//! - `accounts[0]` must be the signer (funding the rent
//!   exemption).
//! - On `Create`, the ATA must be empty (`lamports == 0` AND
//!   `data.is_empty()`); on `CreateIdempotent`, an existing
//!   correctly-formed ATA returns `Ok(())` without mutation.

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_program;
use spl_token::state::{Account as TokenAccount, AccountState};

/// SPL Associated Token Account program ID, hard-coded as the legacy
/// `Pubkey` type. Avoids the type skew between
/// `solana_sdk::pubkey::Pubkey` (used here) and the
/// `solana_address::Address` that `spl-associated-token-account-interface 2.0`
/// returns from its own `id()` helper.
///
/// Bytes match the canonical address `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL`.
const ATA_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    0x8c, 0x97, 0x25, 0x8f, 0x4e, 0x24, 0x89, 0xf1, 0xbb, 0x3d, 0x10, 0x29, 0x14, 0x8e, 0x0d, 0x83,
    0x0b, 0x5a, 0x13, 0x99, 0xda, 0xff, 0x10, 0x84, 0x04, 0x8e, 0x7b, 0xd8, 0xdb, 0xe9, 0xf8, 0x59,
]);

/// Inline ATA derivation. Re-implements
/// `spl_associated_token_account::get_associated_token_address_with_program_id`
/// against the canonical seeds so we avoid the type-skew between
/// `solana_sdk::pubkey::Pubkey` and `solana_address::Address`.
/// Algorithm is the same: PDA derivation over
/// `[wallet, token_program, mint]` against the SPL ATA program ID.
#[inline]
fn get_associated_token_address_with_program_id(
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    let (ata, _bump) = Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ATA_PROGRAM_ID,
    );
    ata
}

/// CU baseline for ATA Create. Mainnet charges around 30 000
/// CU; we charge a flat 25 000 to leave headroom.
const ATA_INSTRUCTION_CU: u64 = 25_000;

/// Default lamports for a fresh ATA — well above rent-exempt
/// for the 165-byte account.
const ATA_DEFAULT_LAMPORTS: u64 = 2_039_280;

/// SPL Associated Token Account program reference simulator.
/// Register with
/// [`crate::HopperSvm::with_spl_associated_token_simulator`].
pub struct SplAtaSimulator;

impl BuiltinProgram for SplAtaSimulator {
    fn name(&self) -> &'static str {
        "spl-associated-token-account (simulated)"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        ATA_INSTRUCTION_CU
    }

    fn invoke(
        &self,
        data: &[u8],
        accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError> {
        // Empty data is the legacy "Create" form (the ATA
        // program originally took no instruction data).
        // Modern ATA encodes Create as tag = 0, CreateIdempotent
        // as tag = 1.
        let (idempotent, body) = match data.split_first() {
            Some((0, body)) => (false, body),
            Some((1, body)) => (true, body),
            None => (false, &[][..]),
            Some((other, _)) => {
                return Err(HopperSvmError::BuiltinError {
                    program_id: *ctx.program_id,
                    message: format!(
                        "spl-ata: unknown tag {other} (supported: 0/Create, 1/CreateIdempotent)"
                    ),
                })
            }
        };
        let _ = body;

        if accounts.len() < 6 {
            return Err(HopperSvmError::AccountIndexOutOfBounds {
                index: 5,
                len: accounts.len(),
            });
        }
        let funding_addr = accounts[0].address;
        let ata_addr = accounts[1].address;
        let wallet_addr = accounts[2].address;
        let mint_addr = accounts[3].address;
        let system_program_addr = accounts[4].address;
        let token_program_addr = accounts[5].address;

        ctx.require_signer(&funding_addr)?;
        ctx.require_writable(&funding_addr)?;
        ctx.require_writable(&ata_addr)?;

        // Validate the system program address.
        if system_program_addr != system_program::id() {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-ata: system program slot is {system_program_addr} (expected {})",
                    system_program::id()
                ),
            });
        }

        // Validate the token program address — must be one of
        // the supported token programs.
        if token_program_addr != spl_token::id() && token_program_addr != spl_token_2022::id() {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-ata: token program slot is {token_program_addr} (expected spl-token or spl-token-2022)"
                ),
            });
        }

        // Validate the ATA address derivation.
        let expected = get_associated_token_address_with_program_id(
            &wallet_addr,
            &mint_addr,
            &token_program_addr,
        );
        if ata_addr != expected {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-ata: ATA address mismatch (got {ata_addr}, derived {expected} from wallet={wallet_addr} mint={mint_addr} token_program={token_program_addr})"
                ),
            });
        }

        // Validate the mint exists and is owned by the
        // declared token program.
        if accounts[3].owner != token_program_addr {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-ata: mint {mint_addr} is owned by {} (expected {token_program_addr})",
                    accounts[3].owner
                ),
            });
        }

        // Idempotent path: if the ATA already exists with the
        // correct mint + owner, return Ok without mutation.
        let already_exists = accounts[1].lamports != 0
            || !accounts[1].data.is_empty()
            || accounts[1].owner != system_program::id();
        if already_exists {
            if !idempotent {
                return Err(HopperSvmError::BuiltinError {
                    program_id: *ctx.program_id,
                    message: format!(
                        "spl-ata::Create: ATA {ata_addr} already exists (use CreateIdempotent if that's expected)"
                    ),
                });
            }
            // CreateIdempotent: validate the existing ATA is
            // correctly formed before returning Ok.
            if accounts[1].owner != token_program_addr {
                return Err(HopperSvmError::BuiltinError {
                    program_id: *ctx.program_id,
                    message: format!(
                        "spl-ata::CreateIdempotent: ATA {ata_addr} exists but is owned by {} (expected {token_program_addr})",
                        accounts[1].owner
                    ),
                });
            }
            if let Ok(token) = TokenAccount::unpack(&accounts[1].data) {
                if token.mint != mint_addr || token.owner != wallet_addr {
                    return Err(HopperSvmError::BuiltinError {
                        program_id: *ctx.program_id,
                        message: format!(
                            "spl-ata::CreateIdempotent: existing ATA mismatch (mint={} expected={mint_addr}, owner={} expected={wallet_addr})",
                            token.mint, token.owner
                        ),
                    });
                }
            }
            ctx.log(format!(
                "spl-ata::CreateIdempotent: {ata_addr} already exists, no-op"
            ));
            return Ok(());
        }

        // Create path: fund the ATA, allocate 165 bytes, set
        // owner to the token program, initialise as a token
        // account.
        if accounts[0].lamports < ATA_DEFAULT_LAMPORTS {
            return Err(HopperSvmError::InsufficientFunds {
                account: funding_addr,
                balance: accounts[0].lamports,
                requested: ATA_DEFAULT_LAMPORTS,
            });
        }
        accounts[0].lamports -= ATA_DEFAULT_LAMPORTS;
        accounts[1].lamports = ATA_DEFAULT_LAMPORTS;
        accounts[1].data = vec![0u8; TokenAccount::LEN];
        accounts[1].owner = token_program_addr;
        accounts[1].executable = false;

        let token = TokenAccount {
            mint: mint_addr,
            owner: wallet_addr,
            amount: 0,
            delegate: None.into(),
            state: AccountState::Initialized,
            is_native: None.into(),
            delegated_amount: 0,
            close_authority: None.into(),
        };
        TokenAccount::pack(token, &mut accounts[1].data)
            .expect("TokenAccount::pack into 165-byte buffer");

        ctx.log(format!(
            "spl-ata::Create: {ata_addr} (wallet={wallet_addr}, mint={mint_addr}, token_program={token_program_addr})"
        ));
        Ok(())
    }
}

/// Convenience: returns the program ID of the
/// Associated Token Account program. Rebound here so callers
/// don't need to import `spl_associated_token_account` directly.
pub fn associated_token_program_id() -> Pubkey {
    ATA_PROGRAM_ID
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogCapture;
    use crate::sysvar::Sysvars;
    use solana_sdk::instruction::AccountMeta;
    use spl_token::state::Mint;

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

    fn mint_account(addr: Pubkey, token_program: Pubkey) -> KeyedAccount {
        let mut data = vec![0u8; Mint::LEN];
        let mint = Mint {
            mint_authority: None.into(),
            supply: 0,
            decimals: 9,
            is_initialized: true,
            freeze_authority: None.into(),
        };
        Mint::pack(mint, &mut data).unwrap();
        KeyedAccount::new(addr, 1_000_000, token_program, data, false)
    }

    fn empty_account(addr: Pubkey) -> KeyedAccount {
        KeyedAccount::new(addr, 0, system_program::id(), vec![], false)
    }

    fn invoke(
        sim: &SplAtaSimulator,
        data: Vec<u8>,
        accounts: &mut Vec<KeyedAccount>,
        metas_list: Vec<AccountMeta>,
    ) -> Result<(), HopperSvmError> {
        let mut budget = ComputeBudget::new(100_000, 25_000);
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = associated_token_program_id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas_list,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        sim.invoke(&data, accounts, &mut ctx)
    }

    /// Create produces a correctly-derived, correctly-funded
    /// ATA owned by the legacy SPL Token program.
    #[test]
    fn create_legacy_token_ata_succeeds() {
        let funding = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let ata = get_associated_token_address_with_program_id(&wallet, &mint, &spl_token::id());

        let mut accounts = vec![
            KeyedAccount::new(funding, 5_000_000, system_program::id(), vec![], false),
            empty_account(ata),
            empty_account(wallet),
            mint_account(mint, spl_token::id()),
            empty_account(system_program::id()),
            empty_account(spl_token::id()),
            empty_account(Pubkey::new_unique()), // rent sysvar (unused here)
        ];
        let metas_list = metas(&[
            (funding, true, true),
            (ata, false, true),
            (wallet, false, false),
            (mint, false, false),
            (system_program::id(), false, false),
            (spl_token::id(), false, false),
            (accounts[6].address, false, false),
        ]);
        invoke(&SplAtaSimulator, vec![0u8], &mut accounts, metas_list).expect("Create");

        assert_eq!(accounts[1].owner, spl_token::id());
        assert_eq!(accounts[1].lamports, ATA_DEFAULT_LAMPORTS);
        let token = TokenAccount::unpack(&accounts[1].data).unwrap();
        assert_eq!(token.mint, mint);
        assert_eq!(token.owner, wallet);
        assert!(matches!(token.state, AccountState::Initialized));
        // Funding account debited.
        assert_eq!(accounts[0].lamports, 5_000_000 - ATA_DEFAULT_LAMPORTS);
    }

    /// Create on Token-2022 ATA derives a different address and
    /// owns it under the Token-2022 program.
    #[test]
    fn create_token_2022_ata_succeeds() {
        let funding = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let ata =
            get_associated_token_address_with_program_id(&wallet, &mint, &spl_token_2022::id());

        let mut accounts = vec![
            KeyedAccount::new(funding, 5_000_000, system_program::id(), vec![], false),
            empty_account(ata),
            empty_account(wallet),
            mint_account(mint, spl_token_2022::id()),
            empty_account(system_program::id()),
            empty_account(spl_token_2022::id()),
            empty_account(Pubkey::new_unique()),
        ];
        let metas_list = metas(&[
            (funding, true, true),
            (ata, false, true),
            (wallet, false, false),
            (mint, false, false),
            (system_program::id(), false, false),
            (spl_token_2022::id(), false, false),
            (accounts[6].address, false, false),
        ]);
        invoke(&SplAtaSimulator, vec![0u8], &mut accounts, metas_list).expect("Create");
        assert_eq!(accounts[1].owner, spl_token_2022::id());
    }

    /// Create with a wrong derived ATA address is rejected with
    /// a clear error.
    #[test]
    fn create_rejects_wrong_derived_address() {
        let funding = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let bogus_ata = Pubkey::new_unique(); // not derived

        let mut accounts = vec![
            KeyedAccount::new(funding, 5_000_000, system_program::id(), vec![], false),
            empty_account(bogus_ata),
            empty_account(wallet),
            mint_account(mint, spl_token::id()),
            empty_account(system_program::id()),
            empty_account(spl_token::id()),
            empty_account(Pubkey::new_unique()),
        ];
        let metas_list = metas(&[
            (funding, true, true),
            (bogus_ata, false, true),
            (wallet, false, false),
            (mint, false, false),
            (system_program::id(), false, false),
            (spl_token::id(), false, false),
            (accounts[6].address, false, false),
        ]);
        let err = invoke(&SplAtaSimulator, vec![0u8], &mut accounts, metas_list).unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("address mismatch"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// CreateIdempotent on an already-existing correctly-formed
    /// ATA returns Ok without mutation.
    #[test]
    fn create_idempotent_on_existing_ata_is_noop() {
        let funding = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let ata = get_associated_token_address_with_program_id(&wallet, &mint, &spl_token::id());

        // Pre-build an existing, correctly-formed ATA.
        let mut buf = vec![0u8; TokenAccount::LEN];
        let token = TokenAccount {
            mint,
            owner: wallet,
            amount: 42, // already has tokens
            state: AccountState::Initialized,
            ..Default::default()
        };
        TokenAccount::pack(token, &mut buf).unwrap();

        let mut accounts = vec![
            KeyedAccount::new(funding, 5_000_000, system_program::id(), vec![], false),
            KeyedAccount::new(ata, ATA_DEFAULT_LAMPORTS, spl_token::id(), buf, false),
            empty_account(wallet),
            mint_account(mint, spl_token::id()),
            empty_account(system_program::id()),
            empty_account(spl_token::id()),
            empty_account(Pubkey::new_unique()),
        ];
        let metas_list = metas(&[
            (funding, true, true),
            (ata, false, true),
            (wallet, false, false),
            (mint, false, false),
            (system_program::id(), false, false),
            (spl_token::id(), false, false),
            (accounts[6].address, false, false),
        ]);
        let funding_before = accounts[0].lamports;
        invoke(&SplAtaSimulator, vec![1u8], &mut accounts, metas_list).expect("CreateIdempotent");
        // Funding account NOT charged again.
        assert_eq!(accounts[0].lamports, funding_before);
        // ATA still has the original 42 tokens.
        let token = TokenAccount::unpack(&accounts[1].data).unwrap();
        assert_eq!(token.amount, 42);
    }

    /// CreateIdempotent on an existing-but-wrong ATA (owned by
    /// a different program, or wrong mint) errors.
    #[test]
    fn create_idempotent_rejects_wrong_existing_ata() {
        let funding = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let other_mint = Pubkey::new_unique();
        let ata = get_associated_token_address_with_program_id(&wallet, &mint, &spl_token::id());

        // Existing ATA, but with the wrong mint.
        let mut buf = vec![0u8; TokenAccount::LEN];
        let token = TokenAccount {
            mint: other_mint, // mismatch
            owner: wallet,
            amount: 0,
            state: AccountState::Initialized,
            ..Default::default()
        };
        TokenAccount::pack(token, &mut buf).unwrap();

        let mut accounts = vec![
            KeyedAccount::new(funding, 5_000_000, system_program::id(), vec![], false),
            KeyedAccount::new(ata, ATA_DEFAULT_LAMPORTS, spl_token::id(), buf, false),
            empty_account(wallet),
            mint_account(mint, spl_token::id()),
            empty_account(system_program::id()),
            empty_account(spl_token::id()),
            empty_account(Pubkey::new_unique()),
        ];
        let metas_list = metas(&[
            (funding, true, true),
            (ata, false, true),
            (wallet, false, false),
            (mint, false, false),
            (system_program::id(), false, false),
            (spl_token::id(), false, false),
            (accounts[6].address, false, false),
        ]);
        let err = invoke(&SplAtaSimulator, vec![1u8], &mut accounts, metas_list).unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("mismatch"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }
}
