//! Context-level schema metadata for the Account DSL.
//!
//! Provides descriptor types that capture context (instruction account struct)
//! metadata for inclusion in ProgramManifest, HopperIdl, and CLI explain output.
//! These are the schema-layer counterparts of the runtime AccountFieldSchema
//! and ContextSchema types in hopper-core.

use core::fmt;

use crate::WriteRange;

/// Schema descriptor for a single account field within a context.
///
/// Richer than the basic `AccountEntry` -- captures the full Account DSL
/// surface including kind, layout, policy, seeds, optionality, and the
/// Anchor-grade lifecycle flags (`init`/`close`/`realloc`/`has_one`) that
/// the Hopper Safety Audit's ST2 closure requires client generators to
/// consume.
#[derive(Clone, Copy)]
pub struct ContextAccountDescriptor {
    /// Field name in the struct (e.g. "vault", "authority").
    pub name: &'static str,
    /// Account wrapper kind (e.g. "HopperAccount", "Signer", "ProgramRef").
    pub kind: &'static str,
    /// Whether the account is writable.
    pub writable: bool,
    /// Whether the account must be a signer.
    pub signer: bool,
    /// Layout name bound via `layout = T`, if any (empty string if none).
    pub layout_ref: &'static str,
    /// Policy pack name bound via `policy = P`, if any (empty string if none).
    pub policy_ref: &'static str,
    /// PDA seed expressions as string representations.
    pub seeds: &'static [&'static str],
    /// Whether the account is optional (may be omitted by the caller).
    pub optional: bool,
    /// Lifecycle role the account plays in this instruction. Clients
    /// use this to synthesize appropriate builder helpers (`findPda`,
    /// `initAccount`, `closeTo`, etc.).
    pub lifecycle: AccountLifecycle,
    /// Name of the field whose key pays CPI fees / rent top-up for
    /// `init` or `realloc`. Empty if not applicable.
    pub payer: &'static str,
    /// Byte count required for `init`. `None` (represented as 0) if
    /// not applicable.
    pub init_space: u32,
    /// Fields listed in `has_one = ...`, required to equal the
    /// corresponding layout field by public key.
    pub has_one: &'static [&'static str],
    /// Address the caller must provide, if pinned via `address = EXPR`
    /// (base58 form for pubkey literals; empty string if not pinned).
    pub expected_address: &'static str,
    /// Program owner the account must be owned by, if pinned via
    /// `owner = EXPR`. Empty string means "owned by the current program".
    pub expected_owner: &'static str,
}

/// Lifecycle role an account plays in one instruction.
///
/// Closes the audit's ST2 schema-metadata gap: clients consuming the
/// manifest need to know which accounts are created/closed/resized so
/// they can synthesize correct builder UX (prompt for payer, compute
/// required rent, wire a close-recipient, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountLifecycle {
    /// Account exists before the instruction and is only read/mutated.
    Existing,
    /// Account is created fresh this instruction (`#[account(init, ...)]`).
    Init,
    /// Account may be created by this instruction if it does not already exist.
    InitIfNeeded,
    /// Account data is resized this instruction.
    Realloc,
    /// Account is drained and reassigned to the System Program.
    Close,
}

impl AccountLifecycle {
    pub const fn as_str(&self) -> &'static str {
        match self {
            AccountLifecycle::Existing => "existing",
            AccountLifecycle::Init => "init",
            AccountLifecycle::InitIfNeeded => "init_if_needed",
            AccountLifecycle::Realloc => "realloc",
            AccountLifecycle::Close => "close",
        }
    }
}

/// Normalize schema and manifest lifecycle strings into Hopper metadata.
pub fn normalize_lifecycle(s: &str) -> AccountLifecycle {
    let s = s.trim();
    if lifecycle_token_eq(s, "initifneeded") || lifecycle_token_eq(s, "createifneeded") {
        AccountLifecycle::InitIfNeeded
    } else if lifecycle_token_eq(s, "init") || lifecycle_token_eq(s, "create") {
        AccountLifecycle::Init
    } else if lifecycle_token_eq(s, "realloc") || lifecycle_token_eq(s, "reallocate") {
        AccountLifecycle::Realloc
    } else if lifecycle_token_eq(s, "close") {
        AccountLifecycle::Close
    } else {
        AccountLifecycle::Existing
    }
}

fn lifecycle_token_eq(input: &str, canonical: &str) -> bool {
    let canonical = canonical.as_bytes();
    let mut canonical_idx = 0;
    for byte in input.bytes() {
        if matches!(byte, b'_' | b'-') {
            continue;
        }
        if canonical_idx >= canonical.len() || byte.to_ascii_lowercase() != canonical[canonical_idx]
        {
            return false;
        }
        canonical_idx += 1;
    }
    canonical_idx == canonical.len()
}

impl fmt::Display for ContextAccountDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.kind)?;
        if self.writable {
            write!(f, " [mut]")?;
        }
        if self.signer {
            write!(f, " [signer]")?;
        }
        if !self.layout_ref.is_empty() {
            write!(f, " layout={}", self.layout_ref)?;
        }
        if !self.policy_ref.is_empty() {
            write!(f, " policy={}", self.policy_ref)?;
        }
        if self.optional {
            write!(f, " [optional]")?;
        }
        if !self.seeds.is_empty() {
            write!(f, " seeds=[")?;
            for (i, s) in self.seeds.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", s)?;
            }
            write!(f, "]")?;
        }
        Ok(())
    }
}

/// Schema descriptor for an entire instruction context (account struct).
///
/// Captures the full Account DSL metadata for a single instruction's
/// account requirements. Used for explain output, schema comparison,
/// and manifest inclusion.
#[derive(Clone, Copy)]
pub struct ContextDescriptor {
    /// Context struct name (e.g. "Deposit", "Withdraw").
    pub name: &'static str,
    /// Per-account field descriptors.
    pub accounts: &'static [ContextAccountDescriptor],
    /// Policy pack names used by this context.
    pub policies: &'static [&'static str],
    /// Whether receipts are expected from this instruction.
    pub receipts_expected: bool,
    /// Mutation class names (e.g. "Financial", "InPlace").
    pub mutation_classes: &'static [&'static str],
    /// Whether the context was compiled with
    /// `#[hopper::context(strict_writes)]`.
    ///
    /// When `true`, [`write_ranges`](Self::write_ranges) is the complete
    /// write surface the runtime `WritePolicy` enforces during `bind()`.
    /// When `false`, the range set is empty and carries no authority
    /// (mirroring `InstructionDescriptor::strict_writes`).
    pub strict_writes: bool,
    /// Declared byte-range write-set for this context.
    ///
    /// Auto-populated by the `#[hopper::context]` macro from the very
    /// same generated const that backs the runtime `WritePolicy` and the
    /// context's `WRITE_RANGES` associated const, so the published set
    /// is byte-identical to the enforced set by construction. Each
    /// [`WriteRange::account_index`] is the field's position in
    /// [`accounts`](Self::accounts). Empty unless
    /// [`strict_writes`](Self::strict_writes) is `true`.
    pub write_ranges: &'static [WriteRange],
    /// Whether this context's declared write set covers **both**
    /// mutation dimensions — data ranges and lamports (BLD-MUT).
    ///
    /// `true` only for `#[hopper::context(strict_writes,
    /// lamports(...))]`; a bare `strict_writes` context leaves its
    /// lamport behavior undeclared and is not mutation-complete
    /// (mirroring `InstructionDescriptor::mutation_complete`).
    pub mutation_complete: bool,
    /// Account indices permitted to have their lamports mutated —
    /// explicit `lamports(...)` names plus the macro's implied
    /// lifecycle set. The same generated const the runtime enforces.
    /// Empty (and carrying no authority) unless
    /// [`mutation_complete`](Self::mutation_complete) is `true`.
    pub lamport_accounts: &'static [u8],
}

impl ContextDescriptor {
    /// Number of accounts in this context.
    pub const fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Number of signer accounts.
    pub fn signer_count(&self) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < self.accounts.len() {
            if self.accounts[i].signer {
                count += 1;
            }
            i += 1;
        }
        count
    }

    /// Number of writable accounts.
    pub fn writable_count(&self) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < self.accounts.len() {
            if self.accounts[i].writable {
                count += 1;
            }
            i += 1;
        }
        count
    }

    /// Find an account descriptor by field name.
    pub fn find_account(&self, name: &str) -> Option<&ContextAccountDescriptor> {
        let mut i = 0;
        while i < self.accounts.len() {
            if str_eq(self.accounts[i].name, name) {
                return Some(&self.accounts[i]);
            }
            i += 1;
        }
        None
    }
}

impl fmt::Display for ContextDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Context: {}", self.name)?;
        for acct in self.accounts {
            writeln!(f, "  {}", acct)?;
        }
        if !self.policies.is_empty() {
            write!(f, "  Policies:")?;
            for p in self.policies {
                write!(f, " {}", p)?;
            }
            writeln!(f)?;
        }
        if self.receipts_expected {
            writeln!(f, "  Receipts: expected")?;
        }
        if !self.mutation_classes.is_empty() {
            write!(f, "  Mutations:")?;
            for m in self.mutation_classes {
                write!(f, " {}", m)?;
            }
            writeln!(f)?;
        }
        if self.strict_writes {
            writeln!(f, "  Strict writes: {} range(s)", self.write_ranges.len())?;
            for r in self.write_ranges {
                if r.size == u32::MAX {
                    writeln!(f, "    account[{}] whole account", r.account_index)?;
                } else {
                    writeln!(
                        f,
                        "    account[{}] bytes [{}, +{})",
                        r.account_index, r.offset, r.size
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Byte-by-byte string equality for const-compatible contexts.
#[inline]
fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::format;

    static TEST_ACCOUNTS: &[ContextAccountDescriptor] = &[
        ContextAccountDescriptor {
            name: "authority",
            kind: "Signer",
            writable: true,
            signer: true,
            layout_ref: "",
            policy_ref: "",
            seeds: &[],
            optional: false,
            lifecycle: AccountLifecycle::Existing,
            payer: "",
            init_space: 0,
            has_one: &[],
            expected_address: "",
            expected_owner: "",
        },
        ContextAccountDescriptor {
            name: "vault",
            kind: "HopperAccount",
            writable: true,
            signer: false,
            layout_ref: "VaultState",
            policy_ref: "TREASURY_WRITE",
            seeds: &["b\"vault\"", "authority"],
            optional: false,
            lifecycle: AccountLifecycle::Existing,
            payer: "",
            init_space: 0,
            has_one: &["authority"],
            expected_address: "",
            expected_owner: "",
        },
        ContextAccountDescriptor {
            name: "system_program",
            kind: "ProgramRef",
            writable: false,
            signer: false,
            layout_ref: "",
            policy_ref: "",
            seeds: &[],
            optional: false,
            lifecycle: AccountLifecycle::Existing,
            payer: "",
            init_space: 0,
            has_one: &[],
            expected_address: "",
            expected_owner: "",
        },
    ];

    static TEST_CTX: ContextDescriptor = ContextDescriptor {
        name: "Deposit",
        accounts: TEST_ACCOUNTS,
        policies: &["TREASURY_WRITE"],
        receipts_expected: true,
        mutation_classes: &["Financial"],
        strict_writes: true,
        write_ranges: &[WriteRange::new(1, 16, 8), WriteRange::whole_account(0)],
        mutation_complete: false,
        lamport_accounts: &[],
    };

    #[test]
    fn context_descriptor_counts() {
        assert_eq!(TEST_CTX.account_count(), 3);
        assert_eq!(TEST_CTX.signer_count(), 1);
        assert_eq!(TEST_CTX.writable_count(), 2);
    }

    #[test]
    fn context_descriptor_find() {
        let found = TEST_CTX.find_account("vault");
        assert!(found.is_some());
        let vault = found.unwrap();
        assert_eq!(vault.kind, "HopperAccount");
        assert_eq!(vault.layout_ref, "VaultState");
        assert_eq!(vault.seeds.len(), 2);
        assert!(vault.writable);
        assert!(!vault.signer);

        assert!(TEST_CTX.find_account("nonexistent").is_none());
    }

    #[test]
    fn context_descriptor_display() {
        let s = format!("{}", TEST_CTX);
        assert!(s.contains("Context: Deposit"));
        assert!(s.contains("authority: Signer"));
        assert!(s.contains("[mut]"));
        assert!(s.contains("[signer]"));
        assert!(s.contains("layout=VaultState"));
        assert!(s.contains("policy=TREASURY_WRITE"));
        assert!(s.contains("seeds=["));
        assert!(s.contains("Policies: TREASURY_WRITE"));
        assert!(s.contains("Mutations: Financial"));
        assert!(s.contains("Strict writes: 2 range(s)"));
        assert!(s.contains("account[1] bytes [16, +8)"));
        assert!(s.contains("account[0] whole account"));
    }

    #[test]
    fn context_descriptor_without_strict_writes_omits_range_section() {
        static PLAIN: ContextDescriptor = ContextDescriptor {
            name: "Inspect",
            accounts: TEST_ACCOUNTS,
            policies: &[],
            receipts_expected: false,
            mutation_classes: &[],
            strict_writes: false,
            write_ranges: &[],
            mutation_complete: false,
            lamport_accounts: &[],
        };
        let s = format!("{}", PLAIN);
        assert!(!s.contains("Strict writes"));
    }

    #[test]
    fn account_descriptor_display() {
        let s = format!("{}", TEST_ACCOUNTS[2]);
        assert!(s.contains("system_program: ProgramRef"));
        assert!(!s.contains("[mut]"));
        assert!(!s.contains("[signer]"));
    }

    #[test]
    fn optional_account_display() {
        let opt = ContextAccountDescriptor {
            name: "extra",
            kind: "Unchecked",
            writable: false,
            signer: false,
            layout_ref: "",
            policy_ref: "",
            seeds: &[],
            optional: true,
            lifecycle: AccountLifecycle::Existing,
            payer: "",
            init_space: 0,
            has_one: &[],
            expected_address: "",
            expected_owner: "",
        };
        let s = format!("{}", opt);
        assert!(s.contains("[optional]"));
    }

    #[test]
    fn lifecycle_as_str_roundtrips_all_variants() {
        assert_eq!(AccountLifecycle::Existing.as_str(), "existing");
        assert_eq!(AccountLifecycle::Init.as_str(), "init");
        assert_eq!(AccountLifecycle::InitIfNeeded.as_str(), "init_if_needed");
        assert_eq!(AccountLifecycle::Realloc.as_str(), "realloc");
        assert_eq!(AccountLifecycle::Close.as_str(), "close");
    }

    #[test]
    fn normalize_lifecycle_maps_init_if_needed_aliases() {
        for alias in [
            "init_if_needed",
            "initIfNeeded",
            "init-if-needed",
            "create_if_needed",
        ] {
            assert_eq!(normalize_lifecycle(alias), AccountLifecycle::InitIfNeeded);
        }
    }

    #[test]
    fn normalize_lifecycle_preserves_existing_variants() {
        assert_eq!(normalize_lifecycle("init"), AccountLifecycle::Init);
        assert_eq!(normalize_lifecycle("realloc"), AccountLifecycle::Realloc);
        assert_eq!(normalize_lifecycle("close"), AccountLifecycle::Close);
        assert_eq!(normalize_lifecycle(""), AccountLifecycle::Existing);
        assert_eq!(normalize_lifecycle("unknown"), AccountLifecycle::Existing);
    }

    #[test]
    fn init_account_descriptor_carries_lifecycle_metadata() {
        let init_acc = ContextAccountDescriptor {
            name: "position",
            kind: "InitAccount",
            writable: true,
            signer: false,
            layout_ref: "Position",
            policy_ref: "",
            seeds: &["b\"position\"", "authority.key()"],
            optional: false,
            lifecycle: AccountLifecycle::Init,
            payer: "authority",
            init_space: 128,
            has_one: &[],
            expected_address: "",
            expected_owner: "",
        };
        assert_eq!(init_acc.lifecycle, AccountLifecycle::Init);
        assert_eq!(init_acc.payer, "authority");
        assert_eq!(init_acc.init_space, 128);
        assert_eq!(init_acc.seeds.len(), 2);
    }

    #[test]
    fn close_account_descriptor_roundtrips() {
        let close_acc = ContextAccountDescriptor {
            name: "vault",
            kind: "HopperAccount",
            writable: true,
            signer: false,
            layout_ref: "Vault",
            policy_ref: "",
            seeds: &[],
            optional: false,
            lifecycle: AccountLifecycle::Close,
            payer: "",
            init_space: 0,
            has_one: &[],
            expected_address: "",
            expected_owner: "",
        };
        assert_eq!(close_acc.lifecycle.as_str(), "close");
    }
}
