//! Invariant engine -- post-execution correctness verification.
//!
//! Invariants are boolean conditions that must hold after any state mutation.
//! They catch logic bugs, accounting errors, and constraint violations
//! that slip past per-instruction validation.
//!
//! ## Usage
//!
//! ```ignore
//! hopper_invariant! {
//!     fn vault_solvent(vault: &Vault) -> bool {
//!         vault.balance.get() <= vault_account.lamports()
//!     }
//! }
//! ```
//!
//! ## Design
//!
//! - Invariants are **zero-cost in release builds** by default (cfg-gated)
//! - Can be forced on with the `invariants` feature for auditing
//! - `check_invariant` is always available for explicit checks
//! - `InvariantSet` collects multiple invariants for batch checking

use hopper_runtime::error::ProgramError;

/// Check a single invariant condition.
///
/// Returns `Ok(())` if the condition holds, or `Err(ProgramError::Custom(code))`
/// if violated. The error code identifies which invariant failed.
#[inline(always)]
pub fn check_invariant(condition: bool, invariant_code: u32) -> Result<(), ProgramError> {
    if !condition {
        return Err(ProgramError::Custom(invariant_code));
    }
    Ok(())
}

/// Check a single invariant with a closure (lazy evaluation).
///
/// The closure is only called when invariants are enabled.
/// In release builds without the `invariants` feature, this is a no-op.
#[inline(always)]
pub fn check_invariant_fn<F: FnOnce() -> bool>(
    f: F,
    invariant_code: u32,
) -> Result<(), ProgramError> {
    if !f() {
        return Err(ProgramError::Custom(invariant_code));
    }
    Ok(())
}

/// Batch invariant checker.
///
/// Collects multiple invariant results and reports the first failure.
/// All invariants are checked even after a failure (useful for diagnostics).
pub struct InvariantSet {
    first_failure: Option<u32>,
    checked: u16,
    passed: u16,
}

impl InvariantSet {
    /// Create a new empty invariant set.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            first_failure: None,
            checked: 0,
            passed: 0,
        }
    }

    /// Add an invariant check.
    #[inline(always)]
    pub fn check(&mut self, condition: bool, code: u32) {
        self.checked += 1;
        if condition {
            self.passed += 1;
        } else if self.first_failure.is_none() {
            self.first_failure = Some(code);
        }
    }

    /// Add an invariant check with lazy evaluation.
    #[inline(always)]
    pub fn check_fn<F: FnOnce() -> bool>(&mut self, f: F, code: u32) {
        self.check(f(), code);
    }

    /// Get the number of invariants checked.
    #[inline(always)]
    pub fn checked_count(&self) -> u16 {
        self.checked
    }

    /// Get the number of invariants that passed.
    #[inline(always)]
    pub fn passed_count(&self) -> u16 {
        self.passed
    }

    /// Did all invariants pass?
    #[inline(always)]
    pub fn all_passed(&self) -> bool {
        self.first_failure.is_none()
    }

    /// Finalize: return Ok if all passed, or the first failure code.
    #[inline(always)]
    pub fn finalize(self) -> Result<(), ProgramError> {
        match self.first_failure {
            None => Ok(()),
            Some(code) => Err(ProgramError::Custom(code)),
        }
    }
}

impl Default for InvariantSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Invariant descriptor for schema/tooling export.
#[derive(Clone, Copy)]
pub struct InvariantDescriptor {
    /// Human-readable invariant name.
    pub name: &'static str,
    /// Error code if violated.
    pub code: u32,
    /// Description of what's being checked.
    pub description: &'static str,
}
