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
//! | [`HopperProgramPolicy::SEALED`] | `strict` + `enforce_token_checks` on, `allow_unsafe` off. Zero-`unsafe`-in-handlers programs. |
//! | [`HopperProgramPolicy::RAW`] | Every lever off. Pinocchio-parity throughput. Responsibility shifts fully to the handler author. |
//!
//! ## Zero runtime cost
//!
//! The policy is consumed by the program macro at compile time.
//! `allow_unsafe = false` emits `#[deny(unsafe_code)]` on each
//! handler so a stray `unsafe` block fails to compile. `strict`
//! toggles auto-injection of `ContextSpec::bind(ctx)?` (which in turn
//! calls `validate(ctx)?`). `enforce_token_checks` is a load-bearing
//! promise read back by the author from
//! `HOPPER_PROGRAM_POLICY.enforce_token_checks` to decide whether to
//! invoke the `*Checked` token CPI pre-check helpers in handlers that
//! reach outside the typed-context envelope.
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
//! `#[instruction(N, unsafe_memory, skip_token_checks)]`. The macro
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
    /// `Context<MyAccounts>` always runs `MyAccounts::bind(ctx)?`
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

    /// Token CPI authors must pair every raw invocation with the
    /// matching `*Checked` builder (which carries the `decimals: u8`
    /// byte the SPL Token program validates against the mint).
    /// Handlers that do their own SPL plumbing read this back to
    /// decide whether the signer + owner invariants are already
    /// upheld elsewhere.
    pub enforce_token_checks: bool,

    /// Permit `unsafe { ... }` blocks inside handler bodies. When
    /// false the program macro wraps each handler in
    /// `#[deny(unsafe_code)]` so the compiler rejects any raw pointer
    /// detour.
    pub allow_unsafe: bool,
}

impl HopperProgramPolicy {
    /// Every safety lever engaged. The shipping default.
    pub const STRICT: Self = Self {
        strict: true,
        enforce_token_checks: true,
        allow_unsafe: true,
    };

    /// Strict + token checks + no `unsafe` in handlers. The zero-escape
    /// mode for programs that never want to drop to raw pointers.
    pub const SEALED: Self = Self {
        strict: true,
        enforce_token_checks: true,
        allow_unsafe: false,
    };

    /// Every lever disengaged. Pinocchio-parity throughput with
    /// responsibility pushed to the handler author.
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
/// The `#[instruction(N, unsafe_memory, skip_token_checks)]`
/// attribute emits `pub const <HANDLER>_POLICY: HopperInstructionPolicy = ...;`
/// alongside the handler. Both fields default to the inherit-from-program
/// behaviour (`false`) so handlers without overrides get the program
/// policy unchanged.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HopperInstructionPolicy {
    /// Opt this handler out of `#[deny(unsafe_code)]` even when the
    /// program-level `allow_unsafe` is false. Used for the one or two
    /// "fast path" handlers in an otherwise-sealed program.
    pub unsafe_memory: bool,

    /// Skip the program-level token-check promise for this handler.
    /// The handler still compiles, but authors must document why the
    /// token invariants are upheld through some other mechanism.
    pub skip_token_checks: bool,
}

impl HopperInstructionPolicy {
    /// Inherit every lever from the program-level policy.
    pub const INHERIT: Self = Self {
        unsafe_memory: false,
        skip_token_checks: false,
    };
}

impl Default for HopperInstructionPolicy {
    fn default() -> Self {
        Self::INHERIT
    }
}

#[cfg(test)]
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
    fn default_policy_is_strict() {
        assert_eq!(HopperProgramPolicy::default(), HopperProgramPolicy::STRICT);
        assert_eq!(HopperProgramPolicy::default_policy(), HopperProgramPolicy::STRICT);
    }

    #[test]
    fn instruction_inherit_zeroes_every_lever() {
        assert!(!HopperInstructionPolicy::INHERIT.unsafe_memory);
        assert!(!HopperInstructionPolicy::INHERIT.skip_token_checks);
        assert_eq!(HopperInstructionPolicy::default(), HopperInstructionPolicy::INHERIT);
    }
}
