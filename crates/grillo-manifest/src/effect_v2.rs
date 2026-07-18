//! Strict, explicitly-versioned Effect ABI v0.2 contract model.
//!
//! This module intentionally does not extend the v0.1 mutation-manifest
//! structs.  A v0.2 consumer must opt into this parser and every authoritative
//! field is required: unknown fields and missing completeness/transition
//! dimensions are errors rather than forward-compatible guesses.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::sha256;

/// Exact Effect ABI version accepted by this module.
pub const EFFECT_ABI_V2: &str = "0.2";

/// A complete program effect contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectContractV2 {
    /// Must be exactly `"0.2"`.
    #[serde(rename = "effectAbiVersion")]
    pub effect_abi_version: String,
    /// Build/deployment identity this contract is valid for.
    pub deployment: DeploymentBindingV2,
    /// Instruction contracts. Discriminators must be globally prefix-free.
    pub instructions: Vec<InstructionEffectContractV2>,
}

/// How a portable contract binds to deployed executable code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum DeploymentBindingV2 {
    /// Pin a particular address, loader, and deployed executable digest.
    ExactDeployment {
        #[serde(rename = "programId")]
        program_id: [u8; 32],
        #[serde(rename = "loaderId")]
        loader_id: [u8; 32],
        #[serde(rename = "executableDigest")]
        executable_digest: [u8; 32],
    },
    /// Permit any address only when its loader-canonical executable bytes
    /// match this build artifact digest.
    ExactArtifact {
        #[serde(rename = "executableDigest")]
        executable_digest: [u8; 32],
    },
}

impl DeploymentBindingV2 {
    /// Pinned executable digest under either binding mode.
    pub fn executable_digest(&self) -> [u8; 32] {
        match self {
            Self::ExactDeployment {
                executable_digest, ..
            }
            | Self::ExactArtifact { executable_digest } => *executable_digest,
        }
    }
}

/// One fully identified instruction contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionEffectContractV2 {
    pub name: String,
    /// Exact dispatcher prefix, not merely its first byte.
    #[serde(rename = "discriminatorBytes")]
    pub discriminator: Vec<u8>,
    /// Explicit authoritative-dimension declaration.
    pub completeness: ContractCompletenessV2,
    /// Fixed positional roles.
    pub accounts: Vec<AccountRoleContractV2>,
    /// Deterministic grammar for the variadic suffix.
    #[serde(rename = "remainingAccounts")]
    pub remaining_accounts: RemainingAccountsContractV2,
    /// Complete nested-call envelope.
    pub cpi: CpiPolicyV2,
}

/// Dimensions for which the producer claims the contract is complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractCompletenessV2 {
    pub accounts: bool,
    pub data: bool,
    pub lamports: bool,
    pub owner: bool,
    #[serde(rename = "dataLength")]
    pub data_length: bool,
    pub presence: bool,
    pub executable: bool,
    pub cpi: bool,
}

/// One account role, reused by fixed and remaining-account grammar entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountRoleContractV2 {
    pub name: String,
    pub signer: PrivilegeRequirementV2,
    pub writable: PrivilegeRequirementV2,
    pub address: AddressConstraintV2,
    pub transition: TransitionPolicyV2,
}

/// Required/forbidden/optional effective privilege.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivilegeRequirementV2 {
    Required,
    Forbidden,
    Allowed,
}

/// Address identity for one role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AddressConstraintV2 {
    Any,
    Exact { address: [u8; 32] },
    /// The executing program address from the concrete frame.
    ProgramId,
}

/// All state-transition dimensions for one account role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionPolicyV2 {
    pub data: DataPolicyV2,
    pub lamports: LamportPolicyV2,
    pub owner: OwnerPolicyV2,
    #[serde(rename = "dataLength")]
    pub data_length: LengthPolicyV2,
    pub presence: PresencePolicyV2,
    pub executable: ExecutablePolicyV2,
}

/// Byte writes permitted within data that exists after the invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataPolicyV2 {
    pub ranges: Vec<DataRangeV2>,
}

/// Half-open data range `[offset, offset + size)` within one role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRangeV2 {
    pub offset: u32,
    pub size: u32,
}

impl DataRangeV2 {
    pub fn end(self) -> u64 {
        self.offset as u64 + self.size as u64
    }

    pub fn contains(self, offset: u64) -> bool {
        self.offset as u64 <= offset && offset < self.end()
    }
}

/// Permitted balance direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LamportPolicyV2 {
    Preserve,
    MayChange,
    DebitOnly,
    CreditOnly,
}

/// Owner transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum OwnerPolicyV2 {
    Preserve,
    SetTo { target: OwnerTargetV2 },
    MayChange,
}

/// Resolved owner target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum OwnerTargetV2 {
    Exact { address: [u8; 32] },
    ProgramId,
    SystemProgram,
}

/// Data-length transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum LengthPolicyV2 {
    Preserve,
    Exact { length: u32 },
    Range { min: u32, max: u32 },
    MayChange,
}

/// Logical account presence transition. A close tombstone can be represented
/// as a present zeroed/System-owned post-state; it is not inferred as absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresencePolicyV2 {
    Preserve,
    MustRemainPresent,
    MustCreate,
    MayCreate,
    MustClose,
    MayClose,
}

/// Executable-bit transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ExecutablePolicyV2 {
    Preserve,
    Set { value: bool },
    MayChange,
}

/// Deterministic account-suffix grammar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemainingAccountsContractV2 {
    pub complete: bool,
    /// A complete first-slice grammar never accepts unparsed trailing metas.
    #[serde(rename = "allowTrailing")]
    pub allow_trailing: bool,
    #[serde(rename = "duplicatePolicy")]
    pub duplicate_policy: DuplicatePolicyV2,
    pub groups: Vec<RemainingGroupV2>,
}

/// One fixed-width role group repeated a statically bounded number of times.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemainingGroupV2 {
    pub name: String,
    pub repeat: u16,
    pub roles: Vec<AccountRoleContractV2>,
}

/// Alias policy for the complete fixed + remaining account list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DuplicatePolicyV2 {
    DenyAll,
    DenyWritable,
    Allow,
}

/// Complete CPI policy for an instruction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CpiPolicyV2 {
    Forbidden,
    Declared {
        complete: bool,
        calls: Vec<CpiEnvelopeV2>,
    },
}

/// One declared child invocation shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpiEnvelopeV2 {
    pub id: String,
    #[serde(rename = "programId")]
    pub program_id: [u8; 32],
    #[serde(rename = "loaderId")]
    pub loader_id: [u8; 32],
    #[serde(rename = "executableDigest")]
    pub executable_digest: [u8; 32],
    #[serde(rename = "discriminatorBytes")]
    pub discriminator: Vec<u8>,
    #[serde(rename = "minCalls")]
    pub min_calls: u16,
    #[serde(rename = "maxCalls")]
    pub max_calls: u16,
    #[serde(rename = "allowRollback")]
    pub allow_rollback: bool,
    #[serde(rename = "allowExtraAccounts")]
    pub allow_extra_accounts: bool,
    pub accounts: Vec<CpiAccountBindingV2>,
}

/// Map one child position to one concrete parent position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpiAccountBindingV2 {
    #[serde(rename = "childPosition")]
    pub child_position: u16,
    #[serde(rename = "parentPosition")]
    pub parent_position: u16,
    pub signer: PrivilegeRequirementV2,
    pub writable: PrivilegeRequirementV2,
}

/// Strict v0.2 parse/validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectContractV2Error {
    Json(String),
    UnsupportedVersion(String),
    InvalidContract(String),
}

impl fmt::Display for EffectContractV2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(message) => write!(f, "effect v0.2 JSON error: {message}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported Effect ABI version `{version}`")
            }
            Self::InvalidContract(message) => write!(f, "invalid Effect ABI v0.2: {message}"),
        }
    }
}

impl std::error::Error for EffectContractV2Error {}

impl EffectContractV2 {
    /// Parse a strict Effect ABI v0.2 JSON document.
    pub fn from_json(json: &str) -> Result<Self, EffectContractV2Error> {
        let contract: Self =
            serde_json::from_str(json).map_err(|error| EffectContractV2Error::Json(error.to_string()))?;
        contract.validate()?;
        Ok(contract)
    }

    /// Validate identities, deterministic account expansion, ranges, and CPI
    /// envelope uniqueness. This is also run before commitment encoding.
    pub fn validate(&self) -> Result<(), EffectContractV2Error> {
        if self.effect_abi_version != EFFECT_ABI_V2 {
            return Err(EffectContractV2Error::UnsupportedVersion(
                self.effect_abi_version.clone(),
            ));
        }
        if self.instructions.is_empty() {
            return invalid("instructions must not be empty");
        }
        for (index, instruction) in self.instructions.iter().enumerate() {
            validate_instruction(instruction)?;
            for other in &self.instructions[index + 1..] {
                if instruction.name == other.name {
                    return invalid(format!(
                        "duplicate instruction name `{}`",
                        instruction.name
                    ));
                }
                if is_prefix(&instruction.discriminator, &other.discriminator)
                    || is_prefix(&other.discriminator, &instruction.discriminator)
                {
                    return invalid(format!(
                        "instruction discriminators for `{}` and `{}` are equal or prefix-shadowed",
                        instruction.name, other.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// Find the one instruction whose exact discriminator prefixes `data`.
    pub fn instruction_for_data(&self, data: &[u8]) -> Option<&InstructionEffectContractV2> {
        self.instructions
            .iter()
            .find(|instruction| data.starts_with(&instruction.discriminator))
    }

    /// Canonical non-JSON encoding for stable commitments.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, EffectContractV2Error> {
        self.validate()?;
        let mut out = Vec::new();
        out.extend_from_slice(b"grillo.effect-contract.v0.2.c1");
        encode_string(&mut out, &self.effect_abi_version);
        encode_deployment(&mut out, &self.deployment);
        encode_len(&mut out, self.instructions.len());
        for instruction in &self.instructions {
            encode_instruction(&mut out, instruction);
        }
        Ok(out)
    }

    /// SHA-256 commitment to every authoritative v0.2 contract field.
    pub fn commitment(&self) -> Result<[u8; 32], EffectContractV2Error> {
        Ok(sha256(&self.canonical_bytes()?))
    }
}

impl InstructionEffectContractV2 {
    /// Deterministically expand fixed and remaining roles in invocation order.
    pub fn expanded_roles(&self) -> Vec<(String, AccountRoleContractV2)> {
        let mut out = Vec::new();
        for role in &self.accounts {
            out.push((role.name.clone(), role.clone()));
        }
        for group in &self.remaining_accounts.groups {
            for iteration in 0..group.repeat {
                for role in &group.roles {
                    out.push((
                        format!("{}.{}[{}]", group.name, role.name, iteration),
                        role.clone(),
                    ));
                }
            }
        }
        out
    }
}

fn validate_instruction(
    instruction: &InstructionEffectContractV2,
) -> Result<(), EffectContractV2Error> {
    if instruction.name.is_empty() {
        return invalid("instruction name must not be empty");
    }
    if instruction.discriminator.is_empty() || instruction.discriminator.len() > 32 {
        return invalid(format!(
            "instruction `{}` discriminator length must be 1..=32",
            instruction.name
        ));
    }
    if instruction.completeness.accounts
        && (!instruction.remaining_accounts.complete
            || instruction.remaining_accounts.allow_trailing)
    {
        return invalid(format!(
            "instruction `{}` claims complete accounts but its remaining grammar is open",
            instruction.name
        ));
    }
    if instruction.completeness.cpi {
        if let CpiPolicyV2::Declared { complete, .. } = &instruction.cpi {
            if !complete {
                return invalid(format!(
                    "instruction `{}` claims complete CPI but its declaration is incomplete",
                    instruction.name
                ));
            }
        }
    }

    let mut total = instruction.accounts.len();
    validate_role_names(&instruction.accounts, &format!("instruction `{}`", instruction.name))?;
    for role in &instruction.accounts {
        validate_role(role)?;
    }
    for (group_index, group) in instruction.remaining_accounts.groups.iter().enumerate() {
        if group.name.is_empty() {
            return invalid("remaining group name must not be empty");
        }
        for other in &instruction.remaining_accounts.groups[group_index + 1..] {
            if group.name == other.name {
                return invalid(format!("duplicate remaining group `{}`", group.name));
            }
        }
        if group.repeat > 0 && group.roles.is_empty() {
            return invalid(format!(
                "remaining group `{}` repeats but has no roles",
                group.name
            ));
        }
        validate_role_names(&group.roles, &format!("remaining group `{}`", group.name))?;
        for role in &group.roles {
            validate_role(role)?;
        }
        total = total
            .checked_add(group.roles.len().saturating_mul(group.repeat as usize))
            .ok_or_else(|| {
                EffectContractV2Error::InvalidContract("account expansion overflows".into())
            })?;
    }
    if total > 256 {
        return invalid(format!(
            "instruction `{}` expands to {total} accounts; maximum is 256",
            instruction.name
        ));
    }
    validate_cpi(&instruction.cpi, total, &instruction.name)
}

fn validate_role_names(
    roles: &[AccountRoleContractV2],
    scope: &str,
) -> Result<(), EffectContractV2Error> {
    for (index, role) in roles.iter().enumerate() {
        if role.name.is_empty() {
            return invalid(format!("{scope} contains an empty role name"));
        }
        if roles[index + 1..]
            .iter()
            .any(|other| other.name == role.name)
        {
            return invalid(format!("{scope} contains duplicate role `{}`", role.name));
        }
    }
    Ok(())
}

fn validate_role(role: &AccountRoleContractV2) -> Result<(), EffectContractV2Error> {
    for range in &role.transition.data.ranges {
        if range.size == 0 || range.end() > u32::MAX as u64 {
            return invalid(format!(
                "role `{}` has an empty or overflowing data range",
                role.name
            ));
        }
    }
    if let LengthPolicyV2::Range { min, max } = role.transition.data_length {
        if min > max {
            return invalid(format!("role `{}` length range is inverted", role.name));
        }
    }
    Ok(())
}

fn validate_cpi(
    policy: &CpiPolicyV2,
    parent_account_count: usize,
    instruction_name: &str,
) -> Result<(), EffectContractV2Error> {
    let CpiPolicyV2::Declared { complete, calls } = policy else {
        return Ok(());
    };
    if *complete && calls.iter().any(|call| call.allow_extra_accounts) {
        return invalid(format!(
            "instruction `{instruction_name}` has a complete CPI policy with an open child account envelope"
        ));
    }
    for (index, call) in calls.iter().enumerate() {
        if call.id.is_empty() || call.discriminator.is_empty() || call.discriminator.len() > 32 {
            return invalid(format!(
                "instruction `{instruction_name}` has a CPI call with an empty id or invalid discriminator"
            ));
        }
        if call.min_calls > call.max_calls || call.max_calls == 0 {
            return invalid(format!("CPI envelope `{}` has invalid call bounds", call.id));
        }
        for other in &calls[index + 1..] {
            if call.id == other.id {
                return invalid(format!("duplicate CPI envelope id `{}`", call.id));
            }
            if call.program_id == other.program_id
                && (is_prefix(&call.discriminator, &other.discriminator)
                    || is_prefix(&other.discriminator, &call.discriminator))
            {
                return invalid(format!(
                    "CPI envelopes `{}` and `{}` are discriminator-ambiguous",
                    call.id, other.id
                ));
            }
        }
        for (binding_index, binding) in call.accounts.iter().enumerate() {
            if binding.parent_position as usize >= parent_account_count {
                return invalid(format!(
                    "CPI envelope `{}` references parent position {} outside {parent_account_count}",
                    call.id, binding.parent_position
                ));
            }
            if call.accounts[binding_index + 1..]
                .iter()
                .any(|other| other.child_position == binding.child_position)
            {
                return invalid(format!(
                    "CPI envelope `{}` binds child position {} twice",
                    call.id, binding.child_position
                ));
            }
        }
        if !call.allow_extra_accounts {
            for expected in 0..call.accounts.len() {
                if !call
                    .accounts
                    .iter()
                    .any(|binding| binding.child_position as usize == expected)
                {
                    return invalid(format!(
                        "CPI envelope `{}` has a gap at child position {expected}",
                        call.id
                    ));
                }
            }
        }
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, EffectContractV2Error> {
    Err(EffectContractV2Error::InvalidContract(message.into()))
}

fn is_prefix(left: &[u8], right: &[u8]) -> bool {
    right.starts_with(left)
}

fn encode_instruction(out: &mut Vec<u8>, instruction: &InstructionEffectContractV2) {
    encode_string(out, &instruction.name);
    encode_bytes(out, &instruction.discriminator);
    encode_completeness(out, instruction.completeness);
    encode_len(out, instruction.accounts.len());
    for role in &instruction.accounts {
        encode_role(out, role);
    }
    out.push(instruction.remaining_accounts.complete as u8);
    out.push(instruction.remaining_accounts.allow_trailing as u8);
    out.push(duplicate_tag(instruction.remaining_accounts.duplicate_policy));
    encode_len(out, instruction.remaining_accounts.groups.len());
    for group in &instruction.remaining_accounts.groups {
        encode_string(out, &group.name);
        out.extend_from_slice(&group.repeat.to_le_bytes());
        encode_len(out, group.roles.len());
        for role in &group.roles {
            encode_role(out, role);
        }
    }
    encode_cpi(out, &instruction.cpi);
}

fn encode_deployment(out: &mut Vec<u8>, deployment: &DeploymentBindingV2) {
    match deployment {
        DeploymentBindingV2::ExactDeployment {
            program_id,
            loader_id,
            executable_digest,
        } => {
            out.push(0);
            out.extend_from_slice(program_id);
            out.extend_from_slice(loader_id);
            out.extend_from_slice(executable_digest);
        }
        DeploymentBindingV2::ExactArtifact { executable_digest } => {
            out.push(1);
            out.extend_from_slice(executable_digest);
        }
    }
}

fn encode_completeness(out: &mut Vec<u8>, c: ContractCompletenessV2) {
    out.extend_from_slice(&[
        c.accounts as u8,
        c.data as u8,
        c.lamports as u8,
        c.owner as u8,
        c.data_length as u8,
        c.presence as u8,
        c.executable as u8,
        c.cpi as u8,
    ]);
}

fn encode_role(out: &mut Vec<u8>, role: &AccountRoleContractV2) {
    encode_string(out, &role.name);
    out.push(privilege_tag(role.signer));
    out.push(privilege_tag(role.writable));
    match &role.address {
        AddressConstraintV2::Any => out.push(0),
        AddressConstraintV2::Exact { address } => {
            out.push(1);
            out.extend_from_slice(address);
        }
        AddressConstraintV2::ProgramId => out.push(2),
    }
    encode_len(out, role.transition.data.ranges.len());
    for range in &role.transition.data.ranges {
        out.extend_from_slice(&range.offset.to_le_bytes());
        out.extend_from_slice(&range.size.to_le_bytes());
    }
    out.push(lamport_tag(role.transition.lamports));
    encode_owner(out, &role.transition.owner);
    encode_length(out, &role.transition.data_length);
    out.push(presence_tag(role.transition.presence));
    match role.transition.executable {
        ExecutablePolicyV2::Preserve => out.push(0),
        ExecutablePolicyV2::Set { value } => out.extend_from_slice(&[1, value as u8]),
        ExecutablePolicyV2::MayChange => out.push(2),
    }
}

fn encode_owner(out: &mut Vec<u8>, policy: &OwnerPolicyV2) {
    match policy {
        OwnerPolicyV2::Preserve => out.push(0),
        OwnerPolicyV2::SetTo { target } => {
            out.push(1);
            match target {
                OwnerTargetV2::Exact { address } => {
                    out.push(0);
                    out.extend_from_slice(address);
                }
                OwnerTargetV2::ProgramId => out.push(1),
                OwnerTargetV2::SystemProgram => out.push(2),
            }
        }
        OwnerPolicyV2::MayChange => out.push(2),
    }
}

fn encode_length(out: &mut Vec<u8>, policy: &LengthPolicyV2) {
    match policy {
        LengthPolicyV2::Preserve => out.push(0),
        LengthPolicyV2::Exact { length } => {
            out.push(1);
            out.extend_from_slice(&length.to_le_bytes());
        }
        LengthPolicyV2::Range { min, max } => {
            out.push(2);
            out.extend_from_slice(&min.to_le_bytes());
            out.extend_from_slice(&max.to_le_bytes());
        }
        LengthPolicyV2::MayChange => out.push(3),
    }
}

fn encode_cpi(out: &mut Vec<u8>, policy: &CpiPolicyV2) {
    match policy {
        CpiPolicyV2::Forbidden => out.push(0),
        CpiPolicyV2::Declared { complete, calls } => {
            out.extend_from_slice(&[1, *complete as u8]);
            encode_len(out, calls.len());
            for call in calls {
                encode_string(out, &call.id);
                out.extend_from_slice(&call.program_id);
                out.extend_from_slice(&call.loader_id);
                out.extend_from_slice(&call.executable_digest);
                encode_bytes(out, &call.discriminator);
                out.extend_from_slice(&call.min_calls.to_le_bytes());
                out.extend_from_slice(&call.max_calls.to_le_bytes());
                out.push(call.allow_rollback as u8);
                out.push(call.allow_extra_accounts as u8);
                encode_len(out, call.accounts.len());
                for binding in &call.accounts {
                    out.extend_from_slice(&binding.child_position.to_le_bytes());
                    out.extend_from_slice(&binding.parent_position.to_le_bytes());
                    out.push(privilege_tag(binding.signer));
                    out.push(privilege_tag(binding.writable));
                }
            }
        }
    }
}

fn encode_string(out: &mut Vec<u8>, value: &str) {
    encode_bytes(out, value.as_bytes());
}

fn encode_bytes(out: &mut Vec<u8>, value: &[u8]) {
    encode_len(out, value.len());
    out.extend_from_slice(value);
}

fn encode_len(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&(value as u32).to_le_bytes());
}

fn privilege_tag(value: PrivilegeRequirementV2) -> u8 {
    match value {
        PrivilegeRequirementV2::Required => 0,
        PrivilegeRequirementV2::Forbidden => 1,
        PrivilegeRequirementV2::Allowed => 2,
    }
}

fn duplicate_tag(value: DuplicatePolicyV2) -> u8 {
    match value {
        DuplicatePolicyV2::DenyAll => 0,
        DuplicatePolicyV2::DenyWritable => 1,
        DuplicatePolicyV2::Allow => 2,
    }
}

fn lamport_tag(value: LamportPolicyV2) -> u8 {
    match value {
        LamportPolicyV2::Preserve => 0,
        LamportPolicyV2::MayChange => 1,
        LamportPolicyV2::DebitOnly => 2,
        LamportPolicyV2::CreditOnly => 3,
    }
}

fn presence_tag(value: PresencePolicyV2) -> u8 {
    match value {
        PresencePolicyV2::Preserve => 0,
        PresencePolicyV2::MustRemainPresent => 1,
        PresencePolicyV2::MustCreate => 2,
        PresencePolicyV2::MayCreate => 3,
        PresencePolicyV2::MustClose => 4,
        PresencePolicyV2::MayClose => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preserve() -> TransitionPolicyV2 {
        TransitionPolicyV2 {
            data: DataPolicyV2 { ranges: vec![] },
            lamports: LamportPolicyV2::Preserve,
            owner: OwnerPolicyV2::Preserve,
            data_length: LengthPolicyV2::Preserve,
            presence: PresencePolicyV2::Preserve,
            executable: ExecutablePolicyV2::Preserve,
        }
    }

    fn contract() -> EffectContractV2 {
        EffectContractV2 {
            effect_abi_version: EFFECT_ABI_V2.into(),
            deployment: DeploymentBindingV2::ExactArtifact {
                executable_digest: [7; 32],
            },
            instructions: vec![InstructionEffectContractV2 {
                name: "set".into(),
                discriminator: vec![1, 2],
                completeness: ContractCompletenessV2 {
                    accounts: true,
                    data: true,
                    lamports: true,
                    owner: true,
                    data_length: true,
                    presence: true,
                    executable: true,
                    cpi: true,
                },
                accounts: vec![AccountRoleContractV2 {
                    name: "state".into(),
                    signer: PrivilegeRequirementV2::Forbidden,
                    writable: PrivilegeRequirementV2::Required,
                    address: AddressConstraintV2::Any,
                    transition: preserve(),
                }],
                remaining_accounts: RemainingAccountsContractV2 {
                    complete: true,
                    allow_trailing: false,
                    duplicate_policy: DuplicatePolicyV2::DenyWritable,
                    groups: vec![],
                },
                cpi: CpiPolicyV2::Forbidden,
            }],
        }
    }

    #[test]
    fn commitment_covers_full_discriminator_and_roles() {
        let base = contract().commitment().unwrap();
        let mut changed = contract();
        changed.instructions[0].discriminator[1] = 3;
        assert_ne!(base, changed.commitment().unwrap());
        let mut changed = contract();
        changed.instructions[0].accounts[0].writable = PrivilegeRequirementV2::Allowed;
        assert_ne!(base, changed.commitment().unwrap());
    }

    #[test]
    fn discriminator_prefix_shadow_is_rejected() {
        let mut value = contract();
        let mut second = value.instructions[0].clone();
        second.name = "shadow".into();
        second.discriminator = vec![1, 2, 3];
        value.instructions.push(second);
        assert!(matches!(
            value.validate(),
            Err(EffectContractV2Error::InvalidContract(message)) if message.contains("prefix-shadowed")
        ));
    }
}
