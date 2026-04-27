//! SPL Token-2022 simulator.
//!
//! Token-2022 is a superset of SPL Token: the legacy 9 tags
//! (`InitializeMint`, `InitializeAccount`, `Transfer`, `Approve`,
//! `Revoke`, `MintTo`, `Burn`, `CloseAccount`, etc.) accept the
//! same wire format on both programs, and on a non-extension
//! mint or token account the on-disk layout is identical
//! (82-byte Mint, 165-byte Account; extensions live in a TLV
//! region appended after).
//!
//! The Phase-1 Token-2022 simulator delegates the common tags
//! to the SPL Token simulator's logic — the owner-check inside
//! those handlers compares against `ctx.program_id`, which is
//! already the Token-2022 ID when invoked from a Token-2022
//! dispatch path, so accounts owned by Token-2022 pass cleanly.
//!
//! ## Coverage
//!
//! - **Common tags 0-9** — same surface as
//!   [`crate::spl::token::SplTokenSimulator`]:
//!   `InitializeMint`, `InitializeAccount`, `Transfer`,
//!   `Approve`, `Revoke`, `MintTo`, `Burn`, `CloseAccount`.
//! - **Extension tags 22+** — return a structured
//!   "extensions not yet supported by Phase 1" error so tests
//!   that hit them fail fast with a clear message. Programs
//!   that need a Token-2022 extension can register the real
//!   `.so` via [`crate::HopperSvm::add_program`] in the
//!   meantime.
//!
//! ## On-curve note
//!
//! Token-2022 mints with extensions like `TransferFeeConfig`,
//! `MintCloseAuthority`, `ConfidentialTransferMint`,
//! `DefaultAccountState`, `NonTransferable`,
//! `InterestBearingConfig`, `PermanentDelegate`,
//! `TransferHook`, `MetadataPointer`, `GroupPointer`,
//! `GroupMemberPointer`, `Pausable`, `ScaledUiAmountConfig`,
//! `Group`, `GroupMember`, `Member`, `MetadataConfig`, etc. —
//! these all initialise the extension TLV region with their
//! own state tags and are out-of-scope for Phase 1.

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use crate::spl::token::SplTokenSimulator;

/// Same per-instruction CU baseline as legacy Token. Extensions
/// would charge more in production; we'll calibrate when the
/// extension surface lands.
const TOKEN_2022_INSTRUCTION_CU: u64 = 4_000;

/// SPL Token-2022 program reference simulator. Register with
/// [`crate::HopperSvm::with_spl_token_2022_simulator`].
pub struct SplToken2022Simulator;

impl BuiltinProgram for SplToken2022Simulator {
    fn name(&self) -> &'static str {
        "spl-token-2022 (simulated)"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        TOKEN_2022_INSTRUCTION_CU
    }

    fn invoke(
        &self,
        data: &[u8],
        accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError> {
        let (tag, _) = data
            .split_first()
            .ok_or_else(|| HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "spl-token-2022: empty instruction data".to_string(),
            })?;

        match *tag {
            // Legacy / common tags — wire-compatible with Token,
            // so we delegate. The owner check inside the Token
            // simulator's handlers compares against `ctx.program_id`
            // which is the Token-2022 ID here, so accounts owned
            // by Token-2022 pass cleanly.
            0 | 1 | 3 | 4 | 5 | 7 | 8 | 9 => {
                SplTokenSimulator.invoke(data, accounts, ctx)
            }
            // Tags 2 (`InitializeMultisig`), 6 (`SetAuthority`),
            // 10 (`FreezeAccount`), 11 (`ThawAccount`),
            // 12-13 (`*Checked` variants), 14 (`SyncNative`),
            // 15-16 (`InitializeAccount2/3`), 17 (`SyncNative`
            // alias), 18-20 (`Initialize{Mint,Account}3`,
            // `*WithExtensions`), 21 (`InitializeMint2`) — these
            // exist in legacy Token and Token-2022 but aren't in
            // the Phase 1 simulator yet. Same "supported tag"
            // list as the Token simulator, just at the
            // Token-2022 dispatch site.
            2 | 6 | 10..=21 => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-token-2022: legacy tag {tag} not yet supported by the bundled simulator \
                     (supported tags: 0/InitializeMint, 1/InitializeAccount, 3/Transfer, \
                     4/Approve, 5/Revoke, 7/MintTo, 8/Burn, 9/CloseAccount). For unsupported \
                     instructions, register the real spl-token-2022 .so via \
                     `HopperSvm::add_program(&id, \"spl_token_2022\")`."
                ),
            }),
            // Token-2022 extensions: 22 = InitializeMintCloseAuthority,
            // 23 = TransferFeeExtension dispatcher, 24 = ConfidentialTransfer,
            // 25 = DefaultAccountState, 26 = ImmutableOwner,
            // 27 = MemoTransfer, 28 = CreateNativeMint,
            // 29 = NonTransferable, 30 = InterestBearing,
            // 31 = CpiGuard, 32 = InitializePermanentDelegate,
            // 33 = TransferHook, 34 = ConfidentialTransferFee,
            // 35 = WithdrawExcessLamports, 36 = MetadataPointer,
            // 37 = GroupPointer, 38 = GroupMemberPointer,
            // 39 = ConfidentialMintBurn, 40 = ScaledUiAmount,
            // 41 = Pausable
            22..=99 => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "spl-token-2022: extension tag {tag} not supported by the Phase-1 simulator. \
                     Token-2022 extensions (TransferFeeConfig, MintCloseAuthority, \
                     ConfidentialTransfer, NonTransferable, TransferHook, MetadataPointer, etc.) \
                     land in a follow-up. Register the real spl-token-2022 .so via \
                     `HopperSvm::add_program(&id, \"spl_token_2022\")` to test extension flows today."
                ),
            }),
            other => Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!("spl-token-2022: unknown instruction tag {other}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogCapture;
    use crate::sysvar::Sysvars;
    use solana_program_pack::Pack;
    use solana_sdk::instruction::AccountMeta;
    use solana_sdk::pubkey::Pubkey;
    use spl_token::state::{Account as TokenAccount, AccountState, Mint};

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
        sim: &SplToken2022Simulator,
        data: Vec<u8>,
        accounts: &mut Vec<KeyedAccount>,
        metas_list: Vec<AccountMeta>,
    ) -> Result<(), HopperSvmError> {
        let mut budget = ComputeBudget::default();
        let mut logs = LogCapture::default();
        let sysvars = Sysvars::default();
        let pid = spl_token_2022::id();
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas_list,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        sim.invoke(&data, accounts, &mut ctx)
    }

    /// Token-2022 should accept the same Transfer wire format as
    /// legacy Token. Pin the delegation: an account owned by
    /// the Token-2022 program ID, transferred via tag 3, results
    /// in correct balance updates.
    #[test]
    fn token_2022_transfer_delegates_to_token_logic() {
        let pid = spl_token_2022::id();
        let mint = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let acct1 = Pubkey::new_unique();
        let acct2 = Pubkey::new_unique();

        let t1 = TokenAccount {
            mint,
            owner,
            amount: 100,
            state: AccountState::Initialized,
            ..Default::default()
        };
        let t2 = TokenAccount {
            mint,
            owner,
            amount: 0,
            state: AccountState::Initialized,
            ..Default::default()
        };
        let mut buf1 = vec![0u8; TokenAccount::LEN];
        let mut buf2 = vec![0u8; TokenAccount::LEN];
        TokenAccount::pack(t1, &mut buf1).unwrap();
        TokenAccount::pack(t2, &mut buf2).unwrap();

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
        data.extend_from_slice(&30u64.to_le_bytes());
        invoke(&SplToken2022Simulator, data, &mut accounts, metas_list).expect("Transfer");

        let post1 = TokenAccount::unpack(&accounts[0].data).unwrap();
        let post2 = TokenAccount::unpack(&accounts[1].data).unwrap();
        assert_eq!(post1.amount, 70);
        assert_eq!(post2.amount, 30);
    }

    /// Extension tags (22+) return structured errors with a
    /// clear "register the real .so" hint.
    #[test]
    fn extension_tag_returns_structured_error() {
        let mut accounts = vec![];
        let err = invoke(
            &SplToken2022Simulator,
            vec![22u8], // InitializeMintCloseAuthority
            &mut accounts,
            vec![],
        )
        .unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("extension"), "{message}");
                assert!(message.contains("spl_token_2022"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Unimplemented legacy tags (e.g. SetAuthority = 6) return
    /// the supported-tag list so test failures stay actionable.
    #[test]
    fn unimplemented_legacy_tag_returns_structured_error() {
        let mut accounts = vec![];
        let err = invoke(&SplToken2022Simulator, vec![6u8], &mut accounts, vec![]).unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("supported tags"), "{message}");
                assert!(message.contains("Transfer"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }

    /// Tag 0 (InitializeMint) on Token-2022 also delegates and
    /// produces a Mint owned by the Token-2022 program.
    #[test]
    fn token_2022_initialize_mint_owns_account() {
        let pid = spl_token_2022::id();
        let mint = Pubkey::new_unique();
        let mint_authority = Pubkey::new_unique();

        let mut accounts = vec![KeyedAccount::new(
            mint,
            1_000_000,
            pid,
            vec![0u8; Mint::LEN],
            false,
        )];
        let mut data = vec![0u8, 6]; // tag=0, decimals=6
        data.extend_from_slice(mint_authority.as_ref());
        data.push(0); // freeze_authority flag = none
        let metas_list = metas(&[(mint, true, true)]);
        invoke(&SplToken2022Simulator, data, &mut accounts, metas_list).expect("InitializeMint");

        let m = Mint::unpack(&accounts[0].data).unwrap();
        assert_eq!(m.decimals, 6);
        assert!(m.is_initialized);
        // The account remained owned by the Token-2022 program ID.
        assert_eq!(accounts[0].owner, pid);
    }
}
