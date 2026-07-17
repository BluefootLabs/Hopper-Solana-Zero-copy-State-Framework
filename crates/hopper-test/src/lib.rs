//! In-process SVM harness for testing Hopper programs.
//!
//! Wraps [`mollusk_svm::Mollusk`] so example and integration tests can
//! load a compiled Hopper `.so`, seed program-owned accounts with a
//! valid Hopper header, fire instructions, and read back lamports,
//! account data, and compute-unit cost — all without spinning up a
//! validator.
//!
//! The name [`LiteSvmHarness`] reflects the role (a lightweight,
//! in-process SVM) rather than a specific backend crate; the current
//! implementation is backed by Mollusk, which is already vendored in
//! this workspace.
//!
//! ```ignore
//! use hopper_test::LiteSvmHarness;
//! use solana_pubkey::Pubkey;
//!
//! let program_id = Pubkey::new_unique();
//! let mut svm = LiteSvmHarness::load(&program_id, "../../target/deploy/hopper_counter")
//!     .expect("build the SBF artifact first");
//!
//! let user = svm.fund(1_000_000_000);
//! let result = svm.process(&ix, &[(user, account)]);
//! assert!(result.raw().program_result.is_ok());
//! println!("cost: {} CU", result.compute_units());
//! ```

extern crate std;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use mollusk_svm::{result::InstructionResult, Mollusk};
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use solana_svm_log_collector::LogCollector;

pub mod trace;
pub use trace::{AccountDelta, Trace};

/// A lightweight, in-process SVM for exercising a single Hopper program.
pub struct LiteSvmHarness {
    program_id: Pubkey,
    mollusk: Mollusk,
    /// Present after [`Self::capture_logs`]: the collector Mollusk feeds
    /// with the execution's log stream (the same `Program ... invoke` /
    /// `Program log:` / `Program data:` lines an RPC `getTransaction`
    /// would return in `logMessages`).
    logs: Option<Rc<RefCell<LogCollector>>>,
}

impl LiteSvmHarness {
    /// Load a compiled Hopper program by ELF path *stem* (the path
    /// Mollusk expects, e.g. `../../target/deploy/hopper_counter`,
    /// without the `.so` suffix). Returns `None` when the `.so` is
    /// missing so callers can skip the test in environments where the
    /// SBF artifact has not been built yet.
    pub fn load(program_id: &Pubkey, elf_path_stem: &str) -> Option<Self> {
        let so = format!("{elf_path_stem}.so");
        if !Path::new(&so).exists() {
            return None;
        }
        Some(Self {
            program_id: *program_id,
            mollusk: Mollusk::new(program_id, elf_path_stem),
            logs: None,
        })
    }

    /// The program id this harness was loaded under.
    pub fn program_id(&self) -> Pubkey {
        self.program_id
    }

    /// Mutable access to the underlying Mollusk instance for advanced
    /// setup (sysvars, warping the clock, adding extra programs).
    pub fn mollusk_mut(&mut self) -> &mut Mollusk {
        &mut self.mollusk
    }

    /// Start (or restart) log capture: installs a **fresh** collector, so
    /// the next [`Self::process`] call's log stream is readable via
    /// [`Self::logs`] without lines from earlier executions. Call before
    /// each instruction whose logs the test asserts on.
    pub fn capture_logs(&mut self) {
        let collector = LogCollector::new_ref();
        self.mollusk.logger = Some(Rc::clone(&collector));
        self.logs = Some(collector);
    }

    /// Snapshot of the captured log lines (empty when
    /// [`Self::capture_logs`] was never called). Lines look exactly like
    /// an RPC transaction's `logMessages`: `Program <id> invoke [1]`,
    /// `Program log: ...`, `Program data: <base64>`, `Program <id>
    /// success` / `failed ...` — so log-stream decoders (touch maps,
    /// events) can be exercised against real execution output.
    pub fn logs(&self) -> Vec<String> {
        self.logs
            .as_ref()
            .map(|collector| collector.borrow().get_recorded_content().to_vec())
            .unwrap_or_default()
    }

    /// Create a system-owned, lamport-funded account (a fee payer or
    /// signer). Returns a fresh address; the account itself is supplied
    /// to [`Self::process`] in the seed list.
    pub fn funded_account(lamports: u64) -> (Pubkey, Account) {
        let addr = Pubkey::new_unique();
        (addr, Account::new(lamports, 0, &Pubkey::default()))
    }

    /// Build a zeroed, program-owned account of `space` bytes. The
    /// caller is responsible for writing the Hopper header + fields via
    /// [`hopper`-runtime helpers](crate) before processing, so this
    /// crate stays decoupled from any specific layout type.
    pub fn program_account(&self, space: usize, lamports: u64) -> Account {
        Account::new(lamports, space, &self.program_id)
    }

    /// The System Program's keyed account, as an instruction fixture —
    /// required whenever the tested instruction CPIs into the System
    /// Program (init lifecycles, transfers).
    pub fn system_program_account() -> (Pubkey, Account) {
        mollusk_svm::program::keyed_account_for_system_program()
    }

    /// The loaded program's own account, as an instruction fixture.
    ///
    /// Any instruction that CPIs into the executing program must carry
    /// the program's account in its account list (a self-CPI target has
    /// to be a transaction account) — Hopper's `event_cpi` contexts and
    /// Anchor's `#[event_cpi]` both append it for exactly this reason.
    /// The fixture matches [`Self::load`]'s registration (Mollusk's
    /// default upgradeable-loader entry), so the runtime resolves the
    /// inner invoke against the already-loaded ELF.
    pub fn own_program_account(&self) -> (Pubkey, Account) {
        (
            self.program_id,
            mollusk_svm::program::create_program_account_loader_v3(&self.program_id),
        )
    }

    /// Register an additional compiled program for CPI and return the
    /// executable account fixture callers should include in instruction
    /// account seeds. Returns `false` when the ELF is absent.
    pub fn add_program(&mut self, program_id: &Pubkey, elf_path_stem: &str) -> bool {
        let so = format!("{elf_path_stem}.so");
        if !Path::new(&so).exists() {
            return false;
        }
        self.mollusk.add_program(program_id, elf_path_stem);
        true
    }

    /// Upgradeable-loader account for a program registered with Mollusk.
    pub fn executable_program_account(program_id: &Pubkey) -> (Pubkey, Account) {
        (
            *program_id,
            mollusk_svm::program::create_program_account_loader_v3(program_id),
        )
    }

    /// Process a single instruction against the given account seeds.
    pub fn process(
        &self,
        instruction: &Instruction,
        accounts: &[(Pubkey, Account)],
    ) -> HarnessResult {
        HarnessResult(self.mollusk.process_instruction(instruction, accounts))
    }
}

/// Thin wrapper over Mollusk's [`InstructionResult`] with the accessors
/// Hopper tests reach for most: success/failure, CU cost, and the
/// resulting account set.
pub struct HarnessResult(InstructionResult);

impl HarnessResult {
    /// Whether the program returned `Ok`.
    pub fn succeeded(&self) -> bool {
        self.0.program_result.is_ok()
    }

    /// Compute units consumed by the instruction.
    pub fn compute_units(&self) -> u64 {
        self.0.compute_units_consumed
    }

    /// The post-execution account at seed index `i`.
    pub fn account(&self, i: usize) -> &Account {
        &self.0.resulting_accounts[i].1
    }

    /// Lamport balance of the post-execution account at seed index `i`.
    pub fn lamports(&self, i: usize) -> u64 {
        self.0.resulting_accounts[i].1.lamports
    }

    /// The raw Mollusk result for assertions this wrapper does not cover.
    pub fn raw(&self) -> &InstructionResult {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_none_for_missing_artifact() {
        let program_id = Pubkey::new_unique();
        // A path that cannot exist in the sandbox: load must decline so a
        // caller can skip the test rather than panic on a missing `.so`.
        assert!(LiteSvmHarness::load(&program_id, "/nonexistent/hopper_missing").is_none());
    }

    #[test]
    fn funded_account_is_system_owned_and_carries_lamports() {
        let (addr, account) = LiteSvmHarness::funded_account(1_234_567);
        assert_ne!(addr, Pubkey::default(), "fresh address should be unique");
        assert_eq!(account.lamports, 1_234_567);
        assert_eq!(
            account.owner,
            Pubkey::default(),
            "fee payer is system-owned"
        );
        assert!(account.data.is_empty(), "fee payer carries no data");
    }
}
