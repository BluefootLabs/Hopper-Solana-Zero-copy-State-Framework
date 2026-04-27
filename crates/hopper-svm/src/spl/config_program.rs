//! Config program — `BuiltinProgram` impl for
//! `Config1111111111111111111111111111111111111`.
//!
//! The Config program is the simplest validator-side built-in:
//! it stores arbitrary key-value config data on chain. Account
//! data layout is opaque (the program treats it as a blob). One
//! instruction, `Store`, replaces the data; signers must be one
//! of the registered keys + the account owner.
//!
//! ## Wire format
//!
//! `Store { keys: Vec<(Pubkey, bool)>, data: Vec<u8> }` —
//! bincode-shaped: `keys_len(u64) + (32 + 1) × n keys +
//! data_len(u64) + data`.
//!
//! ## Why it's worth shipping
//!
//! Almost no Hopper application program uses Config directly —
//! it's primarily for validator gossip + stake-config. But
//! mainnet parity means a complete simulator covers it. The
//! implementation is ~120 lines because the program's surface
//! is genuinely tiny.

use crate::account::KeyedAccount;
use crate::builtin::{BuiltinProgram, InvokeContext};
use crate::compute::ComputeBudget;
use crate::error::HopperSvmError;
use solana_sdk::pubkey::Pubkey;

/// Config program ID.
pub const CONFIG_PROGRAM_ID: Pubkey = solana_sdk::config::program::id();

/// CU baseline. Mainnet charges ~450 CU for a Store.
const CONFIG_INSTRUCTION_CU: u64 = 450;

/// Config program reference simulator. Register via
/// [`crate::HopperSvm::with_config_program`].
pub struct ConfigProgramSimulator;

impl BuiltinProgram for ConfigProgramSimulator {
    fn name(&self) -> &'static str {
        "config"
    }

    fn cost(&self, _budget: &ComputeBudget) -> u64 {
        CONFIG_INSTRUCTION_CU
    }

    fn invoke(
        &self,
        data: &[u8],
        accounts: &mut [KeyedAccount],
        ctx: &mut InvokeContext<'_>,
    ) -> Result<(), HopperSvmError> {
        // The Config program has a single instruction shape:
        // there's no tag byte. The entire instruction data is
        // the bincode-serialised
        // `ConfigKeys { keys: Vec<(Pubkey, bool)> }` followed by
        // the user data. We parse the keys list, validate each
        // signer-flagged key against the account-meta signers,
        // then write the user data into account 0.
        if accounts.is_empty() {
            return Err(HopperSvmError::AccountIndexOutOfBounds { index: 0, len: 0 });
        }
        let target_addr = accounts[0].address;
        ctx.require_writable(&target_addr)?;

        // Parse: keys_len(u64) + n × (Pubkey + bool) + data.
        if data.len() < 8 {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: "config::Store: body too short".to_string(),
            });
        }
        let keys_len = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
        let keys_total = keys_len * 33; // each (Pubkey, bool) = 32 + 1
        if data.len() < 8 + keys_total {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "config::Store: body has {} bytes, need {} for {keys_len} keys",
                    data.len(),
                    8 + keys_total
                ),
            });
        }
        // Each signer-flagged key must appear as a signer in the
        // instruction's account metas.
        for i in 0..keys_len {
            let off = 8 + i * 33;
            let pk = Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
            let is_signer = data[off + 32] != 0;
            if is_signer {
                ctx.require_signer(&pk)?;
            }
        }
        let user_data = &data[8 + keys_total..];
        // Write the user data into the config account, padding
        // or truncating to fit the account's existing size.
        let target_len = accounts[0].data.len();
        if user_data.len() > target_len {
            return Err(HopperSvmError::BuiltinError {
                program_id: *ctx.program_id,
                message: format!(
                    "config::Store: user data {} bytes > config account size {}",
                    user_data.len(),
                    target_len
                ),
            });
        }
        accounts[0].data[..user_data.len()].copy_from_slice(user_data);
        // Zero the trailing bytes so a previous Store's data
        // doesn't leak through after a smaller write.
        for b in accounts[0].data[user_data.len()..].iter_mut() {
            *b = 0;
        }
        ctx.log(format!(
            "config::Store: {target_addr} ({} keys, {} bytes data)",
            keys_len,
            user_data.len()
        ));
        Ok(())
    }
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
        let pid = CONFIG_PROGRAM_ID;
        let mut ctx = InvokeContext {
            program_id: &pid,
            account_metas: &metas,
            sysvars: &sysvars,
            logs: &mut logs,
            budget: &mut budget,
        };
        ConfigProgramSimulator.invoke(&data, accounts, &mut ctx)
    }

    #[test]
    fn config_store_writes_user_data() {
        let target = Pubkey::new_unique();
        let signer = Pubkey::new_unique();
        let mut accounts = vec![
            KeyedAccount::new(target, 1_000_000, CONFIG_PROGRAM_ID, vec![0u8; 64], false),
            KeyedAccount::new(
                signer,
                1_000_000,
                solana_sdk::system_program::id(),
                vec![],
                false,
            ),
        ];
        let user_data = b"hello config";
        // 1 signer-flagged key, then user data.
        let mut data = vec![];
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(signer.as_ref());
        data.push(1); // is_signer = true
        data.extend_from_slice(user_data);

        invoke(
            data,
            &mut accounts,
            metas(&[(target, false, true), (signer, true, false)]),
        )
        .expect("Store");

        // First 12 bytes of target match user data.
        assert_eq!(&accounts[0].data[..user_data.len()], user_data);
        // Trailing bytes zeroed.
        assert!(accounts[0].data[user_data.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn config_store_rejects_unsigned_signer_key() {
        let target = Pubkey::new_unique();
        let listed_signer = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            target,
            1_000_000,
            CONFIG_PROGRAM_ID,
            vec![0u8; 64],
            false,
        )];
        let mut data = vec![];
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(listed_signer.as_ref());
        data.push(1); // claims signer
        data.extend_from_slice(b"data");
        // Don't include listed_signer in the metas — should fail.
        let err = invoke(data, &mut accounts, metas(&[(target, false, true)])).unwrap_err();
        assert!(matches!(err, HopperSvmError::AccountNotSigner { .. }));
    }

    #[test]
    fn config_store_rejects_oversized_data() {
        let target = Pubkey::new_unique();
        let mut accounts = vec![KeyedAccount::new(
            target,
            1_000_000,
            CONFIG_PROGRAM_ID,
            vec![0u8; 8],
            false,
        )];
        let mut data = vec![];
        data.extend_from_slice(&0u64.to_le_bytes()); // no signer keys
        data.extend_from_slice(&[0u8; 100]); // 100 bytes user data > 8 byte account
        let err = invoke(data, &mut accounts, metas(&[(target, false, true)])).unwrap_err();
        match err {
            HopperSvmError::BuiltinError { message, .. } => {
                assert!(message.contains("user data"), "{message}");
            }
            other => panic!("wrong err: {other:?}"),
        }
    }
}
