//! # GRILLO — verifier
//!
//! The INDEPENDENT half of GRILLO: given a transaction's actual byte
//! changes, a program's decoded touch map, and the instruction's published
//! mutation contract (from [`grillo_manifest`]), decide whether the
//! instruction honored its contract.
//!
//! The verdict enforces one invariant, honest by construction:
//!
//! > **changed ⊆ acquired ⊆ authorized**
//!
//! - `changed`    comes from pre/post account byte [snapshots](AccountDelta).
//! - `acquired`   comes from the [touch map](TouchMap) the instruction
//!   emitted (its own self-report of what it mutated).
//! - `authorized` comes from the manifest's `writeRanges`
//!   ([`grillo_manifest::RangeContract`]).
//!
//! Acquired-but-unchanged is LEGAL — access is not modification — and is
//! surfaced as a note on a scoped [PASS](Verdict::Pass), never a violation.
//!
//! Crucially, the verifier is *independent*: it re-derives `changed` from
//! raw bytes rather than trusting the program's self-report. A program that
//! lies in its touch map (claims a write it did not make, or omits one it
//! did) is caught — an omitted real write shows up as an
//! [`UntrackedWrite`](Violation::UntrackedWrite); an over-claimed write
//! outside the authorized set as an
//! [`UnauthorizedAcquisition`](Violation::UnauthorizedAcquisition). And a
//! [partial](TouchMap::is_partial) map yields
//! [`INCONCLUSIVE`](InconclusiveReason::PartialTouchMap), never a false PASS.
//!
//! ```
//! use grillo_manifest::MutationManifest;
//! use grillo_verifier::{verify, AccountDelta, TouchMap, TouchRecord, Verdict};
//!
//! // A `pause` that may only write config byte 114 (paused).
//! let manifest = MutationManifest::from_json(r#"{
//!     "name": "p", "version": "1.0.0",
//!     "instructions": [
//!         { "name": "pause", "tag": 1, "strictWrites": true,
//!           "writeRanges": [ { "accountIndex": 1, "offset": 114, "size": 1 } ],
//!           "accounts": [ { "name": "admin" }, { "name": "config" } ] }
//!     ]
//! }"#).unwrap();
//! let pause = manifest.instruction("pause").unwrap();
//!
//! let pre = vec![0u8; 200];
//! let mut post = pre.clone();
//! post[114] = 1; // set `paused`
//!
//! let map = TouchMap {
//!     overflowed: false, skipped: false,
//!     records: vec![TouchRecord { slot: 1, offset: 114, size: 1, write: true }],
//! };
//! let verdict = verify(pause, &[AccountDelta::new(1, &pre, &post)], &map);
//! assert!(verdict.is_pass());
//! ```

mod touch_map;
mod verify;
mod frame_v2;
mod verify_v2;

pub use touch_map::{
    decode_touch_map, DecodeError, TouchMap, TouchRecord, MAX_TOUCH_RECORDS,
    TOUCH_MAP_FLAG_OVERFLOWED, TOUCH_MAP_FLAG_SKIPPED, TOUCH_MAP_HEADER_LEN, TOUCH_MAP_MAGIC,
    TOUCH_MAP_RECORD_LEN, TOUCH_MAP_VERSION,
};
pub use verify::{
    verify, verify_invocation, verify_resolved, AccountDelta, InconclusiveReason, PassEvidence,
    Verdict, Violation,
};
pub use frame_v2::{
    bind_invocation_v2, AccountStateV2, AccountTransitionV2, BindErrorV2, BoundInvocationV2,
    DeploymentIdentityV2, EvidenceCompletenessV2, EvidenceProvenanceV2, InvocationAccountRefV2,
    InvocationFrameV2, InvocationOutcomeV2, NetworkIdentityV2, ObservationBoundaryV2,
    ResolvedAccountRoleV2, TouchEvidenceV2, TransactionIdentityV2, UnverifiedAuthenticityV2,
};
pub use verify_v2::{
    verify_bound_invocation_v2, EffectVerdictV2, InconclusiveReasonV2, PassEvidenceV2,
    ViolationV2,
};

// Re-exported so downstream users get the contract types without a second
// `use` of the sibling crate.
pub use grillo_manifest::{
    InstructionContract, MutationManifest, ParametricRangeContract, RangeContract, ResolveError,
    ResolvedInstructionContract,
};
pub use grillo_manifest::{
    AccountRoleContractV2, AddressConstraintV2, ContractCompletenessV2, CpiAccountBindingV2,
    CpiEnvelopeV2, CpiPolicyV2, DataPolicyV2, DataRangeV2, DeploymentBindingV2,
    DuplicatePolicyV2, EffectContractV2, EffectContractV2Error, ExecutablePolicyV2,
    InstructionEffectContractV2, LamportPolicyV2, LengthPolicyV2, OwnerPolicyV2,
    OwnerTargetV2, PresencePolicyV2, PrivilegeRequirementV2, RemainingAccountsContractV2,
    RemainingGroupV2, TransitionPolicyV2, EFFECT_ABI_V2,
};
