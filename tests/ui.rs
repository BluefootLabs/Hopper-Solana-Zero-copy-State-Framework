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
//! "Compile-fail coverage" item. The current root suite has 17
//! compile-fail fixtures covering `#[hopper::pod]`, state constraints,
//! dynamic tails, tiny profile restrictions, and borrow guard proofs:
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
//! | `quasar_mut_account_reference.rs` | Quasar-style `&mut Account<T>` fields get a Hopper-specific fix-it |
//! | `bare_tail_not_final.rs` | `TailStr<'a>` / `TailBytes<'a>` bare tails must be final account fields |
//! | `tiny_profile_multibyte_discriminator.rs` | tiny programs must keep one-byte dispatch |
//! | `tiny_profile_receipt_modifier.rs` | tiny programs cannot inject handler modifier instrumentation |
//!
//! The newer `tests/hopper-trybuild` package carries the generated
//! macro-DX fixtures that exercise `hopper::prelude::*` and downstream
//! check-cfg behavior.

#![cfg(feature = "proc-macros")]

fn install_sol_callback_check_cfg() {
    const CHECK_CFG: &str = "--check-cfg=cfg(target_os,values(\"solana\")) --check-cfg=cfg(target_arch,values(\"bpf\"))";
    const CHECK_CFG_ENCODED: &str = "--check-cfg=cfg(target_os,values(\"solana\"))\x1f--check-cfg=cfg(target_arch,values(\"bpf\"))";
    let rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    if !rustflags.contains("cfg(target_os,values(\"solana\"))") {
        let rustflags = if rustflags.trim().is_empty() {
            CHECK_CFG.to_string()
        } else {
            format!("{} {}", rustflags, CHECK_CFG)
        };
        std::env::set_var("RUSTFLAGS", rustflags);
    }

    let encoded = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    if encoded.contains("cfg(target_os,values(\"solana\"))") {
        return;
    }
    let encoded = if encoded.trim().is_empty() {
        CHECK_CFG_ENCODED.to_string()
    } else {
        format!("{}\x1f{}", encoded, CHECK_CFG_ENCODED)
    };
    std::env::set_var("CARGO_ENCODED_RUSTFLAGS", encoded);
}

#[test]
fn compile_fail_pod() {
    install_sol_callback_check_cfg();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/pod_bool_field.rs");
    t.compile_fail("tests/compile_fail/pod_char_field.rs");
    t.compile_fail("tests/compile_fail/pod_reference_field.rs");
    t.compile_fail("tests/compile_fail/pod_missing_repr.rs");
    t.compile_fail("tests/compile_fail/pod_padded_u64.rs");
    t.compile_fail("tests/compile_fail/pod_vec_field.rs");
    t.compile_fail("tests/compile_fail/zerocopy_seal_required.rs");
    t.compile_fail("tests/compile_fail/ref_only_rejects_raw_ref.rs");
    t.compile_fail("tests/compile_fail/quasar_mut_account_reference.rs");
}

#[test]
fn compile_fail_state_constraints() {
    install_sol_callback_check_cfg();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/state_init_no_payer.rs");
    t.compile_fail("tests/compile_fail/state_init_no_space.rs");
    t.compile_fail("tests/compile_fail/state_seeds_no_bump.rs");
    t.compile_fail("tests/compile_fail/state_realloc_no_payer.rs");
    t.compile_fail("tests/compile_fail/state_realloc_no_zero.rs");
}

#[test]
fn compile_fail_dynamic_tails() {
    install_sol_callback_check_cfg();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/bare_tail_not_final.rs");
}

#[test]
fn compile_fail_tiny_profile() {
    install_sol_callback_check_cfg();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/tiny_profile_multibyte_discriminator.rs");
    t.compile_fail("tests/compile_fail/tiny_profile_receipt_modifier.rs");
}
