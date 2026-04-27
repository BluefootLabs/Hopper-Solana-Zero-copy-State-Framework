//! Policy-Aware Capabilities -- tie instruction behavior to validation requirements.
//!
//! A `Capability` declares what an instruction intends to do (mutate treasury,
//! touch journal, call external programs, etc.). An `InstructionPolicy` binds
//! capabilities to the validation rules they require.
//!
//! ## How It Works
//!
//! 1. Declare which capabilities your instruction needs:
//!    ```ignore
//!    const DEPOSIT_CAPS: CapabilitySet = CapabilitySet::new()
//!        .with(Capability::MutatesState)
//!        .with(Capability::TouchesJournal);
//!    ```
//!
//! 2. Define the policy that maps capabilities → validation requirements:
//!    ```ignore
//!    const POLICY: InstructionPolicy<4> = InstructionPolicy::new()
//!        .when(Capability::MutatesState, PolicyRequirement::Authority)
//!        .when(Capability::TouchesJournal, PolicyRequirement::JournalCapacity)
//!        .when(Capability::ExternalCall, PolicyRequirement::PostMutationCheck);
//!    ```
//!
//! 3. At runtime, enforce the policy against the instruction's declared caps:
//!    ```ignore
//!    policy.enforce(&DEPOSIT_CAPS, &ctx)?;
//!    ```
//!
//! This makes Hopper **smart** -- capabilities automatically trigger the
//! correct set of validation guards.

/// Instruction capability flags.
///
/// Each capability is a single bit in a u32 bitmask.
/// Programs declare which capabilities an instruction requires.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Capability {
    /// Instruction reads owned account data.
    ReadsState = 0,
    /// Instruction mutates owned account data.
    MutatesState = 1,
    /// Instruction writes to a journal segment.
    TouchesJournal = 2,
    /// Instruction calls another program via CPI.
    ExternalCall = 3,
    /// Instruction modifies treasury/vault balances.
    MutatesTreasury = 4,
    /// Instruction performs account reallocation.
    ReallocatesAccount = 5,
    /// Instruction creates a new account.
    CreatesAccount = 6,
    /// Instruction closes an account.
    ClosesAccount = 7,
    /// Instruction modifies permissions/authority.
    ModifiesAuthority = 8,
    /// Instruction triggers a state machine transition.
    TransitionsState = 9,
}

impl Capability {
    /// Convert to bitmask.
    #[inline(always)]
    pub const fn mask(self) -> u32 {
        1u32 << (self as u8)
    }
}

/// A set of capabilities declared for an instruction.
///
/// Const-constructible bitmask.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CapabilitySet {
    bits: u32,
}

impl CapabilitySet {
    /// Empty capability set (read-only instruction).
    #[inline(always)]
    pub const fn new() -> Self {
        Self { bits: 0 }
    }

    /// Add a capability to the set.
    #[inline(always)]
    pub const fn with(self, cap: Capability) -> Self {
        Self {
            bits: self.bits | cap.mask(),
        }
    }

    /// Check if a capability is present.
    #[inline(always)]
    pub const fn has(&self, cap: Capability) -> bool {
        self.bits & cap.mask() != 0
    }

    /// Raw bitmask value.
    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.bits
    }

    /// Number of capabilities in the set.
    #[inline(always)]
    pub const fn count(&self) -> u32 {
        self.bits.count_ones()
    }

    /// Union of two capability sets.
    #[inline(always)]
    pub const fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    /// Whether this set is a subset of another.
    #[inline(always)]
    pub const fn is_subset_of(&self, other: &Self) -> bool {
        (self.bits & other.bits) == self.bits
    }
}

/// What validation is required when a capability is active.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PolicyRequirement {
    /// Must have a signer authority account.
    Authority = 0,
    /// Must verify journal segment has capacity.
    JournalCapacity = 1,
    /// Must run post-mutation validation bundle.
    PostMutationCheck = 2,
    /// Must pass CPI guard (assert_no_cpi or explicit allow).
    CpiGuard = 3,
    /// Must verify rent exemption after resize.
    RentExemption = 4,
    /// Must run invariant set after execution.
    InvariantCheck = 5,
    /// Must snapshot state before mutation (for receipts/rollback).
    StateSnapshot = 6,
    /// Must verify lamport conservation.
    LamportConservation = 7,
}

impl PolicyRequirement {
    /// Convert to bitmask.
    #[inline(always)]
    pub const fn mask(self) -> u32 {
        1u32 << (self as u8)
    }
}

/// A set of active policy requirements.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RequirementSet {
    bits: u32,
}

impl RequirementSet {
    /// Empty.
    #[inline(always)]
    pub const fn new() -> Self {
        Self { bits: 0 }
    }

    /// Add a requirement.
    #[inline(always)]
    pub const fn with(self, req: PolicyRequirement) -> Self {
        Self {
            bits: self.bits | req.mask(),
        }
    }

    /// Check if a requirement is active.
    #[inline(always)]
    pub const fn has(&self, req: PolicyRequirement) -> bool {
        self.bits & req.mask() != 0
    }

    /// Raw bitmask.
    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.bits
    }
}

/// A policy rule: when capability C is active, requirement R must be met.
#[derive(Clone, Copy)]
pub struct PolicyRule {
    pub capability: Capability,
    pub requirement: PolicyRequirement,
}

/// Instruction policy -- maps capabilities to validation requirements.
///
/// Const-constructible, stack-allocated. At most N rules.
pub struct InstructionPolicy<const N: usize> {
    rules: [PolicyRule; N],
    count: usize,
}

impl<const N: usize> InstructionPolicy<N> {
    /// Create an empty policy.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            rules: [PolicyRule {
                capability: Capability::ReadsState,
                requirement: PolicyRequirement::Authority,
            }; N],
            count: 0,
        }
    }

    /// Add a policy rule: when `cap` is declared, `req` must be satisfied.
    #[inline(always)]
    pub const fn when(mut self, cap: Capability, req: PolicyRequirement) -> Self {
        assert!(self.count < N, "policy rule overflow");
        self.rules[self.count] = PolicyRule {
            capability: cap,
            requirement: req,
        };
        self.count += 1;
        self
    }

    /// Resolve which requirements are needed for a given capability set.
    ///
    /// Returns the union of all requirements triggered by the declared capabilities.
    #[inline]
    pub const fn resolve(&self, caps: &CapabilitySet) -> RequirementSet {
        let mut reqs = RequirementSet::new();
        let mut i = 0;
        while i < self.count {
            if caps.has(self.rules[i].capability) {
                reqs = reqs.with(self.rules[i].requirement);
            }
            i += 1;
        }
        reqs
    }

    /// Number of rules in this policy.
    #[inline(always)]
    pub const fn rule_count(&self) -> usize {
        self.count
    }
}

impl Default for CapabilitySet {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for RequirementSet {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Default for InstructionPolicy<N> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Named Policy Packs
// ---------------------------------------------------------------------------
//
// Pre-built policies for common instruction patterns. Use these directly or
// as starting points. Each pack encodes the capabilities and validation
// requirements that experienced Solana developers would wire by hand.

/// Capabilities for an instruction that writes to treasury/vault balances.
///
/// Triggers: authority check + state snapshot + lamport conservation + invariants.
pub const TREASURY_WRITE_POLICY: InstructionPolicy<4> = InstructionPolicy::new()
    .when(Capability::MutatesState, PolicyRequirement::Authority)
    .when(Capability::MutatesState, PolicyRequirement::StateSnapshot)
    .when(Capability::MutatesTreasury, PolicyRequirement::LamportConservation)
    .when(Capability::MutatesTreasury, PolicyRequirement::InvariantCheck);

/// Capabilities for a treasury write instruction.
pub const TREASURY_WRITE_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::MutatesState)
    .with(Capability::MutatesTreasury);

/// Capabilities for an instruction that appends to a journal segment.
///
/// Triggers: authority check + journal capacity guard + state snapshot.
pub const JOURNAL_TOUCH_POLICY: InstructionPolicy<3> = InstructionPolicy::new()
    .when(Capability::MutatesState, PolicyRequirement::Authority)
    .when(Capability::TouchesJournal, PolicyRequirement::JournalCapacity)
    .when(Capability::TouchesJournal, PolicyRequirement::StateSnapshot);

/// Capabilities for a journal-writing instruction.
pub const JOURNAL_TOUCH_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::MutatesState)
    .with(Capability::TouchesJournal);

/// Capabilities for an instruction that makes external calls via CPI.
///
/// Triggers: CPI guard + post-mutation check + state snapshot.
pub const EXTERNAL_CALL_POLICY: InstructionPolicy<3> = InstructionPolicy::new()
    .when(Capability::ExternalCall, PolicyRequirement::CpiGuard)
    .when(Capability::ExternalCall, PolicyRequirement::PostMutationCheck)
    .when(Capability::ExternalCall, PolicyRequirement::StateSnapshot);

/// Capabilities for a CPI-invoking instruction.
pub const EXTERNAL_CALL_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::ExternalCall);

/// Capabilities for an instruction that modifies shard data in a sharded account.
///
/// Triggers: authority + state snapshot + invariants.
pub const SHARD_MUTATION_POLICY: InstructionPolicy<3> = InstructionPolicy::new()
    .when(Capability::MutatesState, PolicyRequirement::Authority)
    .when(Capability::MutatesState, PolicyRequirement::StateSnapshot)
    .when(Capability::MutatesState, PolicyRequirement::InvariantCheck);

/// Capabilities for a shard-modifying instruction.
pub const SHARD_MUTATION_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::MutatesState);

/// Capabilities for an instruction that reallocates an account (migration-sensitive).
///
/// Triggers: authority + rent exemption + state snapshot + invariants.
pub const MIGRATION_SENSITIVE_POLICY: InstructionPolicy<4> = InstructionPolicy::new()
    .when(Capability::ReallocatesAccount, PolicyRequirement::Authority)
    .when(Capability::ReallocatesAccount, PolicyRequirement::RentExemption)
    .when(Capability::ReallocatesAccount, PolicyRequirement::StateSnapshot)
    .when(Capability::ReallocatesAccount, PolicyRequirement::InvariantCheck);

/// Capabilities for a migration/realloc instruction.
pub const MIGRATION_SENSITIVE_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::MutatesState)
    .with(Capability::ReallocatesAccount);

/// Capabilities for an instruction that modifies authority/permissions.
///
/// Triggers: authority + CPI guard + post-mutation check + invariants.
pub const AUTHORITY_CHANGE_POLICY: InstructionPolicy<4> = InstructionPolicy::new()
    .when(Capability::ModifiesAuthority, PolicyRequirement::Authority)
    .when(Capability::ModifiesAuthority, PolicyRequirement::CpiGuard)
    .when(Capability::ModifiesAuthority, PolicyRequirement::PostMutationCheck)
    .when(Capability::ModifiesAuthority, PolicyRequirement::InvariantCheck);

/// Capabilities for an authority change instruction.
pub const AUTHORITY_CHANGE_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::MutatesState)
    .with(Capability::ModifiesAuthority);

/// Capabilities for a read-only audit/inspection instruction.
///
/// Triggers: state snapshot only. No mutating capabilities.
pub const READ_ONLY_AUDIT_POLICY: InstructionPolicy<1> = InstructionPolicy::new()
    .when(Capability::ReadsState, PolicyRequirement::StateSnapshot);

/// Capabilities for a read-only audit instruction.
pub const READ_ONLY_AUDIT_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::ReadsState);

/// Capabilities for an account initialization instruction.
///
/// Triggers: authority + rent exemption + invariants.
pub const ACCOUNT_INIT_POLICY: InstructionPolicy<3> = InstructionPolicy::new()
    .when(Capability::CreatesAccount, PolicyRequirement::Authority)
    .when(Capability::CreatesAccount, PolicyRequirement::RentExemption)
    .when(Capability::CreatesAccount, PolicyRequirement::InvariantCheck);

/// Capabilities for an account init instruction.
pub const ACCOUNT_INIT_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::CreatesAccount);

/// Capabilities for an account close instruction.
///
/// Triggers: authority + state snapshot + lamport conservation.
pub const ACCOUNT_CLOSE_POLICY: InstructionPolicy<3> = InstructionPolicy::new()
    .when(Capability::ClosesAccount, PolicyRequirement::Authority)
    .when(Capability::ClosesAccount, PolicyRequirement::StateSnapshot)
    .when(Capability::ClosesAccount, PolicyRequirement::LamportConservation);

/// Capabilities for an account close instruction.
pub const ACCOUNT_CLOSE_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::ClosesAccount);

// ---------------------------------------------------------------------------
// Policy Pack Registry (for schema/manifest export)
// ---------------------------------------------------------------------------

/// Descriptor for a named policy pack with full metadata.
///
/// Used by schema export, CLI tooling, and Manager to describe each
/// pre-built policy pack with its capabilities, validation requirements,
/// receipt expectations, and invariant hints.
#[derive(Clone, Copy)]
pub struct PolicyPackDescriptor {
    /// Short name (e.g. "TreasuryWrite").
    pub name: &'static str,
    /// Human-readable description of when to use this pack.
    pub description: &'static str,
    /// Capability set this pack covers.
    pub capabilities: &'static CapabilitySet,
    /// The policy (rule table) this pack enforces.
    pub requirements: &'static [(&'static str, &'static str)],
    /// Whether instructions using this pack should emit a receipt.
    pub receipt_expected: bool,
    /// Invariant hints (e.g. "balance_conservation", "authority_present").
    pub invariant_hints: &'static [&'static str],
}

/// All named policy packs with full descriptors, in order.
pub const NAMED_POLICY_PACKS: &[PolicyPackDescriptor] = &[
    PolicyPackDescriptor {
        name: "TreasuryWrite",
        description: "Vault/treasury balance mutations. Enforces authority, snapshot, lamport conservation, and invariant checks.",
        capabilities: &TREASURY_WRITE_CAPS,
        requirements: &[
            ("MutatesState", "Authority, StateSnapshot"),
            ("MutatesTreasury", "LamportConservation, InvariantCheck"),
        ],
        receipt_expected: true,
        invariant_hints: &["balance_conservation", "authority_present"],
    },
    PolicyPackDescriptor {
        name: "JournalTouch",
        description: "Journal segment writes. Enforces authority, capacity guard, and snapshot.",
        capabilities: &JOURNAL_TOUCH_CAPS,
        requirements: &[
            ("MutatesState", "Authority"),
            ("TouchesJournal", "JournalCapacity, StateSnapshot"),
        ],
        receipt_expected: true,
        invariant_hints: &["journal_not_full"],
    },
    PolicyPackDescriptor {
        name: "ExternalCall",
        description: "CPI-invoking instructions. Enforces CPI guard, post-mutation check, and snapshot.",
        capabilities: &EXTERNAL_CALL_CAPS,
        requirements: &[
            ("ExternalCall", "CpiGuard, PostMutationCheck, StateSnapshot"),
        ],
        receipt_expected: true,
        invariant_hints: &["cpi_allowlisted"],
    },
    PolicyPackDescriptor {
        name: "ShardMutation",
        description: "Shard data modifications. Enforces authority, snapshot, and invariant checks.",
        capabilities: &SHARD_MUTATION_CAPS,
        requirements: &[
            ("MutatesState", "Authority, StateSnapshot, InvariantCheck"),
        ],
        receipt_expected: true,
        invariant_hints: &[],
    },
    PolicyPackDescriptor {
        name: "MigrationSensitive",
        description: "Account reallocation/migration. Enforces authority, rent exemption, snapshot, and invariant checks.",
        capabilities: &MIGRATION_SENSITIVE_CAPS,
        requirements: &[
            ("ReallocatesAccount", "Authority, RentExemption, StateSnapshot, InvariantCheck"),
        ],
        receipt_expected: true,
        invariant_hints: &["rent_exempt_after_realloc"],
    },
    PolicyPackDescriptor {
        name: "AuthorityChange",
        description: "Authority/permission modifications. Enforces authority, CPI guard, post-mutation, and invariant checks.",
        capabilities: &AUTHORITY_CHANGE_CAPS,
        requirements: &[
            ("ModifiesAuthority", "Authority, CpiGuard, PostMutationCheck, InvariantCheck"),
        ],
        receipt_expected: true,
        invariant_hints: &["new_authority_valid"],
    },
    PolicyPackDescriptor {
        name: "ReadOnlyAudit",
        description: "Read-only inspection/audit. Only requires snapshot for traceability.",
        capabilities: &READ_ONLY_AUDIT_CAPS,
        requirements: &[
            ("ReadsState", "StateSnapshot"),
        ],
        receipt_expected: false,
        invariant_hints: &[],
    },
    PolicyPackDescriptor {
        name: "AccountInit",
        description: "Account creation. Enforces authority, rent exemption, and invariant checks.",
        capabilities: &ACCOUNT_INIT_CAPS,
        requirements: &[
            ("CreatesAccount", "Authority, RentExemption, InvariantCheck"),
        ],
        receipt_expected: true,
        invariant_hints: &["header_initialized"],
    },
    PolicyPackDescriptor {
        name: "AccountClose",
        description: "Account closure. Enforces authority, snapshot, and lamport conservation.",
        capabilities: &ACCOUNT_CLOSE_CAPS,
        requirements: &[
            ("ClosesAccount", "Authority, StateSnapshot, LamportConservation"),
        ],
        receipt_expected: true,
        invariant_hints: &["sentinel_written"],
    },
];

/// Capability name lookup.
impl Capability {
    /// Human-readable name for this capability.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReadsState => "ReadsState",
            Self::MutatesState => "MutatesState",
            Self::TouchesJournal => "TouchesJournal",
            Self::ExternalCall => "ExternalCall",
            Self::MutatesTreasury => "MutatesTreasury",
            Self::ReallocatesAccount => "ReallocatesAccount",
            Self::CreatesAccount => "CreatesAccount",
            Self::ClosesAccount => "ClosesAccount",
            Self::ModifiesAuthority => "ModifiesAuthority",
            Self::TransitionsState => "TransitionsState",
        }
    }
}

/// Requirement name lookup.
impl PolicyRequirement {
    /// Human-readable name for this requirement.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Authority => "Authority",
            Self::JournalCapacity => "JournalCapacity",
            Self::PostMutationCheck => "PostMutationCheck",
            Self::CpiGuard => "CpiGuard",
            Self::RentExemption => "RentExemption",
            Self::InvariantCheck => "InvariantCheck",
            Self::StateSnapshot => "StateSnapshot",
            Self::LamportConservation => "LamportConservation",
        }
    }
}

// ---------------------------------------------------------------------------
// Policy Class -- categorize what kind of operation a policy governs
// ---------------------------------------------------------------------------

/// High-level classification of what a policy governs.
///
/// Enables Manager, CLI, and receipt narration to group and describe policies
/// meaningfully without parsing individual capability/requirement pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PolicyClass {
    /// Read-only inspection or audit.
    Read = 0,
    /// General state mutation.
    Write = 1,
    /// Financial operation (balance, treasury, token transfers).
    Financial = 2,
    /// Administrative operation (authority changes, permissions).
    Administrative = 3,
    /// Account lifecycle (create, close, migrate).
    Lifecycle = 4,
    /// Cross-program invocation.
    CrossProgram = 5,
    /// Governance or threshold operation (multisig, voting).
    Governance = 6,
}

impl PolicyClass {
    /// Human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Financial => "financial",
            Self::Administrative => "administrative",
            Self::Lifecycle => "lifecycle",
            Self::CrossProgram => "cross-program",
            Self::Governance => "governance",
        }
    }

    /// Whether this class involves any state mutation.
    pub const fn is_mutating(self) -> bool {
        !matches!(self, Self::Read)
    }

    /// Whether this class should require receipt emission.
    pub const fn expects_receipt(self) -> bool {
        !matches!(self, Self::Read)
    }

    /// Infer the policy class from a capability set.
    pub const fn from_capabilities(caps: &CapabilitySet) -> Self {
        if caps.has(Capability::MutatesTreasury) {
            return Self::Financial;
        }
        if caps.has(Capability::ModifiesAuthority) {
            return Self::Administrative;
        }
        if caps.has(Capability::CreatesAccount)
            || caps.has(Capability::ClosesAccount)
            || caps.has(Capability::ReallocatesAccount)
        {
            return Self::Lifecycle;
        }
        if caps.has(Capability::ExternalCall) {
            return Self::CrossProgram;
        }
        if caps.has(Capability::MutatesState) || caps.has(Capability::TouchesJournal) || caps.has(Capability::TransitionsState) {
            return Self::Write;
        }
        Self::Read
    }
}

impl core::fmt::Display for PolicyClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}
