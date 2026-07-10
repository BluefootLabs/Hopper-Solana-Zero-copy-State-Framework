//! JSON execution traces for instructions run under [`LiteSvmHarness`].
//!
//! A trace turns a single `process` into a reviewable, snapshot-friendly
//! summary of what an instruction did: compute units, return data, and a
//! per-account before/after diff including SPL Token / Token-2022 balance
//! deltas.
//!
//! ```ignore
//! let pre = vec![(payer, payer_acct), (vault, vault_acct)];
//! let result = svm.process(&ix, &pre);
//! let trace = result.trace(svm.program_id(), &pre);
//! println!("{}", trace.to_json());
//! ```
//!
//! CPI-frame capture (the nested `Program X invoke [n]` tree, per-frame CU) is
//! a planned addition: mollusk 0.10's `InstructionResult` does not surface
//! program logs, so it requires wiring a separate log collector. Everything
//! here is derived from data the result already carries, so it is exact.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use solana_account::Account;
use solana_pubkey::Pubkey;

use crate::HarnessResult;

/// SPL Token program id.
const TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
/// SPL Token-2022 program id.
const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Decode an SPL token account's `amount` (a `u64` LE at byte offset 64) when
/// `account` is owned by a token program and long enough to carry the field.
fn token_amount(account: &Account) -> Option<u64> {
    let is_token = account.owner == TOKEN_PROGRAM_ID || account.owner == TOKEN_2022_PROGRAM_ID;
    if !is_token || account.data.len() < 72 {
        return None;
    }
    let mut amount = [0u8; 8];
    amount.copy_from_slice(&account.data[64..72]);
    Some(u64::from_le_bytes(amount))
}

/// One account's before/after summary within a [`Trace`].
#[derive(Clone, Debug)]
pub struct AccountDelta {
    /// Account address.
    pub address: Pubkey,
    /// Lamports before / after.
    pub pre_lamports: u64,
    pub post_lamports: u64,
    /// Data length before / after.
    pub pre_data_len: usize,
    pub post_data_len: usize,
    /// Owner before / after.
    pub pre_owner: Pubkey,
    pub post_owner: Pubkey,
    /// Decoded SPL token amount before / after, when the account is a token
    /// account on that side.
    pub token_pre: Option<u64>,
    pub token_post: Option<u64>,
}

impl AccountDelta {
    /// Net lamport change (`post - pre`).
    pub fn lamport_delta(&self) -> i128 {
        i128::from(self.post_lamports) - i128::from(self.pre_lamports)
    }

    /// Net token-amount change, when both sides decoded as a token account.
    pub fn token_delta(&self) -> Option<i128> {
        match (self.token_pre, self.token_post) {
            (Some(before), Some(after)) => Some(i128::from(after) - i128::from(before)),
            _ => None,
        }
    }
}

/// A trace-grade summary of one processed instruction.
#[derive(Clone, Debug)]
pub struct Trace {
    /// The program that ran.
    pub program_id: Pubkey,
    /// Whether the program returned `Ok`.
    pub success: bool,
    /// The error, when the program failed.
    pub error: Option<String>,
    /// Compute units consumed.
    pub compute_units: u64,
    /// Raw return data the program set.
    pub return_data: Vec<u8>,
    /// Per-account before/after diffs, in the order the instruction was given.
    pub accounts: Vec<AccountDelta>,
}

impl Trace {
    /// Build a trace from the pre- and post-execution account snapshots.
    ///
    /// Pure, so it is unit-testable without an SBF artifact. `pre` and `post`
    /// are correlated by index (the order `process` was given the accounts).
    pub fn build(
        program_id: Pubkey,
        success: bool,
        error: Option<String>,
        compute_units: u64,
        return_data: Vec<u8>,
        pre: &[(Pubkey, Account)],
        post: &[(Pubkey, Account)],
    ) -> Self {
        let mut accounts = Vec::with_capacity(post.len());
        for (i, (address, post_acct)) in post.iter().enumerate() {
            let pre_acct = pre.get(i).map(|(_, a)| a);
            accounts.push(AccountDelta {
                address: *address,
                pre_lamports: pre_acct.map(|a| a.lamports).unwrap_or(0),
                post_lamports: post_acct.lamports,
                pre_data_len: pre_acct.map(|a| a.data.len()).unwrap_or(0),
                post_data_len: post_acct.data.len(),
                pre_owner: pre_acct.map(|a| a.owner).unwrap_or_default(),
                post_owner: post_acct.owner,
                token_pre: pre_acct.and_then(token_amount),
                token_post: token_amount(post_acct),
            });
        }
        Trace {
            program_id,
            success,
            error,
            compute_units,
            return_data,
            accounts,
        }
    }

    /// Render the trace as pretty-printed JSON.
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "{{");
        let _ = writeln!(s, "  \"programId\": \"{}\",", self.program_id);
        let _ = writeln!(s, "  \"success\": {},", self.success);
        match &self.error {
            Some(err) => {
                let _ = writeln!(s, "  \"error\": {},", json_string(err));
            }
            None => {
                let _ = writeln!(s, "  \"error\": null,");
            }
        }
        let _ = writeln!(s, "  \"computeUnits\": {},", self.compute_units);
        let _ = writeln!(s, "  \"returnData\": \"{}\",", hex(&self.return_data));
        let _ = writeln!(s, "  \"accounts\": [");
        for (i, a) in self.accounts.iter().enumerate() {
            let comma = if i + 1 < self.accounts.len() { "," } else { "" };
            let _ = writeln!(s, "    {{");
            let _ = writeln!(s, "      \"address\": \"{}\",", a.address);
            let _ = writeln!(
                s,
                "      \"lamports\": {{ \"pre\": {}, \"post\": {}, \"delta\": {} }},",
                a.pre_lamports,
                a.post_lamports,
                a.lamport_delta()
            );
            let _ = writeln!(
                s,
                "      \"dataLen\": {{ \"pre\": {}, \"post\": {} }},",
                a.pre_data_len, a.post_data_len
            );
            match a.token_delta() {
                Some(delta) => {
                    let _ = writeln!(
                        s,
                        "      \"ownerChanged\": {},",
                        a.pre_owner != a.post_owner
                    );
                    let _ = writeln!(
                        s,
                        "      \"token\": {{ \"pre\": {}, \"post\": {}, \"delta\": {} }}",
                        a.token_pre.unwrap_or(0),
                        a.token_post.unwrap_or(0),
                        delta
                    );
                }
                None => {
                    let _ = writeln!(s, "      \"ownerChanged\": {}", a.pre_owner != a.post_owner);
                }
            }
            let _ = writeln!(s, "    }}{}", comma);
        }
        let _ = writeln!(s, "  ]");
        let _ = write!(s, "}}");
        s
    }
}

impl HarnessResult {
    /// Produce a [`Trace`] of this result against the `pre` account snapshot
    /// that was passed to `process` (so before/after diffs and token deltas can
    /// be computed). The post snapshot is the result's `resulting_accounts`.
    pub fn trace(&self, program_id: Pubkey, pre: &[(Pubkey, Account)]) -> Trace {
        let raw = self.raw();
        let success = raw.program_result.is_ok();
        let error = if success {
            None
        } else {
            Some(format!("{:?}", raw.program_result))
        };
        Trace::build(
            program_id,
            success,
            error,
            raw.compute_units_consumed,
            raw.return_data.clone(),
            pre,
            &raw.resulting_accounts,
        )
    }
}

/// Lower-hex encode bytes.
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Quote and escape a string as a JSON string literal.
fn json_string(text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 2);
    s.push('"');
    for c in text.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            other => s.push(other),
        }
    }
    s.push('"');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_captures_lamport_and_token_deltas_as_json() {
        let program = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let token_account = Pubkey::new_unique();

        // pre: payer with 1_000_000 lamports; a token account holding 5.
        let mut pre_token = Account::new(2_000_000, 165, &TOKEN_PROGRAM_ID);
        pre_token.data[64..72].copy_from_slice(&5u64.to_le_bytes());
        let pre = alloc::vec![
            (payer, Account::new(1_000_000, 0, &Pubkey::default())),
            (token_account, pre_token.clone()),
        ];

        // post: payer paid 1_000 lamports; the token account now holds 12.
        let mut post_token = pre_token.clone();
        post_token.data[64..72].copy_from_slice(&12u64.to_le_bytes());
        let post = alloc::vec![
            (payer, Account::new(999_000, 0, &Pubkey::default())),
            (token_account, post_token),
        ];

        let trace = Trace::build(
            program,
            true,
            None,
            4200,
            alloc::vec![1u8, 2, 3],
            &pre,
            &post,
        );

        // Structured assertions on the model.
        assert!(trace.success);
        assert_eq!(trace.accounts[0].lamport_delta(), -1000);
        assert_eq!(trace.accounts[1].token_delta(), Some(7));

        // JSON output carries the headline fields.
        let json = trace.to_json();
        assert!(json.contains("\"computeUnits\": 4200"));
        assert!(json.contains("\"success\": true"));
        assert!(json.contains("\"error\": null"));
        assert!(json.contains("\"returnData\": \"010203\""));
        assert!(json.contains("\"delta\": -1000"));
        assert!(json.contains("\"token\": { \"pre\": 5, \"post\": 12, \"delta\": 7 }"));
    }

    #[test]
    fn failed_instruction_records_error_and_no_token_for_plain_accounts() {
        let program = Pubkey::new_unique();
        let acct = Pubkey::new_unique();
        let pre = alloc::vec![(acct, Account::new(10, 8, &program))];
        let post = alloc::vec![(acct, Account::new(10, 8, &program))];

        let trace = Trace::build(
            program,
            false,
            Some(alloc::string::String::from("Custom(7)")),
            900,
            Vec::new(),
            &pre,
            &post,
        );

        assert!(!trace.success);
        assert_eq!(trace.accounts[0].token_delta(), None);

        let json = trace.to_json();
        assert!(json.contains("\"success\": false"));
        assert!(json.contains("\"error\": \"Custom(7)\""));
        assert!(json.contains("\"returnData\": \"\""));
        assert!(!json.contains("\"token\":"));
    }
}
