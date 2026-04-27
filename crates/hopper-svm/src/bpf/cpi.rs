//! Cross-Program Invocation — Phase 2.1 step 10.
//!
//! When a BPF program calls `sol_invoke_signed_c` it's saying:
//! "build an inner instruction targeting program X with these
//! accounts and this data; here are the signer-seed sets that
//! prove the calling program owns the PDAs in the account list;
//! recursively dispatch and return when the inner call finishes."
//!
//! ## Wire format (`sol_invoke_signed_c`)
//!
//! ```text
//! sol_invoke_signed_c(
//!     instruction_addr: u64,    // → SolInstruction
//!     accounts_addr: u64,       // → [SolAccountInfo; n]
//!     accounts_len: u64,
//!     signer_seeds_addr: u64,   // → [SolSignerSeeds; m]
//!     signer_seeds_len: u64,
//! ) -> u64
//!
//! struct SolInstruction {       // 56 bytes
//!     program_id_addr: u64,     //  8  → 32 bytes pubkey
//!     accounts_addr: u64,       //  8  → [SolAccountMeta; k]
//!     accounts_len: u64,        //  8
//!     data_addr: u64,           //  8  → arbitrary bytes
//!     data_len: u64,            //  8
//! }                             // (40 + 16 padding for alignment)
//!
//! struct SolAccountMeta {       // 16 bytes (with padding)
//!     pubkey_addr: u64,         //  8
//!     is_writable: u8,          //  1
//!     is_signer: u8,            //  1
//!     // 6 bytes padding
//! }
//!
//! struct SolAccountInfo {       // 56 bytes
//!     key_addr: u64,            //  8
//!     lamports_addr: u64,       //  8 (mutable u64 in the parameter buffer)
//!     data_len: u64,            //  8
//!     data_addr: u64,           //  8 (mutable bytes in the parameter buffer)
//!     owner_addr: u64,          //  8
//!     rent_epoch: u64,          //  8
//!     is_signer: u8,
//!     is_writable: u8,
//!     executable: u8,
//!     // 5 bytes padding
//! }
//!
//! struct SolSignerSeeds {       // 16 bytes
//!     addr: u64,                // → [SolSignerSeed; q]
//!     len: u64,
//! }
//!
//! struct SolSignerSeed {        // 16 bytes
//!     addr: u64,                // → arbitrary bytes
//!     len: u64,
//! }
//! ```
//!
//! ## Phase 2.1 status
//!
//! - **`sol_invoke_signed_c`** — fully implemented: parse the
//!   inner instruction, verify signer seeds via PDA derivation
//!   against the calling program's ID, recursively dispatch
//!   through the harness, and write back account-state
//!   mutations through the SolAccountInfo pointers.
//! - **`sol_invoke_signed_rust`** — registered but returns a
//!   structured "Rust-ABI CPI lands in 2.2" error. The Rust
//!   shape passes `AccountInfo` through `Rc<RefCell<…>>`
//!   wrappers whose memory layout is a Rust-internal that
//!   shifts between toolchain versions; deferring it keeps
//!   2.1 stable.
//!
//! Programs that use the standard `solana_program::program::invoke_signed`
//! call into the Rust ABI by default. They can use the C path
//! by calling `solana_program::program::invoke_signed_unchecked`
//! or by switching to a `sol_invoke_signed_c`-emitting client.

use crate::account::KeyedAccount;
use crate::bpf::context::{BpfContext, MAX_CPI_DEPTH};
use crate::bpf::syscalls::{do_sol_create_program_address, PdaError};
use crate::engine::ExecutionOutcome;
use crate::error::HopperSvmError;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

/// CPI per-call CU base cost. Matches mainnet's `invoke_units`.
pub const SOL_INVOKE_SIGNED_CU: u64 = 1_000;

/// Maximum number of signer-seed sets a single CPI may carry.
/// Mainnet caps at 16; we mirror.
pub const MAX_SIGNERS: usize = 16;

/// Wire-error codes — non-zero return values that the syscall
/// hands back to the calling program. Match upstream as closely
/// as possible so a port from mainnet sees identical failure
/// modes.
pub mod cpi_err {
    /// Generic CPI error — failed to parse the instruction,
    /// inner call returned a non-success result, etc. The
    /// engine will have logged structured details in the log
    /// transcript.
    pub const FAILED: u64 = 1;
    /// Recursion would exceed [`super::MAX_CPI_DEPTH`].
    pub const DEPTH_EXCEEDED: u64 = 2;
    /// One of the signer-seed sets did not derive to a PDA
    /// present in the inner instruction's account list (the
    /// calling program lied about which PDAs it owns).
    pub const SIGNER_SEEDS_INVALID: u64 = 3;
    /// Too many signer-seed sets ([`super::MAX_SIGNERS`]).
    pub const TOO_MANY_SIGNERS: u64 = 4;
}

/// Parsed inner-instruction shape — what the harness needs to
/// recursively dispatch. The accounts list is in the order the
/// inner instruction wants; each one is paired with its
/// observed pre-state (read from the SolAccountInfo pointers
/// the caller supplied).
#[derive(Debug, Clone)]
pub struct ParsedCpi {
    /// Program ID of the inner instruction.
    pub program_id: Pubkey,
    /// AccountMeta list (pubkey + signer/writable flags) for the
    /// inner instruction, in the order the inner program will
    /// see them.
    pub metas: Vec<AccountMeta>,
    /// Instruction data bytes for the inner call.
    pub data: Vec<u8>,
    /// Snapshot of the calling program's account state at the
    /// moment of the CPI. Each entry's address corresponds to
    /// one of the metas (in the same order).
    pub accounts: Vec<KeyedAccount>,
    /// Signer-seed sets — one slice-of-slices per signer
    /// authority the calling program claims to control. Each
    /// inner element is a single PDA seed.
    pub signer_seeds: Vec<Vec<Vec<u8>>>,
}

/// Verify that every signer in the inner instruction's account
/// list is either:
///
/// - a signer that was already a signer in the outer
///   instruction (the calling program is just forwarding a
///   real signature), or
/// - a PDA whose seeds + the calling program's ID derive to
///   the signer's pubkey (the calling program is signing for
///   a PDA it owns).
///
/// Returns `Ok` on success, [`cpi_err::SIGNER_SEEDS_INVALID`]
/// if any signer is not satisfied by either path.
pub fn verify_signer_seeds(
    ctx: &mut BpfContext,
    parsed: &ParsedCpi,
    caller_program_id: &Pubkey,
    outer_signers: &[Pubkey],
) -> Result<(), u64> {
    if parsed.signer_seeds.len() > MAX_SIGNERS {
        return Err(cpi_err::TOO_MANY_SIGNERS);
    }
    // Each signer in the inner instruction must be authorised.
    for meta in &parsed.metas {
        if !meta.is_signer {
            continue;
        }
        // Path 1: the same pubkey was a signer in the outer
        // instruction — the calling program is forwarding a real
        // signature.
        if outer_signers.contains(&meta.pubkey) {
            continue;
        }
        // Path 2: one of the signer-seed sets derives to this
        // pubkey under the calling program's ID. We try each
        // seed set; if any derives to `meta.pubkey`, the signer
        // is satisfied.
        let caller_id_bytes = caller_program_id.to_bytes();
        let mut authorised = false;
        for seed_set in &parsed.signer_seeds {
            let seed_refs: Vec<&[u8]> = seed_set.iter().map(|s| s.as_slice()).collect();
            if let Ok(pda) = do_sol_create_program_address(ctx, &seed_refs, &caller_id_bytes) {
                if pda == meta.pubkey.to_bytes() {
                    authorised = true;
                    break;
                }
            }
            // PdaError::OnCurve means this seed set didn't
            // produce a valid PDA at all; just try the next set.
        }
        if !authorised {
            return Err(cpi_err::SIGNER_SEEDS_INVALID);
        }
    }
    Ok(())
}

/// Dispatch a parsed CPI through the dispatcher closure on the
/// context. Charges the per-CPI CU baseline, increments depth,
/// runs the recursive call (the inner instruction's logs append
/// directly to the outer context's transcript so the test sees
/// one coherent log buffer across the call boundary), surfaces
/// depth-exceeded as a structured wire error.
pub fn dispatch_cpi(ctx: &mut BpfContext, parsed: ParsedCpi) -> Result<ExecutionOutcome, u64> {
    if ctx.remaining_units < SOL_INVOKE_SIGNED_CU {
        return Err(cpi_err::FAILED);
    }
    ctx.remaining_units -= SOL_INVOKE_SIGNED_CU;
    if ctx.cpi_depth + 1 > MAX_CPI_DEPTH {
        return Err(cpi_err::DEPTH_EXCEEDED);
    }
    // Clone the Arc out of the context so we don't hold a borrow
    // on `ctx` during the dispatcher call. That lets us pass
    // `&mut ctx.logs` through to the closure cleanly.
    let dispatcher = match ctx.cpi_dispatcher.as_ref() {
        Some(d) => d.clone(),
        // No dispatcher attached — Phase 1 path or test harness
        // that didn't configure CPI. Surface as failed.
        None => return Err(cpi_err::FAILED),
    };
    let inner_ix = Instruction {
        program_id: parsed.program_id,
        accounts: parsed.metas,
        data: parsed.data,
    };
    // Record the CPI on the context BEFORE the recursive call,
    // capturing the stack height as the OUTER program's depth + 1
    // (the depth at which this inner call is running). The
    // engine takes the recorded vector at unwind time and lifts
    // it into `ExecutionOutcome::inner_instructions`.
    let inner_record = crate::engine::InnerInstruction {
        program_id: inner_ix.program_id,
        accounts: inner_ix.accounts.clone(),
        data: inner_ix.data.clone(),
        stack_height: ctx.cpi_depth + 1,
    };
    // Run the inner instruction. The dispatcher will append its
    // own invoke/success framing to `ctx.logs` so the outer
    // transcript reads continuous across the depth boundary.
    let outcome = dispatcher(&inner_ix, parsed.accounts, &mut ctx.logs);
    // Record the inner CPI even on failure — the outer program
    // attempted the call, and snapshot tests may want to see
    // that. Order: parent CPIs come BEFORE their children
    // because we record at the call site (which is parent
    // context) before recursing.
    ctx.inner_instructions.push(inner_record);
    // Also fold the inner outcome's own inner_instructions
    // (CPIs the inner program made) into ours. This produces a
    // flat ordered list of all CPIs in dispatch order, matching
    // what mainnet records.
    ctx.inner_instructions
        .extend(outcome.inner_instructions.iter().cloned());
    if let Some(err) = &outcome.error {
        ctx.logs.line(format!("CPI failed: {err}"));
        return Err(cpi_err::FAILED);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysvar::Sysvars;
    use solana_sdk::pubkey::Pubkey;

    fn ctx_with_units(units: u64) -> BpfContext {
        BpfContext::new_with_sysvars(Pubkey::new_unique(), units, Sysvars::default())
    }

    /// Signer-seed verification accepts a signer that was already
    /// a signer in the outer instruction (forwarding case). No
    /// PDA derivation needed.
    #[test]
    fn verify_accepts_outer_signer_passthrough() {
        let signer = Pubkey::new_unique();
        let parsed = ParsedCpi {
            program_id: Pubkey::new_unique(),
            metas: vec![AccountMeta::new(signer, true)],
            data: vec![],
            accounts: vec![],
            signer_seeds: vec![],
        };
        let caller = Pubkey::new_unique();
        let mut ctx = ctx_with_units(100_000);
        let r = verify_signer_seeds(&mut ctx, &parsed, &caller, &[signer]);
        assert!(r.is_ok(), "outer signer should pass through");
    }

    /// Signer-seed verification rejects a signer that's neither
    /// an outer signer nor a PDA derived from the caller program.
    #[test]
    fn verify_rejects_unauthorized_signer() {
        let phantom = Pubkey::new_unique();
        let parsed = ParsedCpi {
            program_id: Pubkey::new_unique(),
            metas: vec![AccountMeta::new(phantom, true)],
            data: vec![],
            accounts: vec![],
            signer_seeds: vec![],
        };
        let caller = Pubkey::new_unique();
        let mut ctx = ctx_with_units(100_000);
        let err = verify_signer_seeds(&mut ctx, &parsed, &caller, &[]).unwrap_err();
        assert_eq!(err, cpi_err::SIGNER_SEEDS_INVALID);
    }

    /// `MAX_SIGNERS` enforcement — 17 signer-seed sets is a
    /// hard reject before any PDA derivation runs.
    #[test]
    fn verify_rejects_too_many_signer_sets() {
        let signer = Pubkey::new_unique();
        let signer_seeds: Vec<Vec<Vec<u8>>> =
            (0..MAX_SIGNERS + 1).map(|_| vec![vec![1u8]]).collect();
        let parsed = ParsedCpi {
            program_id: Pubkey::new_unique(),
            metas: vec![AccountMeta::new(signer, true)],
            data: vec![],
            accounts: vec![],
            signer_seeds,
        };
        let caller = Pubkey::new_unique();
        let mut ctx = ctx_with_units(100_000);
        let err = verify_signer_seeds(&mut ctx, &parsed, &caller, &[]).unwrap_err();
        assert_eq!(err, cpi_err::TOO_MANY_SIGNERS);
    }

    /// PDA-derived signer is accepted when the seeds + caller ID
    /// derive to the signer pubkey. Round-trip through
    /// `do_sol_create_program_address` with a known seed set.
    #[test]
    fn verify_accepts_pda_signer() {
        let caller = Pubkey::new_unique();
        let mut ctx = ctx_with_units(100_000);
        let seeds: Vec<Vec<u8>> = vec![b"vault".to_vec(), vec![1, 2, 3]];
        // Find a bump that produces a valid (off-curve) PDA.
        let mut pda = None;
        for bump in (0u8..=255).rev() {
            let mut seed_refs: Vec<&[u8]> = seeds.iter().map(|s| s.as_slice()).collect();
            let bump_arr = [bump];
            seed_refs.push(&bump_arr);
            if let Ok(p) = do_sol_create_program_address(&mut ctx, &seed_refs, &caller.to_bytes()) {
                let mut full_seeds = seeds.clone();
                full_seeds.push(vec![bump]);
                pda = Some((Pubkey::new_from_array(p), full_seeds));
                break;
            }
        }
        let (pda_key, full_seeds) = pda.expect("should find a valid bump");

        let parsed = ParsedCpi {
            program_id: Pubkey::new_unique(),
            metas: vec![AccountMeta::new(pda_key, true)],
            data: vec![],
            accounts: vec![],
            signer_seeds: vec![full_seeds],
        };
        let mut ctx2 = ctx_with_units(100_000);
        let r = verify_signer_seeds(&mut ctx2, &parsed, &caller, &[]);
        assert!(r.is_ok(), "PDA signer should authorise");
    }

    /// `dispatch_cpi` enforces depth: at depth `MAX_CPI_DEPTH`,
    /// any further CPI returns `DEPTH_EXCEEDED`.
    #[test]
    fn dispatch_rejects_depth_exceeded() {
        let mut ctx = ctx_with_units(100_000);
        ctx.cpi_depth = MAX_CPI_DEPTH; // already at max
        let parsed = ParsedCpi {
            program_id: Pubkey::new_unique(),
            metas: vec![],
            data: vec![],
            accounts: vec![],
            signer_seeds: vec![],
        };
        let err = dispatch_cpi(&mut ctx, parsed).unwrap_err();
        assert_eq!(err, cpi_err::DEPTH_EXCEEDED);
    }

    /// `dispatch_cpi` returns `FAILED` when no dispatcher is
    /// configured (Phase 1 path or unconfigured test).
    #[test]
    fn dispatch_fails_without_configured_dispatcher() {
        let mut ctx = ctx_with_units(100_000);
        let parsed = ParsedCpi {
            program_id: Pubkey::new_unique(),
            metas: vec![],
            data: vec![],
            accounts: vec![],
            signer_seeds: vec![],
        };
        let err = dispatch_cpi(&mut ctx, parsed).unwrap_err();
        assert_eq!(err, cpi_err::FAILED);
    }

    /// `dispatch_cpi` records each inner call on the context's
    /// `inner_instructions` vector. Pin the recording: a stub
    /// dispatcher that succeeds should leave one entry on
    /// ctx.inner_instructions with the right program_id, metas,
    /// data, and stack_height = ctx.cpi_depth + 1.
    #[test]
    fn dispatch_records_inner_instruction() {
        let mut ctx = ctx_with_units(100_000);
        let inner_pid = Pubkey::new_unique();
        let signer = Pubkey::new_unique();
        let dispatcher: super::super::context::CpiDispatcher =
            std::sync::Arc::new(move |_ix, _accounts, _logs| ExecutionOutcome {
                resulting_accounts: vec![],
                compute_units_consumed: 0,
                return_data: vec![],
                inner_instructions: vec![],
                execution_time_us: 0,
                error: None,
            });
        ctx.cpi_dispatcher = Some(dispatcher);
        let parsed = ParsedCpi {
            program_id: inner_pid,
            metas: vec![AccountMeta::new(signer, true)],
            data: vec![1, 2, 3],
            accounts: vec![],
            signer_seeds: vec![],
        };
        // Outer signer in scope for the verify_signer_seeds call.
        // Skip verify in this test by going straight to dispatch.
        let _ = dispatch_cpi(&mut ctx, parsed).expect("ok");
        assert_eq!(ctx.inner_instructions.len(), 1);
        let rec = &ctx.inner_instructions[0];
        assert_eq!(rec.program_id, inner_pid);
        assert_eq!(rec.accounts.len(), 1);
        assert_eq!(rec.accounts[0].pubkey, signer);
        assert_eq!(rec.data, vec![1, 2, 3]);
        // ctx.cpi_depth defaults to 1 (outermost), so the inner
        // call ran at depth 2.
        assert_eq!(rec.stack_height, 2);
    }

    /// Nested CPIs flatten in dispatch order. A dispatcher that
    /// returns its OWN inner_instructions (simulating a nested
    /// call) should produce a flat ordered vector: parent
    /// before child.
    #[test]
    fn nested_cpis_flatten_in_dispatch_order() {
        let mut ctx = ctx_with_units(1_000_000);
        let parent_pid = Pubkey::new_unique();
        let grandchild_pid = Pubkey::new_unique();
        // The dispatcher returns an outcome whose inner_instructions
        // already contains a "grandchild" record (simulating that
        // the parent program made its own CPI which we tracked at
        // depth 3).
        let dispatcher: super::super::context::CpiDispatcher =
            std::sync::Arc::new(move |_ix, _accounts, _logs| ExecutionOutcome {
                resulting_accounts: vec![],
                compute_units_consumed: 0,
                return_data: vec![],
                inner_instructions: vec![crate::engine::InnerInstruction {
                    program_id: grandchild_pid,
                    accounts: vec![],
                    data: vec![0xFF],
                    stack_height: 3,
                }],
                execution_time_us: 0,
                error: None,
            });
        ctx.cpi_dispatcher = Some(dispatcher);
        let parsed = ParsedCpi {
            program_id: parent_pid,
            metas: vec![],
            data: vec![],
            accounts: vec![],
            signer_seeds: vec![],
        };
        let _ = dispatch_cpi(&mut ctx, parsed).expect("ok");
        assert_eq!(ctx.inner_instructions.len(), 2);
        // Parent first.
        assert_eq!(ctx.inner_instructions[0].program_id, parent_pid);
        assert_eq!(ctx.inner_instructions[0].stack_height, 2);
        // Grandchild second.
        assert_eq!(ctx.inner_instructions[1].program_id, grandchild_pid);
        assert_eq!(ctx.inner_instructions[1].stack_height, 3);
    }

    /// Inner-instruction logs append to the outer context's
    /// transcript. Pin the cross-boundary log threading: a stub
    /// dispatcher that calls `logs.program_log("inner")` on its
    /// log argument should leave the outer ctx.logs containing
    /// that line.
    #[test]
    fn dispatcher_logs_thread_into_outer_transcript() {
        let mut ctx = ctx_with_units(100_000);
        // Pre-seed the outer transcript so we can verify
        // append-after.
        ctx.logs.program_log("outer line");
        // Stub dispatcher: writes a marker line to whatever
        // `logs` it's handed.
        let dispatcher: super::super::context::CpiDispatcher =
            std::sync::Arc::new(|_ix, _accounts, logs: &mut crate::log::LogCapture| {
                logs.program_log("inner line");
                ExecutionOutcome {
                    resulting_accounts: vec![],
                    compute_units_consumed: 0,
                    return_data: vec![],
                    inner_instructions: Vec::new(),
                    execution_time_us: 0,
                    error: None,
                }
            });
        ctx.cpi_dispatcher = Some(dispatcher);
        let parsed = ParsedCpi {
            program_id: Pubkey::new_unique(),
            metas: vec![],
            data: vec![],
            accounts: vec![],
            signer_seeds: vec![],
        };
        let _ = dispatch_cpi(&mut ctx, parsed).expect("ok");
        let lines = ctx.logs.lines();
        // Outer line first, inner line right after — one
        // continuous transcript.
        assert!(
            lines.iter().any(|l| l == "Program log: outer line"),
            "outer line missing: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "Program log: inner line"),
            "inner line missing — log threading broken: {lines:?}"
        );
    }
}
