//! Workspace trybuild suite.
//!
//! Two categories:
//!
//! - `tests/ui/pass/*.rs` must compile. Use them to lock in that a
//!   particular macro invocation emits code that typechecks.
//! - `tests/ui/fail/*.rs` must FAIL to compile with the expected
//!   diagnostic captured in the adjacent `.stderr` file. Use them
//!   to lock in error messages for users who misuse a macro.
//!
//! Run with `cargo test -p hopper-trybuild`. Regenerate `.stderr`
//! files with `TRYBUILD=overwrite cargo test -p hopper-trybuild`.

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
fn compile_pass_cases() {
    install_sol_callback_check_cfg();
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass/*.rs");
}

#[test]
fn compile_fail_cases() {
    install_sol_callback_check_cfg();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/*.rs");
}
