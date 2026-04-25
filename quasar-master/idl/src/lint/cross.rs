//! Cross-instruction field tracking.
use {
    super::{
        constraints::FieldClass,
        types::{Diagnostic, LintRule, Severity, TypeRegistry},
    },
    crate::parser::ParsedProgram,
    std::collections::HashMap,
};

struct TypeUsage {
    instruction_name: String,
    accounts_struct: String,
    field_name: String,
    is_init: bool,
    has_ones: Vec<String>,
    seed_refs: Vec<String>,
    token_mint: Option<String>,
    token_authority: Option<String>,
}

pub fn check_cross_instruction(parsed: &ParsedProgram, registry: &TypeRegistry) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Build map: type_name -> [usages across instructions]
    let mut type_usage: HashMap<String, Vec<TypeUsage>> = HashMap::new();

    for accounts_struct in &parsed.accounts_structs {
        let instruction_name = parsed
            .instructions
            .iter()
            .find(|i| i.accounts_type_name == accounts_struct.name)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| accounts_struct.name.clone());

        for field in &accounts_struct.fields {
            if let FieldClass::Account { ref inner_type } = field.field_class {
                type_usage
                    .entry(inner_type.clone())
                    .or_default()
                    .push(TypeUsage {
                        instruction_name: instruction_name.clone(),
                        accounts_struct: accounts_struct.name.clone(),
                        field_name: field.name.clone(),
                        is_init: field.constraints.is_init,
                        has_ones: field.constraints.has_ones.clone(),
                        seed_refs: field.constraints.seeds_account_refs.clone(),
                        token_mint: field.constraints.token_mint.clone(),
                        token_authority: field.constraints.token_authority.clone(),
                    });
            }
        }
    }

    // For each type used across 2+ instructions with at least one init
    for (type_name, usages) in &type_usage {
        if usages.len() < 2 {
            continue;
        }

        // Skip built-in SPL types — they use token::mint/authority, not has_one
        if matches!(type_name.as_str(), "Token" | "TokenAccount" | "Mint") {
            continue;
        }

        let addr_fields = registry.get_address_fields(type_name);
        if addr_fields.is_empty() {
            continue;
        }

        let init_instructions: Vec<&TypeUsage> = usages.iter().filter(|u| u.is_init).collect();
        if init_instructions.is_empty() {
            continue;
        }

        for addr_field in &addr_fields {
            for usage in usages.iter().filter(|u| !u.is_init) {
                let verified = usage.has_ones.contains(addr_field)
                    || usage.seed_refs.contains(addr_field)
                    || (addr_field == "mint" && usage.token_mint.is_some())
                    || (addr_field == "owner" && usage.token_authority.is_some());

                if !verified {
                    diagnostics.push(Diagnostic {
                        rule: LintRule::L009,
                        severity: Severity::Warning,
                        accounts_struct: usage.accounts_struct.clone(),
                        field: Some(usage.field_name.clone()),
                        message: format!(
                            "Cross-instruction: `{}.{}` is set in `{}` but `{}` reads {} without \
                             verifying `{}`.",
                            type_name,
                            addr_field,
                            init_instructions[0].instruction_name,
                            usage.instruction_name,
                            type_name,
                            addr_field,
                        ),
                        suggestion: Some(format!(
                            "Add `has_one = {}` to `{}` in `{}`.",
                            addr_field, usage.field_name, usage.accounts_struct,
                        )),
                    });
                }
            }
        }
    }

    diagnostics
}
