//! Field behaviors: packageable, parameterized per-field lifecycle
//! plugins (innovation I16).
//!
//! # What this is
//!
//! A protocol can author a reusable behavior once — "this account is a
//! fee vault with a basis-points cap", "this oracle must be fresh",
//! "this counter only increments" — parameterize it per attachment, and
//! attach it to a context field. The planned macro surface (see
//! `docs/design/BEHAVIORS_RFC.md`) is Quasar-parity:
//!
//! ```ignore
//! #[hopper::context(strict_writes)]
//! struct Collect<'info> {
//!     #[account(mut(collected), behavior(fee_vault, max_bps = 30))]
//!     vault: Vault,
//!     ...
//! }
//! ```
//!
//! This module is the **runtime core** the macro lowers to. It works
//! standing alone (hand-wired) today; the macro attachment is the
//! follow-up tracked in the RFC.
//!
//! # Where it surpasses Quasar's `AccountBehavior`
//!
//! Quasar behaviors are side-effect-only hooks. Hopper behaviors are
//! **accountable** to the rest of the framework:
//!
//! 1. **Proof tokens.** A successful `check` returns
//!    [`BehaviorChecked<B>`] (plus a behavior-defined payload), which
//!    composes with the existing [`AccountProof`](crate::proof::AccountProof)
//!    capability chain. Downstream APIs can *require* evidence that a
//!    behavior ran — not merely hope the derive emitted the call.
//! 2. **Write-set contribution.** A behavior declares the byte ranges
//!    its `update`/`exit` phases write ([`HopperBehavior::WRITES`],
//!    field-relative). Under I12 `strict_writes` the macro folds these
//!    into the context's static `WritePolicy`, so plugins *extend* the
//!    declared write surface instead of punching holes in it.
//! 3. **Ledger visibility.** Behavior mutations are expected to go
//!    through the Context segment paths, so they land in the I7 touch
//!    map and the receipt system like any handler write: auditable
//!    plugins, not opaque ones.
//!
//! # Phase model
//!
//! Phases mirror the account lifecycle and are gated by associated
//! consts so generated code only emits calls for the phases a behavior
//! actually uses (dead phases cost nothing — same discipline Quasar
//! uses, kept deliberately for parity of codegen cost):
//!
//! ```text
//! phase   const        runs                          receives
//! ------- ------------ ----------------------------- -----------------
//! check   RUN_CHECK    after load, before handler    &view, &state
//! update  RUN_UPDATE   after check (mut fields)      &view, &mut state
//! exit    RUN_EXIT     epilogue (mut fields)         &view
//! ```

use core::marker::PhantomData;

use crate::account::AccountView;
use crate::error::ProgramError;
use crate::layout::LayoutContract;
use crate::ProgramResult;

/// A field-relative byte range a behavior writes during `update`/`exit`.
///
/// Offsets are relative to the attached account's data start (the same
/// absolute-offset convention the segment primitives use once the macro
/// knows the account); the macro resolves the account index and folds
/// the range into the context's I12 `WritePolicy` as
/// `WriteRange::new(index, offset, size)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BehaviorWrite {
    /// Byte offset within the attached account's data.
    pub offset: u32,
    /// Byte size of the written range.
    pub size: u32,
}

impl BehaviorWrite {
    /// Declare a write range at `offset` spanning `size` bytes.
    #[inline(always)]
    pub const fn new(offset: u32, size: u32) -> Self {
        Self { offset, size }
    }
}

/// A packageable per-field lifecycle plugin over layout type `T`.
///
/// Implement on a unit struct; attach per-field with parameterized args.
/// All phase methods default to no-ops so a behavior overrides only what
/// it needs. See the module docs for the phase table and the surpass
/// properties (proof tokens, write-set contribution, ledger visibility).
pub trait HopperBehavior<T: LayoutContract> {
    /// Per-attachment parameters (the `max_bps = 30` payload). Use a
    /// `'static`-free struct of plain values; the macro builds it from
    /// the attribute arguments.
    type Args;

    /// Behavior-defined proof payload returned by a successful `check`
    /// (use `()` when the token alone is enough). Carried inside
    /// [`BehaviorChecked<B>`].
    type CheckOutput;

    /// Whether `check` runs after load (default on — a behavior that
    /// validates nothing should say so explicitly).
    const RUN_CHECK: bool = true;

    /// Whether `update` runs after validation. Requires a `mut` field.
    const RUN_UPDATE: bool = false;

    /// Whether `exit` runs in the epilogue. Requires a `mut` field.
    const RUN_EXIT: bool = false;

    /// Whether the attached field must be mutable.
    const REQUIRES_MUT: bool = Self::RUN_UPDATE || Self::RUN_EXIT;

    /// Field-relative byte ranges `update`/`exit` write. Folded into the
    /// context's I12 `WritePolicy` under `strict_writes`; an empty slice
    /// means the behavior only reads.
    const WRITES: &'static [BehaviorWrite] = &[];

    /// Validate the loaded state; return the proof payload.
    fn check(
        view: &AccountView<'_>,
        state: &T,
        args: &Self::Args,
    ) -> Result<Self::CheckOutput, ProgramError> {
        let _ = (view, state, args);
        Err(ProgramError::InvalidArgument)
    }

    /// Mutate state after validation (only with `RUN_UPDATE`).
    fn update(view: &AccountView<'_>, state: &mut T, args: &Self::Args) -> ProgramResult {
        let _ = (view, state, args);
        Ok(())
    }

    /// Epilogue hook (only with `RUN_EXIT`).
    fn exit(view: &AccountView<'_>, args: &Self::Args) -> ProgramResult {
        let _ = (view, args);
        Ok(())
    }
}

/// Proof token: behavior `B` ran its `check` phase against an account
/// and succeeded, yielding `B::CheckOutput`.
///
/// Zero-cost beyond the payload; the `B` type parameter is the
/// evidence. APIs that must only ever see behavior-validated accounts
/// take this token (or an [`AccountProof`](crate::proof::AccountProof)
/// composed with it) instead of a bare view.
pub struct BehaviorChecked<B, O> {
    /// The behavior-defined check payload.
    pub output: O,
    _behavior: PhantomData<B>,
}

impl<B, O> BehaviorChecked<B, O> {
    #[inline(always)]
    fn new(output: O) -> Self {
        Self {
            output,
            _behavior: PhantomData,
        }
    }
}

/// Run behavior `B`'s `check` phase against `view`, loading the typed
/// state through the normal validated path, and mint the proof token.
///
/// This is the hand-wired form of what the macro will emit at bind time
/// for each `behavior(...)` attachment. `RUN_CHECK = false` behaviors
/// yield an error here rather than a vacuous proof: a token must mean
/// the check actually ran.
#[inline]
pub fn run_check<B, T>(
    view: &AccountView<'_>,
    args: &B::Args,
) -> Result<BehaviorChecked<B, B::CheckOutput>, ProgramError>
where
    T: LayoutContract + crate::Pod,
    B: HopperBehavior<T>,
{
    if !B::RUN_CHECK {
        return Err(ProgramError::InvalidArgument);
    }
    let state = view.load::<T>()?;
    let output = B::check(view, &state, args)?;
    Ok(BehaviorChecked::new(output))
}

/// Run behavior `B`'s `update` phase through the typed mutable path.
///
/// Requires the proof token from [`run_check`] — an update cannot run
/// against unvalidated state. The macro enforces the same ordering.
#[inline]
pub fn run_update<B, T>(
    view: &AccountView<'_>,
    args: &B::Args,
    _proof: &BehaviorChecked<B, B::CheckOutput>,
) -> ProgramResult
where
    T: LayoutContract + crate::Pod,
    B: HopperBehavior<T>,
{
    if !B::RUN_UPDATE {
        return Ok(());
    }
    let mut state = view.load_mut::<T>()?;
    B::update(view, &mut state, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::HopperHeader;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    // A vault whose `collected_bps` must stay under a per-attachment cap.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FeeVault {
        collected_bps: [u8; 2],
        _pad: [u8; 6],
    }
    // SAFETY: repr(C), byte-array fields — align 1, no padding, all
    // patterns valid.
    unsafe impl crate::Zeroable for FeeVault {}
    // SAFETY: as above.
    unsafe impl crate::Pod for FeeVault {}
    impl crate::field_map::FieldMap for FeeVault {
        const FIELDS: &'static [crate::field_map::FieldInfo] =
            &[crate::field_map::FieldInfo::new(
                "collected_bps",
                HopperHeader::SIZE,
                2,
            )];
    }
    impl LayoutContract for FeeVault {
        const DISC: u8 = 42;
        const VERSION: u8 = 1;
        const LAYOUT_ID: [u8; 8] = [0x42; 8];
        const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
    }

    /// The packageable plugin: "fee take must not exceed `max_bps`".
    struct FeeCap;
    struct FeeCapArgs {
        max_bps: u16,
    }
    impl HopperBehavior<FeeVault> for FeeCap {
        type Args = FeeCapArgs;
        /// Check returns the observed bps so downstream code can use it
        /// without re-reading state.
        type CheckOutput = u16;
        const WRITES: &'static [BehaviorWrite] = &[BehaviorWrite::new(
            HopperHeader::SIZE as u32,
            2, // collected_bps
        )];
        const RUN_UPDATE: bool = true;

        fn check(
            _view: &AccountView<'_>,
            state: &FeeVault,
            args: &Self::Args,
        ) -> Result<u16, ProgramError> {
            let bps = u16::from_le_bytes(state.collected_bps);
            if bps > args.max_bps {
                return Err(ProgramError::InvalidAccountData);
            }
            Ok(bps)
        }

        fn update(
            _view: &AccountView<'_>,
            state: &mut FeeVault,
            args: &Self::Args,
        ) -> ProgramResult {
            // Clamp to the cap — a deliberate, declared write to the
            // `collected_bps` range in `WRITES`.
            let bps = u16::from_le_bytes(state.collected_bps).min(args.max_bps);
            state.collected_bps = bps.to_le_bytes();
            Ok(())
        }
    }

    fn make_vault(bps: u16) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let data_len = FeeVault::SIZE;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: backing is sized for header + data and outlives the view.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([1; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports: 1,
                data_len: data_len as u64,
            });
        }
        // SAFETY: raw points at a fully initialized RuntimeAccount.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let view = AccountView::from_backend(backend);
        {
            let mut d = view.try_borrow_mut().unwrap();
            crate::layout::init_header::<FeeVault>(&mut d).unwrap();
            d[HopperHeader::SIZE..HopperHeader::SIZE + 2].copy_from_slice(&bps.to_le_bytes());
        }
        (backing, view)
    }

    #[test]
    fn check_mints_proof_with_payload_and_rejects_violations() {
        let (_b, vault) = make_vault(25);
        let args = FeeCapArgs { max_bps: 30 };

        let proof = run_check::<FeeCap, FeeVault>(&vault, &args).unwrap();
        assert_eq!(proof.output, 25);

        // Over the cap: no proof is minted.
        let (_b2, hot) = make_vault(31);
        assert!(run_check::<FeeCap, FeeVault>(&hot, &args).is_err());
    }

    #[test]
    fn update_requires_the_proof_and_applies_declared_writes() {
        let (_b, vault) = make_vault(30);
        let args = FeeCapArgs { max_bps: 30 };

        // The signature makes ordering structural: update takes the
        // token check minted — there is no way to call it first.
        let proof = run_check::<FeeCap, FeeVault>(&vault, &args).unwrap();
        run_update::<FeeCap, FeeVault>(&vault, &args, &proof).unwrap();

        let state = vault.load::<FeeVault>().unwrap();
        assert_eq!(u16::from_le_bytes(state.collected_bps), 30);
    }

    #[test]
    fn write_contribution_is_declared_for_strict_writes_folding() {
        // The macro folds these into the context's static WritePolicy;
        // pin the shape the FeeCap plugin declares.
        assert_eq!(
            <FeeCap as HopperBehavior<FeeVault>>::WRITES,
            &[BehaviorWrite::new(HopperHeader::SIZE as u32, 2)]
        );
        assert!(<FeeCap as HopperBehavior<FeeVault>>::REQUIRES_MUT);
    }
}
