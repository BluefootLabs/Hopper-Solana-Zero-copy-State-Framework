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

use std::path::Path;

use mollusk_svm::{result::InstructionResult, Mollusk};
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

/// A lightweight, in-process SVM for exercising a single Hopper program.
pub struct LiteSvmHarness {
    program_id: Pubkey,
    mollusk: Mollusk,
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
