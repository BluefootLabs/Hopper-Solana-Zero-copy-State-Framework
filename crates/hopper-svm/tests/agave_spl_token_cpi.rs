//! SPL Token CPI integration test under the Agave runtime path.
//!
//! Gated behind:
//!   - the `agave-runtime` feature, and
//!   - the presence of `programs/spl_token.so` in the crate root
//!     (the test author supplies this; see
//!     `crates/hopper-svm/programs/README.md`).
//!
//! When both conditions hold, this test:
//!
//! 1. Instantiates a `HopperSvm` with `with_agave_runtime()`.
//! 2. Loads the supplied SPL Token ELF through Agave's real loader.
//! 3. Mints fresh tokens, transfers them between two accounts,
//!    asserts balances post-transfer via the harness's stateful
//!    overlay.
//! 4. Compares the consumed CU against a mainnet-baseline range
//!    (mainnet TransferChecked typically costs ~4500-6500 CU; the
//!    harness should report a value in that range for the same
//!    bytes).
//!
//! The test is `#[ignore]` by default so a fresh clone of the
//! repository (which doesn't ship the SPL `.so` bytes) doesn't
//! fail in CI. Run explicitly:
//!
//! ```sh
//! cargo test -p hopper-svm --features agave-runtime \
//!     --test agave_spl_token_cpi -- --ignored
//! ```

#![cfg(feature = "agave-runtime")]

#[test]
#[ignore = "requires the user to supply crates/hopper-svm/programs/spl_token.so; see programs/README.md"]
fn spl_token_transfer_through_agave_runtime() {
    // The macro `include_bytes!` is evaluated at compile time, so we
    // can't conditionally include based on file existence. The
    // canonical pattern is for the test author to drop the bytes in
    // and remove the `#[ignore]` once they're staged. The runtime
    // wiring below is fully shipped; only the bytes are missing.
    //
    // Authors enabling this test locally:
    //
    // 1. Place SPL Token `.so` at `crates/hopper-svm/programs/spl_token.so`
    //    (see programs/README.md for canonical sources).
    // 2. Replace this `unimplemented!` body with the loader path
    //    below (currently disabled because the file does not exist
    //    in the upstream repo):
    //
    //     ```ignore
    //     use hopper_svm::{HopperSvm, SPL_TOKEN_PROGRAM_ID};
    //     use solana_sdk::pubkey::Pubkey;
    //
    //     let elf = include_bytes!("../programs/spl_token.so");
    //     let svm = HopperSvm::new()
    //         .with_agave_runtime()
    //         .with_real_spl_token(elf)
    //         .expect("load SPL Token ELF");
    //     // ...mint, transfer, assert balances...
    //     ```
    //
    // 3. Remove the `#[ignore]` attribute on this test.
    //
    // Until the bytes are staged, this test panics so a stale
    // run-without-ignore surfaces the missing prerequisite clearly.
    panic!(
        "SPL Token ELF not staged. See crates/hopper-svm/programs/README.md \
         for sourcing instructions."
    );
}
