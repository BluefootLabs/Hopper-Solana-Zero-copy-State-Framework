//! Account types — Hopper-flavored, no upstream wrapper crates.
//!
//! `KeyedAccount` is the universal account-with-address shape across
//! the harness. Construction is free-form (`KeyedAccount::new`) or
//! via the `token::*` factories. Conversion to and from
//! `solana_sdk::account::Account` is provided so users can interoperate
//! with the wider Solana ecosystem (e.g. seeding test fixtures from
//! a JSON RPC dump).

use solana_sdk::account::Account as SolanaAccount;
use solana_sdk::pubkey::Pubkey;

/// A `(Pubkey, Account)` pair — the universal account shape across
/// the `hopper-svm` API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyedAccount {
    /// Account address.
    pub address: Pubkey,
    /// SOL balance, in lamports.
    pub lamports: u64,
    /// Raw account data.
    pub data: Vec<u8>,
    /// Account owner — typically a program ID.
    pub owner: Pubkey,
    /// Whether this account is an executable program.
    pub executable: bool,
    /// Rent epoch — purely informational on the Hopper side, set to
    /// 0 by default since Phase 1 doesn't simulate rent collection.
    pub rent_epoch: u64,
}

impl KeyedAccount {
    /// Build a fresh keyed account from individual fields.
    pub fn new(
        address: Pubkey,
        lamports: u64,
        owner: Pubkey,
        data: Vec<u8>,
        executable: bool,
    ) -> Self {
        Self {
            address,
            lamports,
            data,
            owner,
            executable,
            rent_epoch: 0,
        }
    }

    /// Convert into the upstream `solana_sdk::Account` shape. Useful
    /// for interop with crates that expect that exact type.
    pub fn into_solana_account(self) -> (Pubkey, SolanaAccount) {
        let acct = SolanaAccount {
            lamports: self.lamports,
            data: self.data,
            owner: self.owner,
            executable: self.executable,
            rent_epoch: self.rent_epoch,
        };
        (self.address, acct)
    }

    /// Lift an upstream `(Pubkey, solana_sdk::Account)` pair into a
    /// `KeyedAccount` — the inverse of [`into_solana_account`]. Used
    /// when seeding tests from RPC dumps or when interoperating with
    /// other Solana-ecosystem tools.
    pub fn from_solana_account(addr: Pubkey, acct: SolanaAccount) -> Self {
        Self {
            address: addr,
            lamports: acct.lamports,
            data: acct.data,
            owner: acct.owner,
            executable: acct.executable,
            rent_epoch: acct.rent_epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip the conversion through `solana_sdk::Account`. If
    /// either direction loses a field the silent corruption shows up
    /// in every test downstream — pin it here.
    #[test]
    fn solana_account_round_trip() {
        let addr = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let original = KeyedAccount::new(addr, 1_337, owner, vec![1, 2, 3, 4], false);
        let (a2, sol) = original.clone().into_solana_account();
        let lifted = KeyedAccount::from_solana_account(a2, sol);
        assert_eq!(lifted, original);
    }
}
