//! Trybuild compile-fail harness for Hopper's macro-level safety proofs.
//!
//! Each fixture in `tests/compile_fail/` is a crate input that *must not*
//! compile. The matching `.stderr` snapshot captures the exact compiler
//! error we want to surface. When a refactor changes an error message,
//! run `TRYBUILD=overwrite cargo test --test ui` to regenerate snapshots,
//! then eyeball the diff. if the new message still proves the same
//! safety property, accept it; otherwise investigate.
//!
//! This harness mechanically enforces the Hopper Safety Audit's
//! "Compile-fail coverage" item. The five shipping cases cover
//! `#[hopper::pod]`:
//!
//! | Fixture | Violation |
//! |---|---|
//! | `pod_bool_field.rs` | `bool` field (not all bit patterns valid) |
//! | `pod_char_field.rs` | `char` field (sparse valid code points) |
//! | `pod_reference_field.rs` | `&'static u8` field (pointers are not Pod) |
//! | `pod_missing_repr.rs` | no `#[repr(C)]` / `#[repr(transparent)]` |
//! | `pod_padded_u64.rs` | implicit padding between `u8` and `u64` |
//! | `pod_vec_field.rs` | heap `Vec<u8>` field in a Pod layout |
//! | `zerocopy_seal_required.rs` | bypass `#[hopper::pod]` and hand-roll `Pod`: cannot earn `ZeroCopy` |
//! | `ref_only_rejects_raw_ref.rs` | naked `&mut T` cannot satisfy `HopperRefOnly` (audit Finding 2) |
//!
//! Additional `state_*` fixtures are added in Stage 2 as each
//! `#[account(...)]` constraint attribute lands.

#![cfg(feature = "proc-macros")]

#[test]
fn compile_fail_pod() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/pod_bool_field.rs");
    t.compile_fail("tests/compile_fail/pod_char_field.rs");
    t.compile_fail("tests/compile_fail/pod_reference_field.rs");
    t.compile_fail("tests/compile_fail/pod_missing_repr.rs");
    t.compile_fail("tests/compile_fail/pod_padded_u64.rs");
    t.compile_fail("tests/compile_fail/pod_vec_field.rs");
    t.compile_fail("tests/compile_fail/zerocopy_seal_required.rs");
    t.compile_fail("tests/compile_fail/ref_only_rejects_raw_ref.rs");
}

#[test]
fn compile_fail_state_constraints() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/state_init_no_payer.rs");
    t.compile_fail("tests/compile_fail/state_init_no_space.rs");
    t.compile_fail("tests/compile_fail/state_seeds_no_bump.rs");
    t.compile_fail("tests/compile_fail/state_realloc_no_payer.rs");
    t.compile_fail("tests/compile_fail/state_realloc_no_zero.rs");
}
