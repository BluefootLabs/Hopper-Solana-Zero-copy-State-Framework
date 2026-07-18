//! # GRILLO — manifest layer
//!
//! GRILLO is the ecosystem layer over Hopper's byte-range primitive.
//! Solana's own idl-spec covers instructions / accounts / types / events /
//! errors and STOPS SHORT of byte layouts. Grillo supplies the missing
//! **byte-level mutation contract**: for each instruction, exactly which
//! byte ranges of which accounts it is authorized to write, and — when the
//! program opted into the lamport dimension — which accounts may have their
//! lamports moved.
//!
//! This crate is the *parser + commitment* half. It reads the REAL
//! `hopper.manifest.json` emitted by `hopper compile --emit manifest`
//! (renderer: `hopper_schema::codama::ManifestJson`) into a
//! [`MutationManifest`] and can [commit](MutationManifest::commitment) to
//! that contract with a stable `SHA-256`. The independent *verifier* half —
//! which checks a transaction's actual byte changes against this contract —
//! lives in the sibling `grillo-verifier` crate.
//!
//! The core invariant Grillo makes checkable, honest by construction:
//!
//! > **changed ⊆ acquired ⊆ authorized**
//!
//! where acquired-but-unchanged is LEGAL (access is not modification). This
//! crate supplies the `authorized` set; `grillo-verifier` supplies `acquired`
//! (from the touch map) and `changed` (from pre/post byte snapshots).
//!
//! ```
//! use grillo_manifest::MutationManifest;
//!
//! let json = r#"{
//!     "name": "p", "version": "1.0.0",
//!     "instructions": [
//!         { "name": "pause", "tag": 1, "strictWrites": true,
//!           "writeRanges": [
//!             { "accountIndex": 1, "offset": 114, "size": 1 },
//!             { "accountIndex": 1, "offset": 115, "size": 8 }
//!           ],
//!           "accounts": [
//!             { "name": "admin", "signer": true },
//!             { "name": "config", "writable": true }
//!           ] }
//!     ]
//! }"#;
//! let m = MutationManifest::from_json(json).unwrap();
//! let pause = m.instruction("pause").unwrap();
//! assert!(pause.strict_writes);
//! assert_eq!(pause.authorized.len(), 2);
//! let _commitment: [u8; 32] = m.commitment();
//! ```

mod commitment;
mod effect_v2;
mod manifest;
mod resolve;
mod sha256;

pub use manifest::{
    AccountRole, ArgContract, ArgEncodingContract, InstructionContract, MutationContractView,
    MutationManifest, ParametricRangeContract, ParseError, RangeContract,
};
pub use resolve::{ResolveError, ResolvedInstructionContract, ResolvedSelector};
pub use sha256::sha256;
pub use effect_v2::{
    AccountRoleContractV2, AddressConstraintV2, ContractCompletenessV2, CpiAccountBindingV2,
    CpiEnvelopeV2, CpiPolicyV2, DataPolicyV2, DataRangeV2, DeploymentBindingV2,
    DuplicatePolicyV2, EffectContractV2, EffectContractV2Error, ExecutablePolicyV2,
    InstructionEffectContractV2, LamportPolicyV2, LengthPolicyV2, OwnerPolicyV2,
    OwnerTargetV2, PresencePolicyV2, PrivilegeRequirementV2, RemainingAccountsContractV2,
    RemainingGroupV2, TransitionPolicyV2, EFFECT_ABI_V2,
};
