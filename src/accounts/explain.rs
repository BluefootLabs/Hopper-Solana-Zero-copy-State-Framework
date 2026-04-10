//! Structured explain output for contexts and accounts.

/// Schema metadata for a single account field in a context.
#[derive(Clone, Copy)]
pub struct AccountFieldSchema {
    /// Field name in the struct.
    pub name: &'static str,
    /// Account kind (e.g. "HopperAccount", "Signer", "ProgramRef").
    pub kind: &'static str,
    /// Whether the account is writable.
    pub mutable: bool,
    /// Whether the account must be a signer.
    pub signer: bool,
    /// Layout name bound via `layout = T`, if any.
    pub layout: Option<&'static str>,
    /// Policy pack name bound via `policy = P`, if any.
    pub policy: Option<&'static str>,
    /// PDA seed expressions (as string representations).
    pub seeds: &'static [&'static str],
    /// Whether the account is optional.
    pub optional: bool,
}

/// Schema metadata for an entire instruction context.
#[derive(Clone, Copy)]
pub struct ContextSchema {
    /// Context struct name (e.g. "Deposit").
    pub name: &'static str,
    /// Per-account field schemas.
    pub fields: &'static [AccountFieldSchema],
    /// Policy pack names used by this context.
    pub policy_names: &'static [&'static str],
    /// Whether receipts are expected from this instruction.
    pub receipts_expected: bool,
    /// Mutation class names (e.g. "Financial", "InPlace").
    pub mutation_classes: &'static [&'static str],
}

/// Human-readable explanation of a context.
pub struct ContextExplain {
    /// Context name.
    pub context_name: &'static str,
    /// Per-account field schemas (full metadata for each account).
    pub fields: &'static [AccountFieldSchema],
    /// Policy name list.
    pub policies: &'static [&'static str],
    /// Whether receipts are expected.
    pub receipts_expected: bool,
    /// Mutation class name list.
    pub mutation_classes: &'static [&'static str],
}

impl ContextExplain {
    /// Build an explain from a schema, or return a blank if no schema exists.
    pub fn from_schema(schema: Option<&'static ContextSchema>) -> Self {
        match schema {
            Some(s) => Self {
                context_name: s.name,
                fields: s.fields,
                policies: s.policy_names,
                receipts_expected: s.receipts_expected,
                mutation_classes: s.mutation_classes,
            },
            None => Self {
                context_name: "(unknown)",
                fields: &[],
                policies: &[],
                receipts_expected: false,
                mutation_classes: &[],
            },
        }
    }

    /// Number of accounts in this context.
    pub fn account_count(&self) -> usize {
        self.fields.len()
    }

    /// Number of signer accounts.
    pub fn signer_count(&self) -> usize {
        self.fields.iter().filter(|f| f.signer).count()
    }

    /// Number of writable accounts.
    pub fn writable_count(&self) -> usize {
        self.fields.iter().filter(|f| f.mutable).count()
    }
}

/// Human-readable explanation of a single account.
pub struct AccountExplain {
    /// Field name.
    pub name: &'static str,
    /// Account kind.
    pub kind: &'static str,
    /// Layout name, if bound.
    pub layout: Option<&'static str>,
    /// Policy name, if bound.
    pub policy: Option<&'static str>,
    /// Whether the account is writable.
    pub mutable: bool,
    /// Whether the account is a signer.
    pub signer: bool,
    /// Whether the account is optional.
    pub optional: bool,
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;

    static TEST_FIELDS: &[AccountFieldSchema] = &[
        AccountFieldSchema {
            name: "authority",
            kind: "Signer",
            mutable: true,
            signer: true,
            layout: None,
            policy: None,
            seeds: &[],
            optional: false,
        },
        AccountFieldSchema {
            name: "vault",
            kind: "HopperAccount",
            mutable: true,
            signer: false,
            layout: Some("VaultState"),
            policy: Some("TREASURY_WRITE"),
            seeds: &["b\"vault\"", "authority"],
            optional: false,
        },
        AccountFieldSchema {
            name: "system_program",
            kind: "ProgramRef",
            mutable: false,
            signer: false,
            layout: None,
            policy: None,
            seeds: &[],
            optional: false,
        },
    ];

    static TEST_SCHEMA: ContextSchema = ContextSchema {
        name: "Deposit",
        fields: TEST_FIELDS,
        policy_names: &["TREASURY_WRITE"],
        receipts_expected: true,
        mutation_classes: &["Financial"],
    };

    #[test]
    fn context_explain_from_schema() {
        let explain = ContextExplain::from_schema(Some(&TEST_SCHEMA));
        assert_eq!(explain.context_name, "Deposit");
        assert_eq!(explain.fields.len(), 3);
        assert_eq!(explain.fields[0].name, "authority");
        assert_eq!(explain.fields[1].name, "vault");
        assert_eq!(explain.policies.len(), 1);
        assert_eq!(explain.policies[0], "TREASURY_WRITE");
        assert!(explain.receipts_expected);
        assert_eq!(explain.mutation_classes.len(), 1);
        assert_eq!(explain.mutation_classes[0], "Financial");
        assert_eq!(explain.account_count(), 3);
        assert_eq!(explain.signer_count(), 1);
        assert_eq!(explain.writable_count(), 2);
    }

    #[test]
    fn context_explain_from_none() {
        let explain = ContextExplain::from_schema(None);
        assert_eq!(explain.context_name, "(unknown)");
        assert!(!explain.receipts_expected);
        assert!(explain.policies.is_empty());
        assert_eq!(explain.account_count(), 0);
        assert_eq!(explain.signer_count(), 0);
        assert_eq!(explain.writable_count(), 0);
    }

    #[test]
    fn account_field_schema_fields() {
        assert_eq!(TEST_FIELDS[0].name, "authority");
        assert!(TEST_FIELDS[0].signer);
        assert!(TEST_FIELDS[0].mutable);
        assert!(TEST_FIELDS[0].layout.is_none());

        assert_eq!(TEST_FIELDS[1].name, "vault");
        assert!(!TEST_FIELDS[1].signer);
        assert!(TEST_FIELDS[1].mutable);
        assert_eq!(TEST_FIELDS[1].layout, Some("VaultState"));
        assert_eq!(TEST_FIELDS[1].seeds.len(), 2);
    }

    #[test]
    fn context_schema_field_count() {
        assert_eq!(TEST_SCHEMA.fields.len(), 3);
        assert_eq!(TEST_SCHEMA.name, "Deposit");
        assert_eq!(TEST_SCHEMA.policy_names.len(), 1);
        assert_eq!(TEST_SCHEMA.mutation_classes.len(), 1);
    }
}
