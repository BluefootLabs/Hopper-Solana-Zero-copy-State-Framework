//! Token-account factories. Build pre-initialised SPL Token /
//! Token-2022 / ATA accounts as `KeyedAccount` ready to feed into
//! [`crate::HopperSvm::process_instruction`].
//!
//! Every factory is pure Rust — we serialise the SPL wire shape via
//! `Pack` directly. There is no SVM execution involved in *creating*
//! a token account; the SVM only matters when an SPL program
//! actually runs (Phase 2). Phase 1 covers every test case that can
//! seed pre-existing token state and exercise non-SPL business logic
//! against it.

use crate::account::KeyedAccount;
use crate::{ASSOCIATED_TOKEN_PROGRAM_ID, SPL_TOKEN_PROGRAM_ID};
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_program;
use spl_token::state::{
    Account as SplTokenAccount, AccountState as SplAccountState, Mint as SplMint,
};

/// Inline ATA derivation. Re-implements
/// `spl_associated_token_account::get_associated_token_address_with_program_id`
/// against the canonical seeds so we avoid the type skew between
/// `solana_sdk::pubkey::Pubkey` and the `solana_address::Address` that
/// `spl-associated-token-account 8` now uses internally. Same algorithm
/// (PDA over `[wallet, token_program, mint]` against the SPL ATA program ID).
#[inline]
fn get_associated_token_address_with_program_id(
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    let (ata, _bump) = Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    ata
}

// Re-export the data-only types so `use hopper_svm::token::*;` brings
// `Mint` / `TokenAccount` / `AccountState` into scope.
pub use spl_token::state::Account as TokenAccount;
pub use spl_token::state::AccountState;
pub use spl_token::state::Mint;

/// 1 SOL — well above rent-exempt for any token account or mint we'd
/// produce here. Picked to match what wallets typically fund test
/// accounts with so balance assertions read naturally.
const DEFAULT_TEST_LAMPORTS: u64 = 1_000_000_000;

/// Build a system-owned account with a given lamport balance. The
/// data buffer is empty — appropriate for fee-payer keypairs,
/// signers without state, or any "just need a wallet" slot.
pub fn create_keyed_system_account(address: &Pubkey, lamports: u64) -> KeyedAccount {
    KeyedAccount::new(*address, lamports, system_program::id(), Vec::new(), false)
}

/// Build a pre-initialised SPL Token mint owned by the legacy
/// `spl-token` program.
pub fn create_keyed_mint_account(address: &Pubkey, mint: &Mint) -> KeyedAccount {
    create_keyed_mint_account_with_program(address, mint, &SPL_TOKEN_PROGRAM_ID)
}

/// Build a pre-initialised mint owned by the program ID of the
/// caller's choice. Pass `SPL_TOKEN_2022_PROGRAM_ID` to produce a
/// Token-2022 mint.
pub fn create_keyed_mint_account_with_program(
    address: &Pubkey,
    mint: &Mint,
    token_program: &Pubkey,
) -> KeyedAccount {
    let mut data = vec![0u8; SplMint::LEN];
    Mint::pack(*mint, &mut data).expect("Mint pack");
    KeyedAccount::new(*address, DEFAULT_TEST_LAMPORTS, *token_program, data, false)
}

/// Build a pre-initialised SPL token account.
///
/// `token.state` defaults to `AccountState::Initialized` if you used
/// `..Default::default()`. The factory passes an `Uninitialized`
/// state through unchanged but flips an unset `Default::default()`
/// state to `Initialized` so the common case is "an account that
/// already works."
pub fn create_keyed_token_account(address: &Pubkey, token: &TokenAccount) -> KeyedAccount {
    create_keyed_token_account_with_program(address, token, &SPL_TOKEN_PROGRAM_ID)
}

/// Build a pre-initialised token account owned by the given token
/// program. Pass `SPL_TOKEN_2022_PROGRAM_ID` for Token-2022.
pub fn create_keyed_token_account_with_program(
    address: &Pubkey,
    token: &TokenAccount,
    token_program: &Pubkey,
) -> KeyedAccount {
    let mut data = vec![0u8; SplTokenAccount::LEN];
    let mut copy = *token;
    if matches!(copy.state, SplAccountState::Uninitialized) {
        copy.state = SplAccountState::Initialized;
    }
    SplTokenAccount::pack(copy, &mut data).expect("TokenAccount pack");
    KeyedAccount::new(*address, DEFAULT_TEST_LAMPORTS, *token_program, data, false)
}

/// Build a pre-initialised associated token account.
///
/// The ATA address is derived deterministically from
/// `(wallet, mint, SPL_TOKEN_PROGRAM_ID)`. The returned
/// `KeyedAccount.address` is the derived ATA, not the wallet.
pub fn create_keyed_associated_token_account(
    wallet: &Pubkey,
    mint: &Pubkey,
    amount: u64,
) -> KeyedAccount {
    create_keyed_associated_token_account_with_program(wallet, mint, amount, &SPL_TOKEN_PROGRAM_ID)
}

/// Token-2022 (or any token-program) ATA variant. The ATA derivation
/// includes the token-program ID, so a Token and Token-2022 ATA for
/// the same `(wallet, mint)` pair derive to *different* addresses.
pub fn create_keyed_associated_token_account_with_program(
    wallet: &Pubkey,
    mint: &Pubkey,
    amount: u64,
    token_program: &Pubkey,
) -> KeyedAccount {
    let ata = get_associated_token_address_with_program_id(wallet, mint, token_program);
    let token = TokenAccount {
        mint: *mint,
        owner: *wallet,
        amount,
        delegate: None.into(),
        state: SplAccountState::Initialized,
        is_native: None.into(),
        delegated_amount: 0,
        close_authority: None.into(),
    };
    create_keyed_token_account_with_program(&ata, &token, token_program)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SPL_TOKEN_2022_PROGRAM_ID;

    /// Mint round-trips through `Pack`. Pin against pack-unpack
    /// drift so the rest of the harness can trust mint factory output.
    #[test]
    fn mint_pack_round_trips() {
        let addr = Pubkey::new_unique();
        let m = Mint {
            decimals: 6,
            supply: 1_000,
            is_initialized: true,
            ..Default::default()
        };
        let acct = create_keyed_mint_account(&addr, &m);
        let unpacked = Mint::unpack(&acct.data).expect("unpack");
        assert_eq!(unpacked.decimals, 6);
        assert_eq!(unpacked.supply, 1_000);
        assert_eq!(acct.owner, SPL_TOKEN_PROGRAM_ID);
    }

    /// Token-account factory auto-flips Uninitialized to
    /// Initialized so the common case "just default-construct"
    /// produces a usable account.
    #[test]
    fn token_factory_initializes_by_default() {
        let mint = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let addr = Pubkey::new_unique();
        let token = TokenAccount {
            mint,
            owner,
            amount: 42,
            ..Default::default()
        };
        let acct = create_keyed_token_account(&addr, &token);
        let unpacked = TokenAccount::unpack(&acct.data).expect("unpack");
        assert!(matches!(unpacked.state, SplAccountState::Initialized));
        assert_eq!(unpacked.amount, 42);
    }

    /// ATA derivation: legacy and Token-2022 IDs derive different
    /// addresses, both matching upstream `spl-associated-token-account`.
    #[test]
    fn ata_derivation_matches_spl_helper() {
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let legacy = create_keyed_associated_token_account(&wallet, &mint, 0);
        let expected =
            get_associated_token_address_with_program_id(&wallet, &mint, &SPL_TOKEN_PROGRAM_ID);
        assert_eq!(legacy.address, expected);

        let t22 = create_keyed_associated_token_account_with_program(
            &wallet,
            &mint,
            0,
            &SPL_TOKEN_2022_PROGRAM_ID,
        );
        let expected_t22 = get_associated_token_address_with_program_id(
            &wallet,
            &mint,
            &SPL_TOKEN_2022_PROGRAM_ID,
        );
        assert_eq!(t22.address, expected_t22);
        assert_ne!(legacy.address, t22.address);
    }
}
