#![no_std]
//! # hopper-finance
//!
//! DeFi math primitives for Hopper programs.
//!
//! AMM constant-product swap formulas, integer square root, K-invariant
//! verification, LP token minting, slippage guards, and economic boundary
//! checks. All u128 intermediates, all overflow-checked, all `#[inline(always)]`.
//!
//! ```rust,ignore
//! use hopper_finance::prelude::*;
//! ```

pub mod amm;
pub mod slippage;

pub mod prelude {
    //! Convenience re-exports for `use hopper_finance::prelude::*`.
    pub use crate::amm::*;
    pub use crate::slippage::*;
}

// ── Re-exports ───────────────────────────────────────────────────────────────

pub use amm::{
    check_k_invariant, constant_product_in, constant_product_out, initial_lp_amount, isqrt,
    price_impact_bps, proportional_lp_amount,
};
pub use slippage::{
    check_max_amount, check_max_input, check_min_amount, check_nonzero, check_price_bounds,
    check_slippage, check_within_bps,
};
