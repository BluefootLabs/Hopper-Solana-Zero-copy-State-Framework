//! Composable Validation Pipeline.
//!
//! Three layers of validation composition:
//!
//! 1. **Atomic rules** -- `fn` pointers and closures (combinators).
//!    `ValidationGraph` stores `fn` pointers for static rule sets.
//!    `require_signer_at()` and friends return closures for inline use.
//!
//! 2. **Named groups and bundles** -- `ValidationGroup` bundles related rules under
//!    a label for reuse. `ValidationBundle` composes groups into a single check.
//!    `TransitionRulePack` dispatches rules by instruction tag.
//!
//! 3. **Post-mutation checks** -- `PostMutationValidator` holds checks that run
//!    after account writes. Balance conservation, solvency invariants, authority
//!    immutability -- anything that needs the final state to verify.
//!
//! `AccountConstraint` and `TransactionConstraint` provide builder-pattern
//! validation for single accounts and global instruction-level checks.
//!
//! ```ignore
//! // Named group for reuse across instructions:
//! let mut signer_checks = ValidationGroup::<2>::new("signer_checks");
//! signer_checks.add(validate_authority)?;
//! signer_checks.add(validate_fee_payer)?;
//!
//! // Bundle groups together:
//! let mut bundle = ValidationBundle::<2>::new();
//! bundle.add(&signer_checks)?;
//! bundle.add(&tx_constraint)?;
//! bundle.run(&ctx)?;
//!
//! // Instruction-specific rules:
//! let mut rules = TransitionRulePack::<8>::new();
//! rules.add(0, validate_init)?;
//! rules.add(1, validate_deposit)?;
//! rules.run_for(instruction_tag, &ctx)?;
//!
//! // Post-mutation invariants:
//! let mut post = PostMutationValidator::<2>::new();
//! post.add(check_vault_solvent)?;
//! post.run(accounts, program_id)?;
//! ```

use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult};

// -- Validation Node --

/// A validation function signature.
///
/// Receives the full account slice and instruction data.
/// Returns Ok(()) if validation passes, Err otherwise.
pub type ValidateFn = fn(ctx: &ValidationContext) -> ProgramResult;

/// Context passed to each validation node.
pub struct ValidationContext<'a> {
    /// The program ID.
    pub program_id: &'a Address,
    /// All accounts in the instruction.
    pub accounts: &'a [AccountView],
    /// Instruction data.
    pub data: &'a [u8],
}

impl<'a> ValidationContext<'a> {
    /// Create a new validation context.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView],
        data: &'a [u8],
    ) -> Self {
        Self { program_id, accounts, data }
    }

    /// Get an account by index.
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<&'a AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Require all account addresses to be unique.
    #[inline(always)]
    pub fn require_all_unique_accounts(&self) -> ProgramResult {
        crate::check::guards::require_all_unique(self.accounts)
    }

    /// Require that duplicated addresses are never writable aliases.
    #[inline(always)]
    pub fn require_unique_writable_accounts(&self) -> ProgramResult {
        crate::check::guards::require_unique_writable(self.accounts)
    }

    /// Require that duplicated addresses are never used as signer aliases.
    #[inline(always)]
    pub fn require_unique_signer_accounts(&self) -> ProgramResult {
        crate::check::guards::require_unique_signers(self.accounts)
    }
}

// -- Validation Pipeline (const-generic, stack-only) --

/// A stack-allocated validation graph with up to `N` nodes.
///
/// Nodes execute sequentially. The graph can run in two modes:
/// - **Fail-fast** (`run`): stops at first error
/// - **Accumulate** (`run_all`): runs all nodes, returns first error
pub struct ValidationGraph<const N: usize> {
    nodes: [Option<ValidateFn>; N],
    count: usize,
}

impl<const N: usize> ValidationGraph<N> {
    /// Create an empty validation graph.
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            nodes: [None; N],
            count: 0,
        }
    }

    /// Add a validation node.
    #[inline]
    pub fn add(&mut self, node: ValidateFn) -> Result<(), ProgramError> {
        if self.count >= N {
            return Err(ProgramError::InvalidArgument);
        }
        self.nodes[self.count] = Some(node);
        self.count += 1;
        Ok(())
    }

    /// Number of nodes in the graph.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the graph has no nodes.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Run all validations, fail-fast on first error.
    #[inline]
    pub fn run(&self, ctx: &ValidationContext) -> ProgramResult {
        let mut i = 0;
        while i < self.count {
            if let Some(node) = self.nodes[i] {
                node(ctx)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// Run all validations, accumulate results. Returns the first error found.
    #[inline]
    pub fn run_all(&self, ctx: &ValidationContext) -> ProgramResult {
        let mut first_error: Option<ProgramError> = None;
        let mut i = 0;
        while i < self.count {
            if let Some(node) = self.nodes[i] {
                if let Err(e) = node(ctx) {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
            i += 1;
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

// -- Validation Combinators --

/// Validate that a specific account is a signer.
#[inline(always)]
pub fn require_signer_at(index: usize) -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| {
        let acc = ctx.account(index)?;
        crate::check::check_signer(acc)
    }
}

/// Validate that a specific account is writable.
#[inline(always)]
pub fn require_writable_at(index: usize) -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| {
        let acc = ctx.account(index)?;
        crate::check::check_writable(acc)
    }
}

/// Validate that a specific account is owned by the program.
#[inline(always)]
pub fn require_owned_at(index: usize) -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| {
        let acc = ctx.account(index)?;
        crate::check::check_owner(acc, ctx.program_id)
    }
}

/// Validate minimum instruction data length.
#[inline(always)]
pub fn require_data_min(min: usize) -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| {
        if ctx.data.len() < min {
            Err(ProgramError::InvalidInstructionData)
        } else {
            Ok(())
        }
    }
}

/// Validate two accounts have the same key (e.g., stored address == provided account).
#[inline(always)]
pub fn require_keys_equal(a: usize, b: usize) -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| {
        let acc_a = ctx.account(a)?;
        let acc_b = ctx.account(b)?;
        crate::check::check_keys_eq(acc_a, acc_b)
    }
}

/// Validate two accounts are different (no duplicates).
#[inline(always)]
pub fn require_unique(a: usize, b: usize) -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| {
        let acc_a = ctx.account(a)?;
        let acc_b = ctx.account(b)?;
        crate::check::check_accounts_unique(acc_a, acc_b)
    }
}

/// Validate that all account addresses are unique.
#[inline(always)]
pub fn require_all_unique_accounts() -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| ctx.require_all_unique_accounts()
}

/// Validate that no duplicated account is writable.
#[inline(always)]
pub fn require_unique_writable_accounts() -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| ctx.require_unique_writable_accounts()
}

/// Validate that no duplicated account is used as a signer.
#[inline(always)]
pub fn require_unique_signer_accounts() -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| ctx.require_unique_signer_accounts()
}

/// Validate that an account has at least `min` lamports.
#[inline(always)]
pub fn require_lamports_gte(index: usize, min: u64) -> impl Fn(&ValidationContext) -> ProgramResult {
    move |ctx| {
        let acc = ctx.account(index)?;
        crate::check::check_lamports_gte(acc, min)
    }
}

// -- Constraint Builder --

/// A builder for constructing validation constraints on a single account.
///
/// ```ignore
/// AccountConstraint::on(0)
///     .signer()
///     .writable()
///     .owned_by_program()
///     .validate(&ctx)?;
/// ```
pub struct AccountConstraint {
    index: usize,
    require_signer: bool,
    require_writable: bool,
    require_owned: bool,
    require_executable: bool,
}

impl AccountConstraint {
    /// Start building constraints for an account at the given index.
    #[inline(always)]
    pub const fn on(index: usize) -> Self {
        Self {
            index,
            require_signer: false,
            require_writable: false,
            require_owned: false,
            require_executable: false,
        }
    }

    /// Require the account to be a signer.
    #[inline(always)]
    pub const fn signer(mut self) -> Self {
        self.require_signer = true;
        self
    }

    /// Require the account to be writable.
    #[inline(always)]
    pub const fn writable(mut self) -> Self {
        self.require_writable = true;
        self
    }

    /// Require the account to be owned by the program.
    #[inline(always)]
    pub const fn owned(mut self) -> Self {
        self.require_owned = true;
        self
    }

    /// Require the account to be executable.
    #[inline(always)]
    pub const fn executable(mut self) -> Self {
        self.require_executable = true;
        self
    }

    /// Validate all constraints against the context.
    #[inline]
    pub fn validate(&self, ctx: &ValidationContext) -> ProgramResult {
        let acc = ctx.account(self.index)?;

        if self.require_signer {
            crate::check::check_signer(acc)?;
        }
        if self.require_writable {
            crate::check::check_writable(acc)?;
        }
        if self.require_owned {
            crate::check::check_owner(acc, ctx.program_id)?;
        }
        if self.require_executable {
            crate::check::check_executable(acc)?;
        }

        Ok(())
    }
}

// -- Transaction-Level Validator --

/// Transaction-level constraint that validates global properties.
pub struct TransactionConstraint {
    min_accounts: usize,
    min_data_len: usize,
    require_all_unique: bool,
    require_unique_writable: bool,
    require_unique_signers: bool,
}

impl TransactionConstraint {
    /// Create a new transaction constraint.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            min_accounts: 0,
            min_data_len: 0,
            require_all_unique: false,
            require_unique_writable: false,
            require_unique_signers: false,
        }
    }

    /// Require at least N accounts.
    #[inline(always)]
    pub const fn min_accounts(mut self, n: usize) -> Self {
        self.min_accounts = n;
        self
    }

    /// Require at least N bytes of instruction data.
    #[inline(always)]
    pub const fn min_data(mut self, n: usize) -> Self {
        self.min_data_len = n;
        self
    }

    /// Require that all account addresses are distinct.
    #[inline(always)]
    pub const fn all_unique(mut self) -> Self {
        self.require_all_unique = true;
        self
    }

    /// Require that duplicated addresses are never writable aliases.
    #[inline(always)]
    pub const fn unique_writable(mut self) -> Self {
        self.require_unique_writable = true;
        self
    }

    /// Require that duplicated addresses are never signer aliases.
    #[inline(always)]
    pub const fn unique_signers(mut self) -> Self {
        self.require_unique_signers = true;
        self
    }

    /// Validate against a context.
    #[inline]
    pub fn validate(&self, ctx: &ValidationContext) -> ProgramResult {
        if ctx.accounts.len() < self.min_accounts {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        if ctx.data.len() < self.min_data_len {
            return Err(ProgramError::InvalidInstructionData);
        }
        if self.require_all_unique {
            ctx.require_all_unique_accounts()?;
        }
        if self.require_unique_writable {
            ctx.require_unique_writable_accounts()?;
        }
        if self.require_unique_signers {
            ctx.require_unique_signer_accounts()?;
        }
        Ok(())
    }
}

// -- Named Validation Groups --

/// A named group of validation rules.
///
/// Groups bundle related rules under a label for reuse across instructions.
/// Example: a "token_transfer_preconditions" group that checks signer, writable,
/// owner, and balance across the relevant accounts.
///
/// ```ignore
/// let mut group = ValidationGroup::<4>::new("transfer_preconditions");
/// group.add(require_signer_at(0))?;
/// group.add(require_writable_at(1))?;
/// group.add(require_owned_at(1))?;
/// group.run(&ctx)?;
/// ```
pub struct ValidationGroup<const N: usize> {
    name: &'static str,
    rules: [Option<ValidateFn>; N],
    count: usize,
}

impl<const N: usize> ValidationGroup<N> {
    /// Create a new named validation group.
    #[inline(always)]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            rules: [None; N],
            count: 0,
        }
    }

    /// Group name (for diagnostics/logging).
    #[inline(always)]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Add a rule to the group.
    #[inline]
    pub fn add(&mut self, rule: ValidateFn) -> Result<(), ProgramError> {
        if self.count >= N {
            return Err(ProgramError::InvalidArgument);
        }
        self.rules[self.count] = Some(rule);
        self.count += 1;
        Ok(())
    }

    /// Number of rules in the group.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the group has no rules.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Run all rules in the group. Fail-fast.
    #[inline]
    pub fn run(&self, ctx: &ValidationContext) -> ProgramResult {
        let mut i = 0;
        while i < self.count {
            if let Some(rule) = self.rules[i] {
                rule(ctx)?;
            }
            i += 1;
        }
        Ok(())
    }
}

// -- Validation Bundle --

/// A bundle that composes multiple `ValidationGroup`s into a single check.
///
/// Run all groups in order. If any group fails, the bundle fails.
/// Useful for instruction handlers that share common preconditions
/// but add instruction-specific checks on top.
///
/// ```ignore
/// let mut bundle = ValidationBundle::<3>::new();
/// bundle.add_group(&common_checks)?;
/// bundle.add_group(&transfer_checks)?;
/// bundle.run(&ctx)?;
/// ```
pub struct ValidationBundle<'a, const N: usize> {
    groups: [Option<&'a dyn Validatable>; N],
    count: usize,
}

/// Trait for validation runnables (groups and graphs).
pub trait Validatable {
    /// Run validation against the given context.
    fn validate(&self, ctx: &ValidationContext) -> ProgramResult;
}

impl<const M: usize> Validatable for ValidationGraph<M> {
    #[inline]
    fn validate(&self, ctx: &ValidationContext) -> ProgramResult {
        self.run(ctx)
    }
}

impl<const M: usize> Validatable for ValidationGroup<M> {
    #[inline]
    fn validate(&self, ctx: &ValidationContext) -> ProgramResult {
        self.run(ctx)
    }
}

impl Validatable for TransactionConstraint {
    #[inline]
    fn validate(&self, ctx: &ValidationContext) -> ProgramResult {
        TransactionConstraint::validate(self, ctx)
    }
}

impl<'a, const N: usize> ValidationBundle<'a, N> {
    /// Create an empty bundle.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            groups: [None; N],
            count: 0,
        }
    }

    /// Add a validatable group or graph to the bundle.
    #[inline]
    pub fn add(&mut self, v: &'a dyn Validatable) -> Result<(), ProgramError> {
        if self.count >= N {
            return Err(ProgramError::InvalidArgument);
        }
        self.groups[self.count] = Some(v);
        self.count += 1;
        Ok(())
    }

    /// Run all groups in order. Fail-fast on first error.
    #[inline]
    pub fn run(&self, ctx: &ValidationContext) -> ProgramResult {
        let mut i = 0;
        while i < self.count {
            if let Some(v) = self.groups[i] {
                v.validate(ctx)?;
            }
            i += 1;
        }
        Ok(())
    }
}

// -- Post-Mutation Validator --

/// Signature for a post-mutation check function.
///
/// Receives the account slice after mutation. Can inspect state
/// for invariants that should hold after any write.
pub type PostMutationFn = fn(accounts: &[AccountView], program_id: &Address) -> ProgramResult;

/// Collects post-mutation checks that run after instruction execution.
///
/// Use this to verify invariants that can't be checked upfront:
/// balance conservation, escrow solvency, authority immutability, etc.
///
/// ```ignore
/// let mut post = PostMutationValidator::<4>::new();
/// post.add(check_vault_solvent)?;
/// post.add(check_balance_conserved)?;
///
/// // ... execute mutations ...
///
/// post.run(accounts, program_id)?;
/// ```
pub struct PostMutationValidator<const N: usize> {
    checks: [Option<PostMutationFn>; N],
    count: usize,
}

impl<const N: usize> PostMutationValidator<N> {
    /// Create an empty post-mutation validator.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            checks: [None; N],
            count: 0,
        }
    }

    /// Add a post-mutation check.
    #[inline]
    pub fn add(&mut self, check: PostMutationFn) -> Result<(), ProgramError> {
        if self.count >= N {
            return Err(ProgramError::InvalidArgument);
        }
        self.checks[self.count] = Some(check);
        self.count += 1;
        Ok(())
    }

    /// Number of checks registered.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether no checks are registered.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Run all post-mutation checks. Fail-fast.
    #[inline]
    pub fn run(&self, accounts: &[AccountView], program_id: &Address) -> ProgramResult {
        let mut i = 0;
        while i < self.count {
            if let Some(check) = self.checks[i] {
                check(accounts, program_id)?;
            }
            i += 1;
        }
        Ok(())
    }
}

// -- Transition-Specific Rule Pack --

/// Instruction dispatch tag for associating validation rules with specific instructions.
pub type InstructionTag = u8;

/// A rule pack entry: instruction tag + validation function.
#[derive(Clone, Copy)]
struct TransitionRuleEntry {
    tag: InstructionTag,
    rule: ValidateFn,
}

/// Associates validation rules with specific instruction tags.
///
/// Each instruction in your program can have its own set of checks,
/// looked up by the dispatch tag. This avoids running irrelevant
/// checks for instructions that don't need them.
///
/// ```ignore
/// let mut tr = TransitionRulePack::<16>::new();
/// tr.add(0, validate_init_accounts)?;   // Init
/// tr.add(1, validate_deposit_accounts)?; // Deposit
/// tr.add(2, validate_withdraw_accounts)?; // Withdraw
///
/// // In handler: run only rules for this instruction
/// tr.run_for(instruction_tag, &ctx)?;
/// ```
pub struct TransitionRulePack<const N: usize> {
    entries: [Option<TransitionRuleEntry>; N],
    count: usize,
}

impl<const N: usize> TransitionRulePack<N> {
    /// Create an empty rule pack.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            entries: [None; N],
            count: 0,
        }
    }

    /// Register a rule for a specific instruction tag.
    #[inline]
    pub fn add(&mut self, tag: InstructionTag, rule: ValidateFn) -> Result<(), ProgramError> {
        if self.count >= N {
            return Err(ProgramError::InvalidArgument);
        }
        self.entries[self.count] = Some(TransitionRuleEntry { tag, rule });
        self.count += 1;
        Ok(())
    }

    /// Run all rules matching the given instruction tag. Fail-fast.
    #[inline]
    pub fn run_for(&self, tag: InstructionTag, ctx: &ValidationContext) -> ProgramResult {
        let mut i = 0;
        while i < self.count {
            if let Some(entry) = &self.entries[i] {
                if entry.tag == tag {
                    (entry.rule)(ctx)?;
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// Number of entries.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the rule pack has no entries.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

// -- Default impls --

impl<const N: usize> Default for ValidationGraph<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for TransactionConstraint {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, const N: usize> Default for ValidationBundle<'a, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Default for PostMutationValidator<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Default for TransitionRulePack<N> {
    fn default() -> Self {
        Self::new()
    }
}
