//! `#[bump]` marker + `bump = stored`: the explicit stored-bump golden path.
//!
//! The state author marks the canonical-bump field with `#[bump]`; the
//! context author writes `bump = stored`. The macro then verifies the PDA
//! with ONE `create_program_address` hash against the byte read from the
//! already-validated account layout — no `find_program_address` search, no
//! hand-written `config.load::<T>()?.bump` expression, and no name-based
//! auto-detection anywhere (a field merely named `bump` changes nothing).

#![cfg(feature = "proc-macros")]

use hopper::layout::HEADER_LEN;
use hopper::prelude::*;

const CFG_SEED: &[u8] = b"stored-bump-cfg";

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 77, version = 1)]
pub struct BumpedConfig {
    pub admin: Address,
    /// The canonical bump this program wrote at init.
    #[bump]
    pub bump: u8,
    pub reserved: [u8; 7],
}

#[hopper::context]
pub struct UseStored {
    #[account(seeds = [CFG_SEED], bump = stored)]
    pub config: BumpedConfig,
}

/// The marker emits the account-absolute offset of the marked field.
#[test]
fn canonical_bump_abs_offset_points_at_the_marked_field() {
    assert_eq!(
        BumpedConfig::CANONICAL_BUMP_ABS_OFFSET,
        HEADER_LEN as u32 + BumpedConfig::BUMP_OFFSET,
    );
    // `admin: Address` precedes it, so the body-relative offset is 32.
    assert_eq!(BumpedConfig::BUMP_OFFSET, 32);
}

/// The context compiles against the marker-emitted const: the bind path
/// reads `<BumpedConfig>::CANONICAL_BUMP_ABS_OFFSET` and verifies with one
/// `create_program_address` hash. PDA derivation needs the SVM sha256
/// syscall, so the BEHAVIORAL proof (canonical account binds; a tampered
/// bump byte or a foreign address is refused) lives in the compiled-SBF
/// suite: `examples/hopper-cicada/tests/lifecycle_sbf_e2e.rs`, where every
/// lifecycle context now uses `bump = stored` against the `#[bump]`-marked
/// `CicadaConfig.bump`.
#[test]
fn stored_bump_context_compiles_against_the_marker_const() {
    // Force the generated bind (and its use of the const) to typecheck and
    // link; the seed constant participates so the whole attribute parses.
    let _ = CFG_SEED;
    let _ = UseStored::ACCOUNT_COUNT;
    let _ = BumpedConfig::CANONICAL_BUMP_ABS_OFFSET;
}
