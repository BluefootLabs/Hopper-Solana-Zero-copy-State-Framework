//! Concrete Effect ABI v0.2 invocation evidence and fail-closed binding.
//!
//! `InvocationFrameV2` is caller-supplied evidence. Its provenance is only a
//! label and is never promoted to authenticated execution by this crate.
//! `BoundInvocationV2` has private fields and can only be constructed after
//! identity, discriminator, account grammar, unique-state, alias, and CPI
//! envelope validation.

use core::fmt;

use grillo_manifest::{
    sha256, AddressConstraintV2, CpiEnvelopeV2, CpiPolicyV2, DeploymentBindingV2,
    DuplicatePolicyV2, EffectContractV2, InstructionEffectContractV2,
    PrivilegeRequirementV2,
};

/// Cluster identity for replay separation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkIdentityV2 {
    pub genesis_hash: [u8; 32],
}

/// Transaction and exact call-tree location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionIdentityV2 {
    pub signature: [u8; 64],
    pub message_hash: [u8; 32],
    pub bank_slot: u64,
    pub outer_instruction_index: u16,
    /// Child ordinals from the outer instruction to this frame.
    pub invocation_path: Vec<u16>,
}

/// Concrete loader/deployment identity observed for one invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentIdentityV2 {
    pub program_id: [u8; 32],
    pub loader_id: [u8; 32],
    pub executable_digest: [u8; 32],
    pub programdata_address: Option<[u8; 32]>,
    pub deployment_slot: Option<u64>,
    pub manifest_commitment: [u8; 32],
}

/// Effective account meta at one invocation position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationAccountRefV2 {
    pub position: u16,
    pub transaction_index: u16,
    pub pubkey: [u8; 32],
    pub signer: bool,
    pub writable: bool,
}

/// Logical loaded-state observation. Absence is explicit and is never
/// inferred from zero lamports or System ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountStateV2 {
    Absent,
    Present {
        lamports: u64,
        owner: [u8; 32],
        executable: bool,
        data: Vec<u8>,
    },
}

impl AccountStateV2 {
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present { .. })
    }

    pub fn lamports(&self) -> u64 {
        match self {
            Self::Absent => 0,
            Self::Present { lamports, .. } => *lamports,
        }
    }

    pub fn data_len(&self) -> usize {
        match self {
            Self::Absent => 0,
            Self::Present { data, .. } => data.len(),
        }
    }
}

/// Pre/post state for one unique pubkey.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountTransitionV2 {
    pub pubkey: [u8; 32],
    pub pre: AccountStateV2,
    pub post: AccountStateV2,
}

/// Dimensions claimed present in this evidence object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvidenceCompletenessV2 {
    pub accounts: bool,
    pub data: bool,
    pub lamports: bool,
    pub owner: bool,
    pub data_length: bool,
    pub presence: bool,
    pub executable: bool,
    pub cpi: bool,
    pub touch: bool,
}

impl EvidenceCompletenessV2 {
    pub fn all_state_dimensions(self) -> bool {
        self.accounts
            && self.data
            && self.lamports
            && self.owner
            && self.data_length
            && self.presence
            && self.executable
    }
}

/// Boundary at which pre/post state was observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationBoundaryV2 {
    InvocationEntryExit,
    TransactionPrePost,
    Unknown,
}

/// Untrusted provenance label retained in evidence and verdicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceProvenanceV2 {
    Fixture,
    RpcObserved { endpoint: String },
    ReplayClaimed { source: String },
    ProviderClaimed {
        provider: [u8; 32],
        signature: [u8; 64],
    },
}

/// This pure verifier performs no signature, ledger, or replay validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnverifiedAuthenticityV2 {
    Unauthenticated,
}

/// Optional attributed touch carrier. The digest is committed, but this
/// first slice does not treat it as independently authenticated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TouchEvidenceV2 {
    pub complete: bool,
    pub digest: Option<[u8; 32]>,
}

/// Whether a call returned normally or was rolled back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvocationOutcomeV2 {
    Succeeded,
    Failed,
    RolledBack,
}

/// Full concrete evidence for one invocation plus its ordered CPI children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationFrameV2 {
    pub network: NetworkIdentityV2,
    pub transaction: TransactionIdentityV2,
    pub deployment: DeploymentIdentityV2,
    pub instruction_data: Vec<u8>,
    pub accounts: Vec<InvocationAccountRefV2>,
    /// Exactly one entry per unique account pubkey when `evidence.accounts`.
    pub states: Vec<AccountTransitionV2>,
    pub touch: TouchEvidenceV2,
    pub children: Vec<InvocationFrameV2>,
    pub outcome: InvocationOutcomeV2,
    pub boundary: ObservationBoundaryV2,
    pub evidence: EvidenceCompletenessV2,
    pub provenance: EvidenceProvenanceV2,
}

impl InvocationFrameV2 {
    /// Stable digest over all frame fields. This commits caller-supplied
    /// provenance but does not authenticate it.
    pub fn commitment(&self) -> [u8; 32] {
        let mut out = Vec::new();
        out.extend_from_slice(b"grillo.invocation-frame.v0.2.c1");
        encode_frame(&mut out, self);
        sha256(&out)
    }
}

/// Concrete resolved role after expanding the remaining-account grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedAccountRoleV2 {
    name: String,
    position: u16,
    pubkey: [u8; 32],
    contract: grillo_manifest::AccountRoleContractV2,
}

impl ResolvedAccountRoleV2 {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn position(&self) -> u16 {
        self.position
    }

    pub fn pubkey(&self) -> [u8; 32] {
        self.pubkey
    }

    pub fn contract(&self) -> &grillo_manifest::AccountRoleContractV2 {
        &self.contract
    }
}

/// Immutable, validated binding of a concrete frame to a v0.2 contract.
#[derive(Clone, Debug)]
pub struct BoundInvocationV2 {
    contract_commitment: [u8; 32],
    frame_commitment: [u8; 32],
    commitment: [u8; 32],
    instruction: InstructionEffectContractV2,
    frame: InvocationFrameV2,
    roles: Vec<ResolvedAccountRoleV2>,
}

impl BoundInvocationV2 {
    pub fn contract_commitment(&self) -> [u8; 32] {
        self.contract_commitment
    }

    pub fn frame_commitment(&self) -> [u8; 32] {
        self.frame_commitment
    }

    pub fn commitment(&self) -> [u8; 32] {
        self.commitment
    }

    pub fn instruction_name(&self) -> &str {
        &self.instruction.name
    }

    pub fn roles(&self) -> &[ResolvedAccountRoleV2] {
        &self.roles
    }

    pub fn provenance(&self) -> &EvidenceProvenanceV2 {
        &self.frame.provenance
    }

    pub fn authenticity(&self) -> UnverifiedAuthenticityV2 {
        UnverifiedAuthenticityV2::Unauthenticated
    }

    pub(crate) fn instruction(&self) -> &InstructionEffectContractV2 {
        &self.instruction
    }

    pub(crate) fn frame(&self) -> &InvocationFrameV2 {
        &self.frame
    }
}

/// Why an untrusted frame could not be bound to a contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindErrorV2 {
    InvalidContract(String),
    InstructionNotFound,
    DeploymentMismatch { field: &'static str },
    ManifestCommitmentMismatch,
    AccountCount { expected: usize, actual: usize },
    AccountPosition { expected: u16, actual: u16 },
    PrivilegeMismatch { position: u16, privilege: &'static str },
    AddressMismatch { position: u16 },
    DuplicateAccount { first: u16, second: u16 },
    DuplicateState { pubkey: [u8; 32] },
    MissingState { pubkey: [u8; 32] },
    ExtraState { pubkey: [u8; 32] },
    ForbiddenCpi,
    UndeclaredCpi { child_index: usize },
    CpiCount { envelope: String, min: u16, max: u16, actual: u16 },
    CpiAccountMismatch { child_index: usize, child_position: u16 },
    CpiRollbackForbidden { child_index: usize },
    IncompleteRollbackEvidence { child_index: usize },
    RolledBackMutation { child_index: usize, pubkey: [u8; 32] },
    InvalidFrame(String),
}

impl fmt::Display for BindErrorV2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Effect ABI v0.2 frame bind failed: {self:?}")
    }
}

impl std::error::Error for BindErrorV2 {}

/// Validate and bind one concrete frame. Provenance is copied verbatim and
/// authenticity remains `Unauthenticated` regardless of its label.
pub fn bind_invocation_v2(
    contract: &EffectContractV2,
    frame: &InvocationFrameV2,
) -> Result<BoundInvocationV2, BindErrorV2> {
    contract
        .validate()
        .map_err(|error| BindErrorV2::InvalidContract(error.to_string()))?;
    validate_frame_shape(frame, None)?;

    let contract_commitment = contract
        .commitment()
        .map_err(|error| BindErrorV2::InvalidContract(error.to_string()))?;
    if frame.deployment.manifest_commitment != contract_commitment {
        return Err(BindErrorV2::ManifestCommitmentMismatch);
    }
    match &contract.deployment {
        DeploymentBindingV2::ExactDeployment {
            program_id,
            loader_id,
            executable_digest,
        } => {
            if &frame.deployment.program_id != program_id {
                return Err(BindErrorV2::DeploymentMismatch { field: "program_id" });
            }
            if &frame.deployment.loader_id != loader_id {
                return Err(BindErrorV2::DeploymentMismatch { field: "loader_id" });
            }
            if &frame.deployment.executable_digest != executable_digest {
                return Err(BindErrorV2::DeploymentMismatch {
                    field: "executable_digest",
                });
            }
        }
        DeploymentBindingV2::ExactArtifact { executable_digest } => {
            if &frame.deployment.executable_digest != executable_digest {
                return Err(BindErrorV2::DeploymentMismatch {
                    field: "executable_digest",
                });
            }
        }
    }

    let instruction = contract
        .instruction_for_data(&frame.instruction_data)
        .ok_or(BindErrorV2::InstructionNotFound)?;
    let expanded = instruction.expanded_roles();
    if frame.accounts.len() != expanded.len() {
        return Err(BindErrorV2::AccountCount {
            expected: expanded.len(),
            actual: frame.accounts.len(),
        });
    }

    let mut roles = Vec::with_capacity(expanded.len());
    for (index, ((name, role), account)) in expanded.iter().zip(&frame.accounts).enumerate() {
        validate_privilege(role.signer, account.signer, account.position, "signer")?;
        validate_privilege(role.writable, account.writable, account.position, "writable")?;
        let address_ok = match &role.address {
            AddressConstraintV2::Any => true,
            AddressConstraintV2::Exact { address } => account.pubkey == *address,
            AddressConstraintV2::ProgramId => account.pubkey == frame.deployment.program_id,
        };
        if !address_ok {
            return Err(BindErrorV2::AddressMismatch {
                position: index as u16,
            });
        }
        roles.push(ResolvedAccountRoleV2 {
            name: name.clone(),
            position: index as u16,
            pubkey: account.pubkey,
            contract: role.clone(),
        });
    }

    validate_aliases(instruction, &frame.accounts)?;
    validate_cpi_policy(instruction, frame)?;

    let frame_commitment = frame.commitment();
    let mut bound_bytes = Vec::new();
    bound_bytes.extend_from_slice(b"grillo.bound-invocation.v0.2.c1");
    bound_bytes.extend_from_slice(&contract_commitment);
    bound_bytes.extend_from_slice(&frame_commitment);
    encode_bytes(&mut bound_bytes, instruction.name.as_bytes());
    encode_bytes(&mut bound_bytes, &instruction.discriminator);
    encode_len(&mut bound_bytes, roles.len());
    for role in &roles {
        encode_bytes(&mut bound_bytes, role.name.as_bytes());
        bound_bytes.extend_from_slice(&role.position.to_le_bytes());
        bound_bytes.extend_from_slice(&role.pubkey);
    }
    let commitment = sha256(&bound_bytes);

    Ok(BoundInvocationV2 {
        contract_commitment,
        frame_commitment,
        commitment,
        instruction: instruction.clone(),
        frame: frame.clone(),
        roles,
    })
}

fn validate_frame_shape(
    frame: &InvocationFrameV2,
    child_index: Option<usize>,
) -> Result<(), BindErrorV2> {
    for (index, account) in frame.accounts.iter().enumerate() {
        if account.position as usize != index {
            return Err(BindErrorV2::AccountPosition {
                expected: index as u16,
                actual: account.position,
            });
        }
    }
    for (index, state) in frame.states.iter().enumerate() {
        if frame.states[index + 1..]
            .iter()
            .any(|other| other.pubkey == state.pubkey)
        {
            return Err(BindErrorV2::DuplicateState { pubkey: state.pubkey });
        }
        if !frame.accounts.iter().any(|account| account.pubkey == state.pubkey) {
            return Err(BindErrorV2::ExtraState { pubkey: state.pubkey });
        }
    }
    if frame.evidence.accounts {
        for account in &frame.accounts {
            if !frame.states.iter().any(|state| state.pubkey == account.pubkey) {
                return Err(BindErrorV2::MissingState {
                    pubkey: account.pubkey,
                });
            }
        }
    }
    if frame.outcome == InvocationOutcomeV2::RolledBack {
        let index = child_index.unwrap_or(0);
        if !frame.evidence.all_state_dimensions() {
            return Err(BindErrorV2::IncompleteRollbackEvidence { child_index: index });
        }
        for state in &frame.states {
            if state.pre != state.post {
                return Err(BindErrorV2::RolledBackMutation {
                    child_index: index,
                    pubkey: state.pubkey,
                });
            }
        }
    }
    for (index, child) in frame.children.iter().enumerate() {
        validate_frame_shape(child, Some(index))?;
    }
    Ok(())
}

fn validate_privilege(
    requirement: PrivilegeRequirementV2,
    actual: bool,
    position: u16,
    privilege: &'static str,
) -> Result<(), BindErrorV2> {
    let valid = match requirement {
        PrivilegeRequirementV2::Required => actual,
        PrivilegeRequirementV2::Forbidden => !actual,
        PrivilegeRequirementV2::Allowed => true,
    };
    if valid {
        Ok(())
    } else {
        Err(BindErrorV2::PrivilegeMismatch { position, privilege })
    }
}

fn validate_aliases(
    instruction: &InstructionEffectContractV2,
    accounts: &[InvocationAccountRefV2],
) -> Result<(), BindErrorV2> {
    let policy = instruction.remaining_accounts.duplicate_policy;
    for (index, left) in accounts.iter().enumerate() {
        for right in &accounts[index + 1..] {
            if left.pubkey != right.pubkey {
                continue;
            }
            let reject = match policy {
                DuplicatePolicyV2::DenyAll => true,
                DuplicatePolicyV2::DenyWritable => left.writable || right.writable,
                DuplicatePolicyV2::Allow => false,
            };
            if reject {
                return Err(BindErrorV2::DuplicateAccount {
                    first: left.position,
                    second: right.position,
                });
            }
        }
    }
    Ok(())
}

fn validate_cpi_policy(
    instruction: &InstructionEffectContractV2,
    frame: &InvocationFrameV2,
) -> Result<(), BindErrorV2> {
    match &instruction.cpi {
        CpiPolicyV2::Forbidden => {
            if frame.children.is_empty() {
                Ok(())
            } else {
                Err(BindErrorV2::ForbiddenCpi)
            }
        }
        CpiPolicyV2::Declared { calls, .. } => {
            let mut counts = vec![0u16; calls.len()];
            for (child_index, child) in frame.children.iter().enumerate() {
                let Some((call_index, call)) = calls.iter().enumerate().find(|(_, call)| {
                    child.deployment.program_id == call.program_id
                        && child.instruction_data.starts_with(&call.discriminator)
                }) else {
                    return Err(BindErrorV2::UndeclaredCpi { child_index });
                };
                if child.deployment.loader_id != call.loader_id
                    || child.deployment.executable_digest != call.executable_digest
                {
                    return Err(BindErrorV2::UndeclaredCpi { child_index });
                }
                counts[call_index] = counts[call_index].saturating_add(1);
                validate_cpi_child(frame, child, child_index, call)?;
            }
            for (index, call) in calls.iter().enumerate() {
                let actual = counts[index];
                let below_min = frame.evidence.cpi && actual < call.min_calls;
                if below_min || actual > call.max_calls {
                    return Err(BindErrorV2::CpiCount {
                        envelope: call.id.clone(),
                        min: call.min_calls,
                        max: call.max_calls,
                        actual,
                    });
                }
            }
            Ok(())
        }
    }
}

fn validate_cpi_child(
    parent: &InvocationFrameV2,
    child: &InvocationFrameV2,
    child_index: usize,
    call: &CpiEnvelopeV2,
) -> Result<(), BindErrorV2> {
    if child.outcome == InvocationOutcomeV2::RolledBack && !call.allow_rollback {
        return Err(BindErrorV2::CpiRollbackForbidden { child_index });
    }
    if !call.allow_extra_accounts && child.accounts.len() != call.accounts.len() {
        return Err(BindErrorV2::CpiAccountMismatch {
            child_index,
            child_position: child.accounts.len() as u16,
        });
    }
    for binding in &call.accounts {
        let Some(child_account) = child.accounts.get(binding.child_position as usize) else {
            return Err(BindErrorV2::CpiAccountMismatch {
                child_index,
                child_position: binding.child_position,
            });
        };
        let Some(parent_account) = parent.accounts.get(binding.parent_position as usize) else {
            return Err(BindErrorV2::CpiAccountMismatch {
                child_index,
                child_position: binding.child_position,
            });
        };
        if child_account.pubkey != parent_account.pubkey
            || validate_privilege(
                binding.signer,
                child_account.signer,
                child_account.position,
                "child signer",
            )
            .is_err()
            || validate_privilege(
                binding.writable,
                child_account.writable,
                child_account.position,
                "child writable",
            )
            .is_err()
        {
            return Err(BindErrorV2::CpiAccountMismatch {
                child_index,
                child_position: binding.child_position,
            });
        }
    }
    Ok(())
}

fn encode_frame(out: &mut Vec<u8>, frame: &InvocationFrameV2) {
    out.extend_from_slice(&frame.network.genesis_hash);
    out.extend_from_slice(&frame.transaction.signature);
    out.extend_from_slice(&frame.transaction.message_hash);
    out.extend_from_slice(&frame.transaction.bank_slot.to_le_bytes());
    out.extend_from_slice(&frame.transaction.outer_instruction_index.to_le_bytes());
    encode_len(out, frame.transaction.invocation_path.len());
    for ordinal in &frame.transaction.invocation_path {
        out.extend_from_slice(&ordinal.to_le_bytes());
    }
    encode_deployment(out, &frame.deployment);
    encode_bytes(out, &frame.instruction_data);
    encode_len(out, frame.accounts.len());
    for account in &frame.accounts {
        out.extend_from_slice(&account.position.to_le_bytes());
        out.extend_from_slice(&account.transaction_index.to_le_bytes());
        out.extend_from_slice(&account.pubkey);
        out.push(account.signer as u8);
        out.push(account.writable as u8);
    }
    encode_len(out, frame.states.len());
    for state in &frame.states {
        out.extend_from_slice(&state.pubkey);
        encode_state(out, &state.pre);
        encode_state(out, &state.post);
    }
    out.push(frame.touch.complete as u8);
    encode_option_32(out, frame.touch.digest);
    encode_len(out, frame.children.len());
    for child in &frame.children {
        encode_frame(out, child);
    }
    out.push(match frame.outcome {
        InvocationOutcomeV2::Succeeded => 0,
        InvocationOutcomeV2::Failed => 1,
        InvocationOutcomeV2::RolledBack => 2,
    });
    out.push(match frame.boundary {
        ObservationBoundaryV2::InvocationEntryExit => 0,
        ObservationBoundaryV2::TransactionPrePost => 1,
        ObservationBoundaryV2::Unknown => 2,
    });
    out.extend_from_slice(&[
        frame.evidence.accounts as u8,
        frame.evidence.data as u8,
        frame.evidence.lamports as u8,
        frame.evidence.owner as u8,
        frame.evidence.data_length as u8,
        frame.evidence.presence as u8,
        frame.evidence.executable as u8,
        frame.evidence.cpi as u8,
        frame.evidence.touch as u8,
    ]);
    match &frame.provenance {
        EvidenceProvenanceV2::Fixture => out.push(0),
        EvidenceProvenanceV2::RpcObserved { endpoint } => {
            out.push(1);
            encode_bytes(out, endpoint.as_bytes());
        }
        EvidenceProvenanceV2::ReplayClaimed { source } => {
            out.push(2);
            encode_bytes(out, source.as_bytes());
        }
        EvidenceProvenanceV2::ProviderClaimed {
            provider,
            signature,
        } => {
            out.push(3);
            out.extend_from_slice(provider);
            out.extend_from_slice(signature);
        }
    }
}

fn encode_deployment(out: &mut Vec<u8>, deployment: &DeploymentIdentityV2) {
    out.extend_from_slice(&deployment.program_id);
    out.extend_from_slice(&deployment.loader_id);
    out.extend_from_slice(&deployment.executable_digest);
    encode_option_32(out, deployment.programdata_address);
    match deployment.deployment_slot {
        Some(slot) => {
            out.push(1);
            out.extend_from_slice(&slot.to_le_bytes());
        }
        None => out.push(0),
    }
    out.extend_from_slice(&deployment.manifest_commitment);
}

fn encode_state(out: &mut Vec<u8>, state: &AccountStateV2) {
    match state {
        AccountStateV2::Absent => out.push(0),
        AccountStateV2::Present {
            lamports,
            owner,
            executable,
            data,
        } => {
            out.push(1);
            out.extend_from_slice(&lamports.to_le_bytes());
            out.extend_from_slice(owner);
            out.push(*executable as u8);
            encode_bytes(out, data);
        }
    }
}

fn encode_option_32(out: &mut Vec<u8>, value: Option<[u8; 32]>) {
    match value {
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value);
        }
        None => out.push(0),
    }
}

fn encode_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    encode_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

fn encode_len(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&(value as u32).to_le_bytes());
}
