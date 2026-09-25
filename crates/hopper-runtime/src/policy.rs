//! Program-level safety policy.
//!
//! Hopper's "policy-driven zero-copy runtime" model exposes each
//! safety lever as a bit in a compile-time const struct. The
//! `#[hopper::program(...)]` macro parses the attribute args and
//! emits `pub const HOPPER_PROGRAM_POLICY: HopperProgramPolicy = ...;`
//! inside the annotated module. Users read it back through
//! [`HopperProgramPolicy`] to specialize handler paths.
//!
//! ## Named modes
//!
//! | Mode | Levers |
//! |---|---|
//! | [`HopperProgramPolicy::STRICT`] | `strict`, `enforce_token_checks`, `allow_unsafe` all on. Recommended default. |
//! | [`HopperProgramPolicy::SEALED`] | `strict` + `enforce_token_checks` on, `allow_unsafe` off. Adds a default unsafe-code denial on handler items. |
//! | [`HopperProgramPolicy::RAW`] | Typed-validation and token-check intent off; unsafe code permitted. Typed handlers still bind. |
//! | [`HopperProgramProfile::TINY`] | Binary-size profile for compact programs: one-byte instruction discriminators and no handler-level modifier instrumentation. |
//!
//! ## Zero runtime cost
//!
//! The policy is consumed by the program macro at compile time.
//! `allow_unsafe = false` emits `#[deny(unsafe_code)]` on each
//! handler so unsafe code in that item is denied by default. Called helpers
//! and dependencies are outside this lint's scope. The handler's parameter
//! type determines whether `ContextSpec::bind(ctx)?` runs: typed handlers
//! bind regardless of `strict`, and raw handlers receive the raw context.
//! `strict` and `enforce_token_checks` are author intent markers, not checks
//! automatically inserted into arbitrary handler code. Authors can consult
//! the constants when selecting explicit token helpers such as
//! `invoke_strict()` and `invoke_signed_strict()`. The `*Checked` builder
//! names describe SPL Token's mint/decimals checks; they do not mean the
//! program policy automatically inserted Hopper authority pre-checks.
//!
//! No runtime flag, no thread-local, no syscall. Users who need to
//! branch on the policy inside a handler read the const directly:
//!
//! ```ignore
//! if super::HOPPER_PROGRAM_POLICY.enforce_token_checks {
//!     hopper_runtime::require!(authority.is_signer());
//! }
//! ```
//!
//! ## Per-instruction overrides
//!
//! A handler can override the program-level policy with
//! `#[instruction(N, unsafe_memory, skip_token_checks, allow_arbitrary_cpi)]`. The macro
//! emits `pub const <HANDLER>_POLICY: HopperInstructionPolicy = ...;`
//! alongside the handler so the same const-branch pattern works at
//! the per-instruction grain.

/// Program-level safety policy emitted by `#[hopper::program(...)]`.
///
/// Each field is a *compile-time* lever. The const value ends up
/// inlined at every call site the program evaluates it from, so the
/// branches fold away when a lever is known to be on or off at
/// compile time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HopperProgramPolicy {
    /// Program-level intent marker: handlers in this program run
    /// under Hopper's full enforcement envelope.
    ///
    /// The actual per-handler behaviour is controlled by the
    /// handler's context parameter type. A handler typed as
    /// `Ctx<MyAccounts>` always runs `MyAccounts::bind(ctx)?`
    /// (which chains into `validate(ctx)?`) regardless of policy. A
    /// handler typed as `&mut Context<'_>` always receives the
    /// context raw. `strict = true` is the documentation contract
    /// that every handler in the module opts into the typed form;
    /// `strict = false` signals the author intends to use raw
    /// contexts and accepts the responsibility of calling
    /// `validate()` manually where needed.
    ///
    /// The flag is read back by callers at compile time
    /// (`HOPPER_PROGRAM_POLICY.strict`) to specialize code paths that
    /// depend on whether the enforcement envelope is active.
    pub strict: bool,

    /// Author-maintained token-check intent. The macro records this flag
    /// without inserting or removing checks from CPI calls. Explicit strict
    /// methods on TransferChecked, BurnChecked, and ApproveChecked check the
    /// token owner field; their direct variants also require signer privilege.
    /// Signed strict variants accept PDA seeds, whose signing authority is
    /// validated during CPI rather than requiring an incoming signer flag.
    pub enforce_token_checks: bool,

    /// Permit `unsafe { ... }` blocks inside handler bodies. When
    /// false the program macro wraps each handler in
    /// `#[deny(unsafe_code)]`. The lint covers that handler item, not called
    /// helpers or dependencies, and ordinary Rust lint override rules apply.
    pub allow_unsafe: bool,
}

/// Program-size/audit profile emitted by `#[hopper::program(profile = "...")]`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HopperProgramProfile {
    Tiny = 0,
    Strict = 1,
    Audit = 2,
    Raw = 3,
}

impl HopperProgramProfile {
    pub const TINY: Self = Self::Tiny;
    pub const STRICT: Self = Self::Strict;
    pub const AUDIT: Self = Self::Audit;
    pub const RAW: Self = Self::Raw;
}

impl HopperProgramPolicy {
    /// Typed-validation and token-check intent enabled; unsafe code permitted.
    /// The shipping default. Handler types and helper calls determine checks.
    pub const STRICT: Self = Self {
        strict: true,
        enforce_token_checks: true,
        allow_unsafe: true,
    };

    /// STRICT intent plus a default unsafe-code denial on handler items.
    /// This does not audit unsafe implementations in called helpers.
    pub const SEALED: Self = Self {
        strict: true,
        enforce_token_checks: true,
        allow_unsafe: false,
    };

    /// Typed-validation and token-check intent disabled; unsafe code permitted.
    /// Typed handlers still bind. Raw handlers own their explicit validation.
    pub const RAW: Self = Self {
        strict: false,
        enforce_token_checks: false,
        allow_unsafe: true,
    };

    /// The shipping default, identical to [`HopperProgramPolicy::STRICT`].
    ///
    /// Exposed as a `const fn` so downstream macro expansion can
    /// reach it from `const` context without an intermediate binding.
    #[inline(always)]
    pub const fn default_policy() -> Self {
        Self::STRICT
    }
}

impl Default for HopperProgramPolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

/// Per-instruction policy override.
///
/// The `#[instruction(N, unsafe_memory, skip_token_checks, allow_arbitrary_cpi, ctx_args = K)]`
/// attribute emits `pub const <HANDLER>_POLICY: HopperInstructionPolicy = ...;`
/// alongside the handler. All fields default to the inherit-from-program
/// behaviour (`false` / `0`) so handlers without overrides get the program
/// policy unchanged.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HopperInstructionPolicy {
    /// Opt this handler out of `#[deny(unsafe_code)]` even when the
    /// program-level `allow_unsafe` is false. Used for the one or two
    /// "fast path" handlers in an otherwise-sealed program.
    pub unsafe_memory: bool,

    /// Declare an exception to the program-level token-check intent.
    /// This does not remove checks from helper calls; authors document how
    /// the handler upholds its token invariants.
    pub skip_token_checks: bool,

    /// Marks a handler as intentionally able to invoke arbitrary external
    /// programs, for governance/proposal executors and plugin dispatchers.
    /// Hopper does not forbid this path; the flag makes the capability visible
    /// to generated schema, review tools, and audit-oriented explain output.
    pub allow_arbitrary_cpi: bool,

    /// Count of leading instruction args the dispatcher threads to the
    /// typed context's `bind_with_args(...)`. `0` means the context
    /// (if any) is bound via `bind(ctx)?` and no args participate in
    /// constraint evaluation. which is the legacy shape and matches
    /// Anchor's non-`#[instruction]` accounts struct. When a context
    /// was declared with `#[instruction(name: Type, ...)]`, the handler
    /// must set `ctx_args` equal to the number of declared args. Generated
    /// code also pins identical names and order, so every seed, constraint,
    /// and exact-cell selector resolves to the same wire value off chain and
    /// on chain.
    pub ctx_args: u8,
}

impl HopperInstructionPolicy {
    /// Inherit every lever from the program-level policy.
    pub const INHERIT: Self = Self {
        unsafe_memory: false,
        skip_token_checks: false,
        allow_arbitrary_cpi: false,
        ctx_args: 0,
    };
}

impl Default for HopperInstructionPolicy {
    fn default() -> Self {
        Self::INHERIT
    }
}

#[cfg(test)]
// These tests assert the field values of `const` policy profiles; the constant
// value of each assertion is precisely the invariant under test.
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn named_modes_differ_on_every_lever() {
        assert!(HopperProgramPolicy::STRICT.strict);
        assert!(HopperProgramPolicy::STRICT.enforce_token_checks);
        assert!(HopperProgramPolicy::STRICT.allow_unsafe);

        assert!(HopperProgramPolicy::SEALED.strict);
        assert!(HopperProgramPolicy::SEALED.enforce_token_checks);
        assert!(!HopperProgramPolicy::SEALED.allow_unsafe);

        assert!(!HopperProgramPolicy::RAW.strict);
        assert!(!HopperProgramPolicy::RAW.enforce_token_checks);
        assert!(HopperProgramPolicy::RAW.allow_unsafe);
    }

    #[test]
    fn program_profiles_are_stable() {
        assert_eq!(HopperProgramProfile::TINY as u8, 0);
        assert_eq!(HopperProgramProfile::STRICT as u8, 1);
        assert_eq!(HopperProgramProfile::AUDIT as u8, 2);
        assert_eq!(HopperProgramProfile::RAW as u8, 3);
    }

    #[test]
    fn default_policy_is_strict() {
        assert_eq!(HopperProgramPolicy::default(), HopperProgramPolicy::STRICT);
        assert_eq!(
            HopperProgramPolicy::default_policy(),
            HopperProgramPolicy::STRICT
        );
    }

    #[test]
    fn instruction_inherit_zeroes_every_lever() {
        assert!(!HopperInstructionPolicy::INHERIT.unsafe_memory);
        assert!(!HopperInstructionPolicy::INHERIT.skip_token_checks);
        assert!(!HopperInstructionPolicy::INHERIT.allow_arbitrary_cpi);
        assert_eq!(HopperInstructionPolicy::INHERIT.ctx_args, 0);
        assert_eq!(
            HopperInstructionPolicy::default(),
            HopperInstructionPolicy::INHERIT
        );
    }

    #[test]
    fn instruction_ctx_args_round_trips() {
        let p = HopperInstructionPolicy {
            unsafe_memory: false,
            skip_token_checks: false,
            allow_arbitrary_cpi: false,
            ctx_args: 3,
        };
        assert_eq!(p.ctx_args, 3);
        assert_ne!(p, HopperInstructionPolicy::INHERIT);
    }
}
