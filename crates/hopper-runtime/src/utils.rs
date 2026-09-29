//! Small utilities for Hopper program authors.
//!
//! The line for inclusion is narrow: a helper lands here when it is
//! too small to justify its own module and too broadly useful to
//! leave as a local copy in three places. At the moment that means
//! branch-prediction hints; future additions fit the same rule or
//! they do not fit at all.

/// Branch-prediction hints for hot handlers.
///
/// Both functions are identity over `bool` on every supported
/// target. The call site cost is zero: the compiler inlines them
/// into the containing branch with no runtime effect. What they
/// communicate is intent. A handler that expects the fast path to
/// win 99% of the time writes
///
/// ```ignore
/// if hopper::utils::hint::likely(is_cached) {
///     return fast_path(ctx);
/// }
/// slow_path(ctx)
/// ```
///
/// and the compiler keeps the fast path straight-line, pushing the
/// spill into the cold branch. The `unlikely` helper expresses the inverse
/// opposite direction.
///
/// On host targets (tests, off-chain tooling) future versions may
/// route through `core::intrinsics::likely` when that intrinsic is
/// stabilized; the API shape stays identical so user code never
/// needs to change. On SBF the Solana runtime does not expose a
/// branch-weight hint today, so the hint compiles out entirely.
/// Leaving the calls in the code path is safe and free.
pub mod hint {
    /// Hint the branch condition is probably `true`. Zero runtime
    /// cost on SBF; on host targets it maps to LLVM's
    /// `llvm.expect.i1` via `core::hint::likely` when available,
    /// and to an identity otherwise.
    #[inline(always)]
    pub const fn likely(cond: bool) -> bool {
        // `core::hint::likely` is not stable. A call to a `#[cold]`
        // function on the other arm tells the optimizer the same thing:
        // the arm that reaches it is the unlikely one.
        if cond {
            true
        } else {
            cold_path();
            false
        }
    }

    /// Hint the branch condition is probably `false`. Mirror of
    /// [`likely`].
    #[inline(always)]
    pub const fn unlikely(cond: bool) -> bool {
        if cond {
            cold_path();
            true
        } else {
            false
        }
    }

    /// Marks the path that calls it as rarely taken. Empty, so it costs
    /// no instruction; the optimizer lays the other path out as the
    /// fall-through.
    #[cold]
    #[inline(always)]
    pub const fn cold_path() {}

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn likely_returns_its_condition() {
            assert!(likely(true));
            assert!(!likely(false));
        }

        #[test]
        fn unlikely_returns_its_condition() {
            assert!(unlikely(true));
            assert!(!unlikely(false));
        }
    }
}
