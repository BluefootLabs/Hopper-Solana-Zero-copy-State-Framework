//! Compiler optimization hints.
//!
//! Provides `hint::unlikely` and `hint::cold_path` for guiding branch
//! prediction on the SBF backend. These are the `no_std` equivalents of the
//! nightly `core::intrinsics::unlikely`.

/// Compiler branch-prediction hints.
pub mod hint {
    /// A "dummy" function with a hint to the compiler that it is unlikely to be
    /// called.
    ///
    /// This function is used as a hint to the compiler to optimize other code
    /// paths instead of the one where the function is used.
    #[cold]
    pub const fn cold_path() {}

    /// Return a given `bool` value with a hint to the compiler that `false` is
    /// the likely case.
    #[inline(always)]
    pub const fn unlikely(b: bool) -> bool {
        if b {
            cold_path();
            true
        } else {
            false
        }
    }
}
