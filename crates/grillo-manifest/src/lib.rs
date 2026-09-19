//! # Grillo manifest layer
//!
//! Grillo parses Hopper's byte-level mutation contracts. Each instruction can
//! declare authorized data ranges and, when enabled, lamport permissions.
//!
//! This crate is the parser and commitment half. It reads the
//! `hopper.manifest.json` emitted by `hopper compile --emit manifest`
//! (renderer: `hopper_schema::codama::ManifestJson`) into a
//! [`MutationManifest`] and can [commit](MutationManifest::commitment) to
//! that contract with a stable `SHA-256`. The separately runnable verifier
//! checks caller-supplied account snapshots and touch evidence against this
//! contract. Neither crate authenticates or completes the supplied evidence.
//!
//! Within the supplied evidence scope, Grillo checks:
//!
//! > **changed subset acquired subset authorized**
//!
//! where acquired-but-unchanged is legal because access is not modification. This
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

pub mod authority;
mod commitment;
mod effect_v2;
mod manifest;
mod resolve;
mod sha256;

pub use effect_v2::{
    AccountRoleContractV2, AddressConstraintV2, ContractCompletenessV2, CpiAccountBindingV2,
    CpiEnvelopeV2, CpiPolicyV2, DataPolicyV2, DataRangeV2, DeploymentBindingV2, DuplicatePolicyV2,
    EffectContractV2, EffectContractV2Error, ExecutablePolicyV2, InstructionEffectContractV2,
    LamportPolicyV2, LengthPolicyV2, OwnerPolicyV2, OwnerTargetV2, PresencePolicyV2,
    PrivilegeRequirementV2, RemainingAccountsContractV2, RemainingGroupV2, TransitionPolicyV2,
    EFFECT_ABI_V2,
};
pub use manifest::{
    AccountRole, ArgContract, ArgEncodingContract, InstructionContract, MutationContractView,
    MutationManifest, ParametricRangeContract, ParseError, RangeContract,
};
pub use resolve::{ResolveError, ResolvedInstructionContract, ResolvedSelector};
pub use sha256::sha256;
