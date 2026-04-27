//! Account metadata provider trait.

/// Trait for types that provide static metadata about themselves.
///
/// Implementors expose their account name and kind for schema generation
/// and introspection. Typically derived or manually implemented alongside
/// `HopperAccounts`.
pub trait AccountMetaProvider {
    /// Human-readable account name (e.g. "vault", "authority").
    fn account_name() -> &'static str;
    /// Account kind identifier (e.g. "HopperAccount", "Signer", "ProgramRef").
    fn account_kind() -> &'static str;
}
