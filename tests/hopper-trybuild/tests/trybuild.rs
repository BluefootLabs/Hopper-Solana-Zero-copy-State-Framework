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

#[test]
fn compile_pass_cases() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass/*.rs");
}

#[test]
fn compile_fail_cases() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/*.rs");
}
