use grillo_manifest::{
    AccountRoleContractV2, AddressConstraintV2, ContractCompletenessV2, CpiPolicyV2,
    DataPolicyV2, DataRangeV2, DeploymentBindingV2, DuplicatePolicyV2, EffectContractV2,
    EffectContractV2Error, ExecutablePolicyV2, InstructionEffectContractV2, LamportPolicyV2,
    LengthPolicyV2, OwnerPolicyV2, PresencePolicyV2, PrivilegeRequirementV2,
    RemainingAccountsContractV2, RemainingGroupV2, TransitionPolicyV2, EFFECT_ABI_V2,
};

fn transition() -> TransitionPolicyV2 {
    TransitionPolicyV2 {
        data: DataPolicyV2 {
            ranges: vec![DataRangeV2 { offset: 8, size: 4 }],
        },
        lamports: LamportPolicyV2::Preserve,
        owner: OwnerPolicyV2::Preserve,
        data_length: LengthPolicyV2::Preserve,
        presence: PresencePolicyV2::MustRemainPresent,
        executable: ExecutablePolicyV2::Preserve,
    }
}

fn role(name: &str) -> AccountRoleContractV2 {
    AccountRoleContractV2 {
        name: name.into(),
        signer: PrivilegeRequirementV2::Forbidden,
        writable: PrivilegeRequirementV2::Required,
        address: AddressConstraintV2::Any,
        transition: transition(),
    }
}

fn contract() -> EffectContractV2 {
    EffectContractV2 {
        effect_abi_version: EFFECT_ABI_V2.into(),
        deployment: DeploymentBindingV2::ExactDeployment {
            program_id: [1; 32],
            loader_id: [2; 32],
            executable_digest: [3; 32],
        },
        instructions: vec![InstructionEffectContractV2 {
            name: "execute".into(),
            discriminator: vec![0x12, 0x34, 0x56],
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
            accounts: vec![role("state")],
            remaining_accounts: RemainingAccountsContractV2 {
                complete: true,
                allow_trailing: false,
                duplicate_policy: DuplicatePolicyV2::DenyWritable,
                groups: vec![RemainingGroupV2 {
                    name: "hop".into(),
                    repeat: 2,
                    roles: vec![role("pool"), role("vault")],
                }],
            },
            cpi: CpiPolicyV2::Forbidden,
        }],
    }
}

#[test]
fn strict_json_requires_and_round_trips_full_discriminator_bytes() {
    let json = serde_json::to_string(&contract()).unwrap();
    assert!(json.contains("\"discriminatorBytes\":[18,52,86]"));
    assert!(!json.contains("\"discriminator\":"));
    let parsed = EffectContractV2::from_json(&json).unwrap();
    assert_eq!(parsed.instructions[0].discriminator, [0x12, 0x34, 0x56]);

    let mut missing: serde_json::Value = serde_json::from_str(&json).unwrap();
    missing["instructions"][0]
        .as_object_mut()
        .unwrap()
        .remove("discriminatorBytes");
    let error = EffectContractV2::from_json(&missing.to_string()).unwrap_err();
    assert!(matches!(error, EffectContractV2Error::Json(_)));
}

#[test]
fn strict_json_rejects_unknown_fields_and_wrong_version() {
    let mut value = serde_json::to_value(contract()).unwrap();
    value["surprise"] = serde_json::json!(true);
    assert!(matches!(
        EffectContractV2::from_json(&value.to_string()),
        Err(EffectContractV2Error::Json(_))
    ));

    let mut value = serde_json::to_value(contract()).unwrap();
    value["effectAbiVersion"] = serde_json::json!("0.3");
    assert!(matches!(
        EffectContractV2::from_json(&value.to_string()),
        Err(EffectContractV2Error::UnsupportedVersion(version)) if version == "0.3"
    ));
}

#[test]
fn grammar_expands_in_fixed_group_iteration_role_order() {
    let roles = contract().instructions[0].expanded_roles();
    let names: Vec<&str> = roles.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "state",
            "hop.pool[0]",
            "hop.vault[0]",
            "hop.pool[1]",
            "hop.vault[1]"
        ]
    );
}

#[test]
fn commitment_covers_deployment_discriminator_transitions_and_grammar() {
    let base = contract().commitment().unwrap();

    let mut changed = contract();
    changed.instructions[0].discriminator[2] ^= 1;
    assert_ne!(base, changed.commitment().unwrap());

    let mut changed = contract();
    changed.instructions[0].accounts[0].transition.owner = OwnerPolicyV2::MayChange;
    assert_ne!(base, changed.commitment().unwrap());

    let mut changed = contract();
    changed.instructions[0].remaining_accounts.groups[0].repeat = 1;
    assert_ne!(base, changed.commitment().unwrap());

    let mut changed = contract();
    changed.deployment = DeploymentBindingV2::ExactArtifact {
        executable_digest: [3; 32],
    };
    assert_ne!(base, changed.commitment().unwrap());
}
