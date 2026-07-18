use grillo_manifest::{
    AccountRoleContractV2, AddressConstraintV2, ContractCompletenessV2, CpiAccountBindingV2,
    CpiEnvelopeV2, CpiPolicyV2, DataPolicyV2, DataRangeV2, DeploymentBindingV2,
    DuplicatePolicyV2, EffectContractV2, ExecutablePolicyV2, InstructionEffectContractV2,
    LamportPolicyV2, LengthPolicyV2, OwnerPolicyV2, PresencePolicyV2,
    PrivilegeRequirementV2, RemainingAccountsContractV2, RemainingGroupV2, TransitionPolicyV2,
    EFFECT_ABI_V2,
};
use grillo_verifier::{
    bind_invocation_v2, verify_bound_invocation_v2, AccountStateV2, AccountTransitionV2,
    BindErrorV2, DeploymentIdentityV2, EffectVerdictV2, EvidenceCompletenessV2,
    EvidenceProvenanceV2, InvocationAccountRefV2, InvocationFrameV2, InvocationOutcomeV2,
    NetworkIdentityV2, ObservationBoundaryV2, TouchEvidenceV2, TransactionIdentityV2,
    UnverifiedAuthenticityV2, ViolationV2,
};

const PROGRAM: [u8; 32] = [9; 32];
const LOADER: [u8; 32] = [8; 32];
const EXECUTABLE: [u8; 32] = [7; 32];

fn preserve(ranges: Vec<DataRangeV2>) -> TransitionPolicyV2 {
    TransitionPolicyV2 {
        data: DataPolicyV2 { ranges },
        lamports: LamportPolicyV2::Preserve,
        owner: OwnerPolicyV2::Preserve,
        data_length: LengthPolicyV2::Preserve,
        presence: PresencePolicyV2::MustRemainPresent,
        executable: ExecutablePolicyV2::Preserve,
    }
}

fn role(
    name: &str,
    address: AddressConstraintV2,
    signer: PrivilegeRequirementV2,
    writable: PrivilegeRequirementV2,
    ranges: Vec<DataRangeV2>,
) -> AccountRoleContractV2 {
    AccountRoleContractV2 {
        name: name.into(),
        signer,
        writable,
        address,
        transition: preserve(ranges),
    }
}

fn completeness() -> ContractCompletenessV2 {
    ContractCompletenessV2 {
        accounts: true,
        data: true,
        lamports: true,
        owner: true,
        data_length: true,
        presence: true,
        executable: true,
        cpi: true,
    }
}

fn contract() -> EffectContractV2 {
    EffectContractV2 {
        effect_abi_version: EFFECT_ABI_V2.into(),
        deployment: DeploymentBindingV2::ExactDeployment {
            program_id: PROGRAM,
            loader_id: LOADER,
            executable_digest: EXECUTABLE,
        },
        instructions: vec![InstructionEffectContractV2 {
            name: "apply".into(),
            discriminator: vec![0xaa, 0xbb],
            completeness: completeness(),
            accounts: vec![
                role(
                    "authority",
                    AddressConstraintV2::Exact { address: [1; 32] },
                    PrivilegeRequirementV2::Required,
                    PrivilegeRequirementV2::Forbidden,
                    vec![],
                ),
                role(
                    "state",
                    AddressConstraintV2::Exact { address: [2; 32] },
                    PrivilegeRequirementV2::Forbidden,
                    PrivilegeRequirementV2::Required,
                    vec![DataRangeV2 { offset: 1, size: 1 }],
                ),
            ],
            remaining_accounts: RemainingAccountsContractV2 {
                complete: true,
                allow_trailing: false,
                duplicate_policy: DuplicatePolicyV2::DenyWritable,
                groups: vec![RemainingGroupV2 {
                    name: "route".into(),
                    repeat: 1,
                    roles: vec![
                        role(
                            "program",
                            AddressConstraintV2::Exact { address: [3; 32] },
                            PrivilegeRequirementV2::Forbidden,
                            PrivilegeRequirementV2::Forbidden,
                            vec![],
                        ),
                        role(
                            "scratch",
                            AddressConstraintV2::Any,
                            PrivilegeRequirementV2::Forbidden,
                            PrivilegeRequirementV2::Required,
                            vec![DataRangeV2 { offset: 0, size: 1 }],
                        ),
                    ],
                }],
            },
            cpi: CpiPolicyV2::Forbidden,
        }],
    }
}

fn evidence() -> EvidenceCompletenessV2 {
    EvidenceCompletenessV2 {
        accounts: true,
        data: true,
        lamports: true,
        owner: true,
        data_length: true,
        presence: true,
        executable: true,
        cpi: true,
        touch: false,
    }
}

fn present(address_byte: u8, data: &[u8]) -> AccountStateV2 {
    AccountStateV2::Present {
        lamports: 100,
        owner: [address_byte; 32],
        executable: false,
        data: data.to_vec(),
    }
}

fn state(pubkey_byte: u8, owner_byte: u8, pre: &[u8], post: &[u8]) -> AccountTransitionV2 {
    AccountTransitionV2 {
        pubkey: [pubkey_byte; 32],
        pre: present(owner_byte, pre),
        post: present(owner_byte, post),
    }
}

fn frame(contract: &EffectContractV2) -> InvocationFrameV2 {
    InvocationFrameV2 {
        network: NetworkIdentityV2 {
            genesis_hash: [0x11; 32],
        },
        transaction: TransactionIdentityV2 {
            signature: [0x22; 64],
            message_hash: [0x33; 32],
            bank_slot: 42,
            outer_instruction_index: 1,
            invocation_path: vec![],
        },
        deployment: DeploymentIdentityV2 {
            program_id: PROGRAM,
            loader_id: LOADER,
            executable_digest: EXECUTABLE,
            programdata_address: Some([0x44; 32]),
            deployment_slot: Some(40),
            manifest_commitment: contract.commitment().unwrap(),
        },
        instruction_data: vec![0xaa, 0xbb, 0x99],
        accounts: vec![
            InvocationAccountRefV2 {
                position: 0,
                transaction_index: 2,
                pubkey: [1; 32],
                signer: true,
                writable: false,
            },
            InvocationAccountRefV2 {
                position: 1,
                transaction_index: 3,
                pubkey: [2; 32],
                signer: false,
                writable: true,
            },
            InvocationAccountRefV2 {
                position: 2,
                transaction_index: 4,
                pubkey: [3; 32],
                signer: false,
                writable: false,
            },
            InvocationAccountRefV2 {
                position: 3,
                transaction_index: 5,
                pubkey: [4; 32],
                signer: false,
                writable: true,
            },
        ],
        states: vec![
            state(1, 21, &[0], &[0]),
            state(2, 22, &[0, 0], &[0, 1]),
            state(3, 23, &[0], &[0]),
            state(4, 24, &[0], &[0]),
        ],
        touch: TouchEvidenceV2 {
            complete: false,
            digest: None,
        },
        children: vec![],
        outcome: InvocationOutcomeV2::Succeeded,
        boundary: ObservationBoundaryV2::InvocationEntryExit,
        evidence: evidence(),
        provenance: EvidenceProvenanceV2::ReplayClaimed {
            source: "local-fixture".into(),
        },
    }
}

fn state_mut(frame: &mut InvocationFrameV2, key: u8) -> &mut AccountTransitionV2 {
    frame
        .states
        .iter_mut()
        .find(|state| state.pubkey == [key; 32])
        .unwrap()
}

#[test]
fn complete_frame_binds_and_passes_without_authenticity_promotion() {
    let contract = contract();
    let frame = frame(&contract);
    let bound = bind_invocation_v2(&contract, &frame).unwrap();
    assert_eq!(bound.roles()[2].name(), "route.program[0]");
    assert_eq!(bound.roles()[3].name(), "route.scratch[0]");
    assert_eq!(
        bound.authenticity(),
        UnverifiedAuthenticityV2::Unauthenticated
    );
    match verify_bound_invocation_v2(&bound) {
        EffectVerdictV2::Pass(evidence) => {
            assert_eq!(evidence.changed_data_bytes, 1);
            assert_eq!(evidence.provenance, frame.provenance);
            assert_eq!(
                evidence.authenticity,
                UnverifiedAuthenticityV2::Unauthenticated
            );
        }
        other => panic!("expected pass, got {other:?}"),
    }
}

#[test]
fn deployment_manifest_and_full_discriminator_are_bound() {
    let contract = contract();

    let mut wrong = frame(&contract);
    wrong.deployment.program_id[0] ^= 1;
    assert!(matches!(
        bind_invocation_v2(&contract, &wrong),
        Err(BindErrorV2::DeploymentMismatch { field: "program_id" })
    ));

    let mut wrong = frame(&contract);
    wrong.deployment.executable_digest[0] ^= 1;
    assert!(matches!(
        bind_invocation_v2(&contract, &wrong),
        Err(BindErrorV2::DeploymentMismatch { field: "executable_digest" })
    ));

    let mut wrong = frame(&contract);
    wrong.deployment.manifest_commitment[0] ^= 1;
    assert!(matches!(
        bind_invocation_v2(&contract, &wrong),
        Err(BindErrorV2::ManifestCommitmentMismatch)
    ));

    let mut substituted = frame(&contract);
    substituted.instruction_data = vec![0xaa, 0xbc, 0x99];
    assert!(matches!(
        bind_invocation_v2(&contract, &substituted),
        Err(BindErrorV2::InstructionNotFound)
    ));
}

#[test]
fn fixed_and_remaining_roles_reject_reorder_and_omission() {
    let contract = contract();
    let mut reordered = frame(&contract);
    reordered.accounts[2].pubkey = [4; 32];
    reordered.accounts[3].pubkey = [3; 32];
    assert!(matches!(
        bind_invocation_v2(&contract, &reordered),
        Err(BindErrorV2::AddressMismatch { position: 2 })
    ));

    let mut omitted = frame(&contract);
    omitted.accounts.pop();
    omitted.states.pop();
    assert!(matches!(
        bind_invocation_v2(&contract, &omitted),
        Err(BindErrorV2::AccountCount { expected: 4, actual: 3 })
    ));
}

#[test]
fn writable_duplicate_alias_is_rejected_before_verification() {
    let contract = contract();
    let mut aliased = frame(&contract);
    aliased.accounts[3].pubkey = [2; 32];
    aliased.states.pop(); // one unique state for the duplicate address
    assert!(matches!(
        bind_invocation_v2(&contract, &aliased),
        Err(BindErrorV2::DuplicateAccount { first: 1, second: 3 })
    ));
}

#[test]
fn verifier_catches_each_state_dimension() {
    let contract = contract();

    let mut changed = frame(&contract);
    if let AccountStateV2::Present { data, .. } = &mut state_mut(&mut changed, 2).post {
        data[0] = 7;
    }
    assert!(matches!(
        verify_bound_invocation_v2(&bind_invocation_v2(&contract, &changed).unwrap()),
        EffectVerdictV2::Violation(v) if v.iter().any(|v| matches!(v, ViolationV2::DataWriteOutsidePolicy { offset: 0, .. }))
    ));

    let mut changed = frame(&contract);
    if let AccountStateV2::Present { lamports, .. } = &mut state_mut(&mut changed, 2).post {
        *lamports += 1;
    }
    assert!(matches!(
        verify_bound_invocation_v2(&bind_invocation_v2(&contract, &changed).unwrap()),
        EffectVerdictV2::Violation(v) if v.iter().any(|v| matches!(v, ViolationV2::LamportTransition { .. }))
    ));

    let mut changed = frame(&contract);
    if let AccountStateV2::Present { owner, .. } = &mut state_mut(&mut changed, 2).post {
        *owner = [99; 32];
    }
    assert!(matches!(
        verify_bound_invocation_v2(&bind_invocation_v2(&contract, &changed).unwrap()),
        EffectVerdictV2::Violation(v) if v.iter().any(|v| matches!(v, ViolationV2::OwnerTransition { .. }))
    ));

    let mut changed = frame(&contract);
    if let AccountStateV2::Present { data, .. } = &mut state_mut(&mut changed, 2).post {
        data.push(0);
    }
    assert!(matches!(
        verify_bound_invocation_v2(&bind_invocation_v2(&contract, &changed).unwrap()),
        EffectVerdictV2::Violation(v) if v.iter().any(|v| matches!(v, ViolationV2::DataLengthTransition { .. }))
    ));

    let mut changed = frame(&contract);
    state_mut(&mut changed, 2).post = AccountStateV2::Absent;
    assert!(matches!(
        verify_bound_invocation_v2(&bind_invocation_v2(&contract, &changed).unwrap()),
        EffectVerdictV2::Violation(v) if v.iter().any(|v| matches!(v, ViolationV2::PresenceTransition { .. }))
    ));
}

fn child_frame(parent_contract: &EffectContractV2, rolled_back: bool) -> InvocationFrameV2 {
    let mut child = frame(parent_contract);
    child.transaction.invocation_path = vec![0];
    child.deployment = DeploymentIdentityV2 {
        program_id: [55; 32],
        loader_id: [56; 32],
        executable_digest: [57; 32],
        programdata_address: None,
        deployment_slot: Some(41),
        manifest_commitment: [58; 32],
    };
    child.instruction_data = vec![0x10, 0x99];
    child.accounts = vec![InvocationAccountRefV2 {
        position: 0,
        transaction_index: 3,
        pubkey: [2; 32],
        signer: false,
        writable: true,
    }];
    child.states = vec![state(2, 22, &[0, 1], &[0, 1])];
    child.children.clear();
    child.outcome = if rolled_back {
        InvocationOutcomeV2::RolledBack
    } else {
        InvocationOutcomeV2::Succeeded
    };
    child
}

fn declared_cpi_contract() -> EffectContractV2 {
    let mut contract = contract();
    contract.instructions[0].cpi = CpiPolicyV2::Declared {
        complete: true,
        calls: vec![CpiEnvelopeV2 {
            id: "route".into(),
            program_id: [55; 32],
            loader_id: [56; 32],
            executable_digest: [57; 32],
            discriminator: vec![0x10],
            min_calls: 0,
            max_calls: 1,
            allow_rollback: true,
            allow_extra_accounts: false,
            accounts: vec![CpiAccountBindingV2 {
                child_position: 0,
                parent_position: 1,
                signer: PrivilegeRequirementV2::Forbidden,
                writable: PrivilegeRequirementV2::Required,
            }],
        }],
    };
    contract
}

#[test]
fn forbidden_and_undeclared_cpi_are_rejected() {
    let contract = contract();
    let mut with_child = frame(&contract);
    with_child.children.push(child_frame(&contract, false));
    assert!(matches!(
        bind_invocation_v2(&contract, &with_child),
        Err(BindErrorV2::ForbiddenCpi)
    ));

    let contract = declared_cpi_contract();
    let mut with_child = frame(&contract);
    let mut child = child_frame(&contract, false);
    child.deployment.program_id = [66; 32];
    with_child.children.push(child);
    assert!(matches!(
        bind_invocation_v2(&contract, &with_child),
        Err(BindErrorV2::UndeclaredCpi { child_index: 0 })
    ));
}

#[test]
fn rolled_back_child_must_have_complete_unchanged_state() {
    let contract = declared_cpi_contract();
    let mut parent = frame(&contract);
    parent.children.push(child_frame(&contract, true));
    let bound = bind_invocation_v2(&contract, &parent).unwrap();
    assert!(verify_bound_invocation_v2(&bound).is_pass());

    let mut mutated = frame(&contract);
    let mut child = child_frame(&contract, true);
    if let AccountStateV2::Present { data, .. } = &mut child.states[0].post {
        data[0] ^= 1;
    }
    mutated.children.push(child);
    assert!(matches!(
        bind_invocation_v2(&contract, &mutated),
        Err(BindErrorV2::RolledBackMutation { child_index: 0, .. })
    ));
}

#[test]
fn every_frame_and_bound_tamper_changes_commitments() {
    let contract = contract();
    let original = frame(&contract);
    let original_frame_commitment = original.commitment();
    let original_bound = bind_invocation_v2(&contract, &original).unwrap();

    let mut tampered = original.clone();
    tampered.transaction.message_hash[0] ^= 1;
    assert_ne!(original_frame_commitment, tampered.commitment());
    let tampered_bound = bind_invocation_v2(&contract, &tampered).unwrap();
    assert_ne!(original_bound.commitment(), tampered_bound.commitment());

    let mut tampered = original;
    if let AccountStateV2::Present { owner, .. } = &mut tampered.states[0].post {
        owner[0] ^= 1;
    }
    assert_ne!(original_frame_commitment, tampered.commitment());
}
