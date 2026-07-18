//! Transition verification for a privately bound Effect ABI v0.2 frame.

use grillo_manifest::{
    sha256, ContractCompletenessV2, CpiPolicyV2, ExecutablePolicyV2, LamportPolicyV2,
    LengthPolicyV2, OwnerPolicyV2, OwnerTargetV2, PresencePolicyV2,
};

use crate::frame_v2::{
    AccountStateV2, AccountTransitionV2, BoundInvocationV2, EvidenceCompletenessV2,
    EvidenceProvenanceV2, InvocationOutcomeV2, ObservationBoundaryV2,
    UnverifiedAuthenticityV2,
};

/// One precise v0.2 transition violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViolationV2 {
    DataWriteOutsidePolicy {
        role: String,
        position: u16,
        offset: u32,
    },
    LamportTransition {
        role: String,
        position: u16,
        pre: u64,
        post: u64,
    },
    OwnerTransition { role: String, position: u16 },
    DataLengthTransition {
        role: String,
        position: u16,
        pre: u32,
        post: u32,
    },
    PresenceTransition { role: String, position: u16 },
    ExecutableTransition { role: String, position: u16 },
}

/// Why a privately bound frame still cannot support a complete verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InconclusiveReasonV2 {
    ContractDimensionIncomplete(&'static str),
    EvidenceDimensionIncomplete(&'static str),
    InvocationDidNotSucceed,
    NestedCpiRequiresInvocationBoundary,
}

/// Evidence returned by an unauthenticated behavioral PASS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassEvidenceV2 {
    pub contract_commitment: [u8; 32],
    pub frame_commitment: [u8; 32],
    pub bound_commitment: [u8; 32],
    pub verdict_commitment: [u8; 32],
    pub provenance: EvidenceProvenanceV2,
    pub authenticity: UnverifiedAuthenticityV2,
    pub observed_accounts: Vec<[u8; 32]>,
    pub changed_data_bytes: u64,
}

/// Behavioral verdict. No variant claims ledger or provider authenticity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectVerdictV2 {
    Pass(PassEvidenceV2),
    Violation(Vec<ViolationV2>),
    Inconclusive(InconclusiveReasonV2),
}

impl EffectVerdictV2 {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass(_))
    }
}

/// Verify every modeled account transition in a validated frame.
pub fn verify_bound_invocation_v2(bound: &BoundInvocationV2) -> EffectVerdictV2 {
    let instruction = bound.instruction();
    let frame = bound.frame();

    if frame.outcome != InvocationOutcomeV2::Succeeded {
        return EffectVerdictV2::Inconclusive(InconclusiveReasonV2::InvocationDidNotSucceed);
    }
    if let Some(dimension) = first_incomplete_contract(instruction.completeness) {
        return EffectVerdictV2::Inconclusive(
            InconclusiveReasonV2::ContractDimensionIncomplete(dimension),
        );
    }
    if let Some(dimension) = first_incomplete_evidence(frame.evidence) {
        return EffectVerdictV2::Inconclusive(
            InconclusiveReasonV2::EvidenceDimensionIncomplete(dimension),
        );
    }
    if !frame.children.is_empty()
        && matches!(instruction.cpi, CpiPolicyV2::Declared { .. })
        && frame.boundary != ObservationBoundaryV2::InvocationEntryExit
    {
        return EffectVerdictV2::Inconclusive(
            InconclusiveReasonV2::NestedCpiRequiresInvocationBoundary,
        );
    }

    let mut violations = Vec::new();
    let mut changed_data_bytes = 0u64;
    for role in bound.roles() {
        let state = frame
            .states
            .iter()
            .find(|state| state.pubkey == role.pubkey())
            .expect("complete bound frame has one state per role pubkey");
        verify_role(
            role.name(),
            role.position(),
            role.contract(),
            state,
            &frame.deployment.program_id,
            &mut changed_data_bytes,
            &mut violations,
        );
    }
    if !violations.is_empty() {
        return EffectVerdictV2::Violation(violations);
    }

    let mut observed_accounts: Vec<[u8; 32]> = frame.states.iter().map(|s| s.pubkey).collect();
    observed_accounts.sort_unstable();
    let mut verdict_bytes = Vec::new();
    verdict_bytes.extend_from_slice(b"grillo.effect-verdict.v0.2.c1");
    verdict_bytes.extend_from_slice(&bound.commitment());
    verdict_bytes.push(0); // PASS
    verdict_bytes.extend_from_slice(&changed_data_bytes.to_le_bytes());
    verdict_bytes.extend_from_slice(&(observed_accounts.len() as u32).to_le_bytes());
    for pubkey in &observed_accounts {
        verdict_bytes.extend_from_slice(pubkey);
    }

    EffectVerdictV2::Pass(PassEvidenceV2 {
        contract_commitment: bound.contract_commitment(),
        frame_commitment: bound.frame_commitment(),
        bound_commitment: bound.commitment(),
        verdict_commitment: sha256(&verdict_bytes),
        provenance: bound.provenance().clone(),
        authenticity: bound.authenticity(),
        observed_accounts,
        changed_data_bytes,
    })
}

fn first_incomplete_contract(completeness: ContractCompletenessV2) -> Option<&'static str> {
    [
        ("accounts", completeness.accounts),
        ("data", completeness.data),
        ("lamports", completeness.lamports),
        ("owner", completeness.owner),
        ("dataLength", completeness.data_length),
        ("presence", completeness.presence),
        ("executable", completeness.executable),
        ("cpi", completeness.cpi),
    ]
    .into_iter()
    .find_map(|(name, complete)| (!complete).then_some(name))
}

fn first_incomplete_evidence(completeness: EvidenceCompletenessV2) -> Option<&'static str> {
    [
        ("accounts", completeness.accounts),
        ("data", completeness.data),
        ("lamports", completeness.lamports),
        ("owner", completeness.owner),
        ("dataLength", completeness.data_length),
        ("presence", completeness.presence),
        ("executable", completeness.executable),
        ("cpi", completeness.cpi),
    ]
    .into_iter()
    .find_map(|(name, complete)| (!complete).then_some(name))
}

#[allow(clippy::too_many_arguments)]
fn verify_role(
    role_name: &str,
    position: u16,
    contract: &grillo_manifest::AccountRoleContractV2,
    state: &AccountTransitionV2,
    program_id: &[u8; 32],
    changed_data_bytes: &mut u64,
    violations: &mut Vec<ViolationV2>,
) {
    let pre_present = state.pre.is_present();
    let post_present = state.post.is_present();

    let presence_ok = match contract.transition.presence {
        PresencePolicyV2::Preserve => pre_present == post_present,
        PresencePolicyV2::MustRemainPresent => pre_present && post_present,
        PresencePolicyV2::MustCreate => !pre_present && post_present,
        PresencePolicyV2::MayCreate => post_present,
        PresencePolicyV2::MustClose => pre_present && !post_present,
        PresencePolicyV2::MayClose => pre_present,
    };
    if !presence_ok {
        violations.push(ViolationV2::PresenceTransition {
            role: role_name.to_string(),
            position,
        });
    }

    let pre_lamports = state.pre.lamports();
    let post_lamports = state.post.lamports();
    let lamports_ok = match contract.transition.lamports {
        LamportPolicyV2::Preserve => pre_lamports == post_lamports,
        LamportPolicyV2::MayChange => true,
        LamportPolicyV2::DebitOnly => post_lamports <= pre_lamports,
        LamportPolicyV2::CreditOnly => post_lamports >= pre_lamports,
    };
    if !lamports_ok {
        violations.push(ViolationV2::LamportTransition {
            role: role_name.to_string(),
            position,
            pre: pre_lamports,
            post: post_lamports,
        });
    }

    if !owner_transition_ok(&contract.transition.owner, &state.pre, &state.post, program_id) {
        violations.push(ViolationV2::OwnerTransition {
            role: role_name.to_string(),
            position,
        });
    }

    let pre_len = state.pre.data_len().min(u32::MAX as usize) as u32;
    let post_len = state.post.data_len().min(u32::MAX as usize) as u32;
    let length_ok = match contract.transition.data_length {
        LengthPolicyV2::Preserve => pre_len == post_len,
        LengthPolicyV2::Exact { length } => post_len == length,
        LengthPolicyV2::Range { min, max } => min <= post_len && post_len <= max,
        LengthPolicyV2::MayChange => true,
    };
    if !length_ok {
        violations.push(ViolationV2::DataLengthTransition {
            role: role_name.to_string(),
            position,
            pre: pre_len,
            post: post_len,
        });
    }

    if !executable_transition_ok(
        &contract.transition.executable,
        &state.pre,
        &state.post,
    ) {
        violations.push(ViolationV2::ExecutableTransition {
            role: role_name.to_string(),
            position,
        });
    }

    let pre_data = data_or_empty(&state.pre);
    let post_data = data_or_empty(&state.post);
    // Removed tail bytes are governed by the length transition because no
    // post-state byte exists. Changed common-prefix bytes and newly created
    // bytes must be covered by the data policy.
    for offset in 0..post_data.len() {
        if pre_data.get(offset) == post_data.get(offset) {
            continue;
        }
        *changed_data_bytes += 1;
        let authorized = contract
            .transition
            .data
            .ranges
            .iter()
            .any(|range| range.contains(offset as u64));
        if !authorized {
            violations.push(ViolationV2::DataWriteOutsidePolicy {
                role: role_name.to_string(),
                position,
                offset: offset as u32,
            });
        }
    }
}

fn owner_transition_ok(
    policy: &OwnerPolicyV2,
    pre: &AccountStateV2,
    post: &AccountStateV2,
    program_id: &[u8; 32],
) -> bool {
    match policy {
        OwnerPolicyV2::MayChange => true,
        OwnerPolicyV2::Preserve => owner(pre) == owner(post),
        OwnerPolicyV2::SetTo { target } => {
            let Some(post_owner) = owner(post) else {
                return false;
            };
            match target {
                OwnerTargetV2::Exact { address } => post_owner == address,
                OwnerTargetV2::ProgramId => post_owner == program_id,
                OwnerTargetV2::SystemProgram => post_owner == &[0; 32],
            }
        }
    }
}

fn executable_transition_ok(
    policy: &ExecutablePolicyV2,
    pre: &AccountStateV2,
    post: &AccountStateV2,
) -> bool {
    match policy {
        ExecutablePolicyV2::MayChange => true,
        ExecutablePolicyV2::Preserve => executable(pre) == executable(post),
        ExecutablePolicyV2::Set { value } => executable(post) == Some(*value),
    }
}

fn owner(state: &AccountStateV2) -> Option<&[u8; 32]> {
    match state {
        AccountStateV2::Absent => None,
        AccountStateV2::Present { owner, .. } => Some(owner),
    }
}

fn executable(state: &AccountStateV2) -> Option<bool> {
    match state {
        AccountStateV2::Absent => None,
        AccountStateV2::Present { executable, .. } => Some(*executable),
    }
}

fn data_or_empty(state: &AccountStateV2) -> &[u8] {
    match state {
        AccountStateV2::Absent => &[],
        AccountStateV2::Present { data, .. } => data,
    }
}
