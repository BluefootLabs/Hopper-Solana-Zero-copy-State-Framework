//! Strict host-semantic adapter for Cicada's manifest-derived fuzz plan.
//!
//! This binary does not claim SBF execution. The CLI/process pipeline
//! recomputes the plan from the input manifest before launching this adapter.
//! The adapter independently matches every request and case to the committed
//! plan and Cicada's live manifest, then pairs it with seeded execution of
//! Cicada's actual private business guards. Unsupported cases or invariant
//! names fail closed.

use hopper_schema::accounts::{AccountLifecycle, ContextDescriptor};
use hopper_schema::{ArgEncoding, InstructionDescriptor, ProgramManifest, WriteRange};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read};

const REQUEST_SCHEMA: &str = "hopper.manifest-fuzz-request.v1";
const RESPONSE_SCHEMA: &str = "hopper.manifest-fuzz-response.v1";
const PLAN_SCHEMA: &str = "hopper.manifest-fuzz-plan.v2";
const BUSINESS_INVARIANT: &str = "cicada-business-semantics";
const COMMITTED_PLAN: &str = include_str!("../../../fuzz/plans/hopper-cicada.plan.json");

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    schema: String,
    program: String,
    program_version: String,
    contract_commitment: String,
    coverage: serde_json::Value,
    cases: Vec<FuzzCase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema: String,
    program: String,
    program_version: String,
    contract_commitment: String,
    cases: Vec<FuzzCase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FuzzCase {
    id: String,
    seed: String,
    kind: String,
    target: String,
    expectation: String,
    required_invariants: Vec<String>,
    #[serde(default)]
    parameters: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    schema: &'static str,
    contract_commitment: String,
    results: Vec<ResultRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Outcome {
    Passed,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultRecord {
    id: String,
    outcome: Outcome,
    checked_invariants: Vec<String>,
    detail: String,
}

fn main() {
    if let Err(error) = run_stdio() {
        eprintln!("cicada fuzz adapter: {error}");
        std::process::exit(1);
    }
}

fn run_stdio() -> Result<(), String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("cannot read request: {error}"))?;
    let request: Request =
        serde_json::from_str(&input).map_err(|error| format!("invalid request JSON: {error}"))?;
    let response = evaluate_request(request)?;
    serde_json::to_writer(io::stdout().lock(), &response)
        .map_err(|error| format!("cannot write response: {error}"))?;
    Ok(())
}

fn evaluate_request(request: Request) -> Result<Response, String> {
    let plan = committed_plan()?;
    validate_request_identity(&request, &plan)?;

    let canonical: BTreeMap<&str, &FuzzCase> = plan
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect();
    if canonical.len() != plan.cases.len() {
        return Err("committed plan contains duplicate case IDs".to_string());
    }

    let mut requested = BTreeSet::new();
    let mut results = Vec::with_capacity(request.cases.len());
    for case in &request.cases {
        if !requested.insert(case.id.as_str()) {
            return Err(format!("request contains duplicate case `{}`", case.id));
        }
        let result = match canonical.get(case.id.as_str()) {
            Some(expected) if case_matches_committed(case, expected) => {
                evaluate_case_catching_panics(case)
            }
            Some(_) => failed(case, "requested case differs from the committed plan"),
            None => failed(case, "requested case is absent from the committed plan"),
        };
        results.push(result);
    }

    if results.len() != request.cases.len() {
        return Err("adapter did not produce exactly one result per case".to_string());
    }
    Ok(Response {
        schema: RESPONSE_SCHEMA,
        contract_commitment: request.contract_commitment,
        results,
    })
}

fn committed_plan() -> Result<Plan, String> {
    let plan: Plan = serde_json::from_str(COMMITTED_PLAN)
        .map_err(|error| format!("embedded committed plan is invalid: {error}"))?;
    if plan.schema != PLAN_SCHEMA {
        return Err(format!(
            "unsupported embedded plan schema `{}`",
            plan.schema
        ));
    }
    let generated_cases = plan
        .coverage
        .get("generatedCases")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "embedded plan lacks coverage.generatedCases".to_string())?;
    if generated_cases != plan.cases.len() as u64 {
        return Err("embedded plan case count differs from its coverage".to_string());
    }
    Ok(plan)
}

fn case_matches_committed(requested: &FuzzCase, committed: &FuzzCase) -> bool {
    requested.id == committed.id
        && requested.seed == committed.seed
        && requested.kind == committed.kind
        && requested.target == committed.target
        && requested.expectation == committed.expectation
        && requested.parameters == committed.parameters
        && committed
            .required_invariants
            .iter()
            .all(|invariant| requested.required_invariants.contains(invariant))
}

fn validate_request_identity(request: &Request, plan: &Plan) -> Result<(), String> {
    let manifest = &hopper_cicada::PROGRAM_MANIFEST;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!("unsupported request schema `{}`", request.schema));
    }
    if request.program != manifest.name || plan.program != manifest.name {
        return Err("request, committed plan, and live Cicada program names differ".to_string());
    }
    if request.program_version != manifest.version || plan.program_version != manifest.version {
        return Err("request, committed plan, and live Cicada versions differ".to_string());
    }
    if request.contract_commitment != plan.contract_commitment {
        return Err("request commitment differs from the committed plan".to_string());
    }
    Ok(())
}

fn evaluate_case_catching_panics(case: &FuzzCase) -> ResultRecord {
    match std::panic::catch_unwind(|| evaluate_case(case)) {
        Ok(Ok(detail)) => ResultRecord {
            id: case.id.clone(),
            outcome: Outcome::Passed,
            checked_invariants: case.required_invariants.clone(),
            detail,
        },
        Ok(Err(error)) => failed(case, &error),
        Err(_) => failed(case, "semantic probe panicked"),
    }
}

fn evaluate_case(case: &FuzzCase) -> Result<String, String> {
    validate_invariants(case)?;
    validate_structural_case(case, &hopper_cicada::PROGRAM_MANIFEST)?;
    let seed = parse_seed(&case.seed)?;
    hopper_cicada::fuzz_semantics::exercise_business_guards(seed)
        .map_err(|error| format!("Cicada business guard probe failed: {error}"))?;
    Ok(format!(
        "host-semantic:{}; live-manifest + Cicada private business guards; not SBF execution",
        case.kind
    ))
}

fn validate_invariants(case: &FuzzCase) -> Result<(), String> {
    let supported = [
        "account-constraints",
        "alias-safety",
        "atomic-rejection",
        BUSINESS_INVARIANT,
        "field-bounds",
        "instruction-parser",
        "layout-gate",
        "no-panic",
        "receipt-integrity",
        "remaining-account-bounds",
        "write-confinement",
    ];
    let mut seen = BTreeSet::new();
    for invariant in &case.required_invariants {
        if !seen.insert(invariant.as_str()) {
            return Err(format!("duplicate required invariant `{invariant}`"));
        }
        if !supported.contains(&invariant.as_str()) {
            return Err(format!("unsupported required invariant `{invariant}`"));
        }
    }
    Ok(())
}

fn validate_structural_case(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    match case.kind.as_str() {
        "layout-truncation" => validate_layout_truncation(case, manifest),
        "layout-identity" => validate_layout_identity(case, manifest),
        "field-boundary" => validate_field_boundary(case, manifest),
        "instruction-truncation" => validate_instruction_truncation(case, manifest),
        "argument-boundary" => validate_argument_boundary(case, manifest),
        "bounded-argument" => validate_bounded_argument(case, manifest),
        "account-privilege" => validate_account_privilege(case, manifest),
        "typed-account-substitution" => validate_typed_account(case, manifest),
        "pda-substitution" => validate_pda(case, manifest),
        "account-lifecycle" => validate_lifecycle(case, manifest),
        "has-one-substitution" => validate_has_one(case, manifest),
        "duplicate-account-alias" => validate_alias(case, manifest),
        "write-range-boundary" => validate_write_boundary(case, manifest),
        "write-range-escape" => validate_write_escape(case, manifest),
        "parametric-write-selector" => validate_parametric(case, manifest),
        "remaining-account-boundary" => validate_remaining(case, manifest),
        "receipt-contract" => validate_receipt(case, manifest),
        other => Err(format!("unsupported Cicada case kind `{other}`")),
    }
}

fn validate_layout_truncation(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-reject-without-panic")?;
    let layout = find_layout(manifest, &case.target)?;
    let length = parameter(case, "length")?;
    require(
        length < layout.total_size as u64,
        "truncation length is not short",
    )
}

fn validate_layout_identity(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-reject")?;
    let layout = find_layout(manifest, &case.target)?;
    if case.parameters.len() != 1 {
        return Err("layout identity probe has an unsupported parameter shape".to_string());
    }
    let (name, value) = case.parameters.iter().next().expect("one parameter");
    match name.as_str() {
        "disc" => require(
            *value <= u8::MAX as u64 && *value as u8 != layout.disc,
            "identity probe does not change the discriminator",
        ),
        "version" => require(
            *value <= u8::MAX as u64 && *value as u8 != layout.version,
            "identity probe does not change the version",
        ),
        "flipByte" => require(
            *value < layout.layout_id.len() as u64,
            "layout-id flip is out of bounds",
        ),
        _ => Err("layout identity probe has an unsupported parameter shape".to_string()),
    }
}

fn validate_field_boundary(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-remain-in-layout")?;
    let target = case
        .target
        .strip_prefix("layout:")
        .ok_or_else(|| "field target lacks layout prefix".to_string())?;
    let (layout_name, field_name) = target
        .split_once('.')
        .ok_or_else(|| "field target lacks a field name".to_string())?;
    let layout = manifest
        .layouts
        .iter()
        .find(|layout| layout.name == layout_name)
        .ok_or_else(|| format!("unknown layout `{layout_name}`"))?;
    let field = layout
        .fields
        .iter()
        .find(|field| field.name == field_name)
        .ok_or_else(|| format!("unknown field `{field_name}`"))?;
    let offset = parameter(case, "offset")?;
    let size = parameter(case, "size")?;
    let end = offset
        .checked_add(size)
        .ok_or_else(|| "field boundary overflows".to_string())?;
    let field_start = u64::from(field.offset);
    let field_end = field_start + u64::from(field.size);
    require(
        offset >= field_start && end <= field_end && end <= layout.total_size as u64,
        "field probe leaves its declared field or layout",
    )?;
    if let Some(layout_size) = case.parameters.get("layoutSize") {
        require(
            *layout_size == layout.total_size as u64,
            "field probe publishes the wrong layout size",
        )?;
    }
    Ok(())
}

fn validate_instruction_truncation(
    case: &FuzzCase,
    manifest: &ProgramManifest,
) -> Result<(), String> {
    expect(case, "must-reject-without-panic")?;
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    require(suffix.is_none(), "instruction truncation names an argument")?;
    require(
        parameter(case, "length")? < instruction.discriminator.len() as u64,
        "instruction discriminator is not truncated",
    )
}

fn validate_argument_boundary(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-reject-without-panic")?;
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    let arg_name = suffix.ok_or_else(|| "argument probe lacks an argument target".to_string())?;
    let mut wire_offset = instruction.discriminator.len() as u64;
    for arg in instruction.args {
        if arg.name == arg_name {
            require(
                arg.encoding == ArgEncoding::Fixed,
                "argument is not fixed-width",
            )?;
            return require(
                arg.size > 0 && parameter(case, "length")? == wire_offset + u64::from(arg.size) - 1,
                "fixed argument is not truncated at its final byte",
            );
        }
        wire_offset += u64::from(arg.size);
    }
    Err(format!("unknown argument `{arg_name}`"))
}

fn validate_bounded_argument(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    let arg_name = suffix.ok_or_else(|| "bounded probe lacks an argument target".to_string())?;
    let arg = instruction
        .args
        .iter()
        .find(|arg| arg.name == arg_name)
        .ok_or_else(|| format!("unknown argument `{arg_name}`"))?;
    let selected = parameter(case, "selectedLength")?;
    let published_max = parameter(case, "maxLength")?;
    let (max, element_size) = match arg.encoding {
        ArgEncoding::BoundedVec {
            max_len,
            element_size,
        } => (u64::from(max_len), Some(u64::from(element_size))),
        ArgEncoding::BoundedString { max_len } => (u64::from(max_len), None),
        ArgEncoding::Fixed => return Err("bounded probe targets a fixed argument".to_string()),
    };
    require(
        max == published_max,
        "bounded probe max differs from manifest",
    )?;
    if let Some(element_size) = element_size {
        require(
            parameter(case, "elementSize")? == element_size,
            "bounded vector element size differs from manifest",
        )?;
    }
    let expected = if selected <= max {
        if element_size.is_some() {
            "must-parse-when-payload-fits"
        } else {
            "must-validate-utf8-and-bounds"
        }
    } else {
        "must-reject"
    };
    expect(case, expected)
}

fn validate_account_privilege(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    let (instruction, suffix, account_index) = account_target(case, manifest)?;
    let account = &instruction.accounts[account_index];
    require(
        suffix == account.name,
        "privilege target does not match account index",
    )?;
    if case.id.ends_with("-missing-signer") {
        expect(case, "must-reject")?;
        require(account.signer, "missing-signer probe targets a non-signer")
    } else if case.id.ends_with("-readonly") {
        expect(case, "must-reject-before-write")?;
        require(
            account.writable,
            "readonly probe targets a readonly account",
        )
    } else {
        Err("unknown account privilege mutation".to_string())
    }
}

fn validate_typed_account(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-reject-before-typed-access")?;
    let (instruction, suffix, account_index) = account_target(case, manifest)?;
    let context = matching_context(instruction, manifest.contexts);
    let account = &instruction.accounts[account_index];
    let layout_ref = if account.layout_ref.is_empty() {
        context
            .and_then(|context| context.accounts.get(account_index))
            .map_or("", |account| account.layout_ref)
    } else {
        account.layout_ref
    };
    require(
        suffix == account.name,
        "typed target does not match account index",
    )?;
    require(
        !layout_ref.is_empty(),
        "typed probe targets an untyped account",
    )?;
    require(
        manifest
            .layouts
            .iter()
            .any(|layout| layout.name == layout_ref),
        "typed account references an unpublished layout",
    )
}

fn validate_pda(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-reject")?;
    let (instruction, suffix, account_index) = account_target(case, manifest)?;
    let context = matching_context(instruction, manifest.contexts);
    let account = &instruction.accounts[account_index];
    let seeds = if account.seeds.is_empty() {
        context
            .and_then(|context| context.accounts.get(account_index))
            .map_or(&[][..], |account| account.seeds)
    } else {
        account.seeds
    };
    require(
        suffix == account.name,
        "PDA target does not match account index",
    )?;
    require(!seeds.is_empty(), "PDA probe targets a provided account")?;
    require(
        parameter(case, "seedCount")? == seeds.len() as u64,
        "PDA seed count differs from live manifest",
    )
}

fn validate_lifecycle(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    let (instruction, suffix, account_index) = account_target(case, manifest)?;
    let context = matching_context(instruction, manifest.contexts)
        .ok_or_else(|| "lifecycle probe has no matching context".to_string())?;
    let account = context
        .accounts
        .get(account_index)
        .ok_or_else(|| "lifecycle account index is out of bounds".to_string())?;
    require(
        suffix == account.name,
        "lifecycle target does not match account index",
    )?;
    let (code, expectation) = match account.lifecycle {
        AccountLifecycle::Existing => {
            return Err("lifecycle probe targets existing account".to_string())
        }
        AccountLifecycle::Init => (1, "must-reject-preinitialized-account-before-write"),
        AccountLifecycle::InitIfNeeded => (
            2,
            "must-reject-invalid-existing-account-or-initialize-atomically",
        ),
        AccountLifecycle::Realloc => (3, "must-reject-invalid-resize-and-preserve-account"),
        AccountLifecycle::Close => (4, "must-close-only-after-success-and-preserve-recipient"),
    };
    expect(case, expectation)?;
    require(
        parameter(case, "lifecycle")? == code,
        "wrong lifecycle code",
    )?;
    require(
        parameter(case, "initSpace")? == u64::from(account.init_space),
        "wrong lifecycle init space",
    )?;
    let payer_index = context
        .accounts
        .iter()
        .position(|candidate| candidate.name == account.payer)
        .map_or(u64::MAX, |index| index as u64);
    require(
        parameter(case, "payerIndex")? == payer_index,
        "wrong lifecycle payer index",
    )
}

fn validate_has_one(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-reject-related-key-mismatch-before-write")?;
    let (instruction, suffix, account_index) = account_target(case, manifest)?;
    let (account_name, related_name) = suffix
        .split_once("->")
        .ok_or_else(|| "has-one target lacks related account".to_string())?;
    let context = matching_context(instruction, manifest.contexts)
        .ok_or_else(|| "has-one probe has no matching context".to_string())?;
    let account = context
        .accounts
        .get(account_index)
        .ok_or_else(|| "has-one account index is out of bounds".to_string())?;
    require(
        account.name == account_name,
        "has-one target account differs",
    )?;
    require(
        account.has_one.contains(&related_name),
        "related account is absent from has-one constraints",
    )?;
    let related_index = context
        .accounts
        .iter()
        .position(|candidate| candidate.name == related_name)
        .ok_or_else(|| "related account is absent from context".to_string())?;
    require(
        parameter(case, "relatedAccountIndex")? == related_index as u64,
        "wrong related account index",
    )
}

fn validate_alias(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-preserve-borrow-and-role-invariants")?;
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    require(
        suffix.is_none(),
        "alias target unexpectedly names an account",
    )?;
    let left = parameter(case, "left")? as usize;
    let right = parameter(case, "right")? as usize;
    require(left < right, "alias pair is not canonical")?;
    let left_role = instruction
        .accounts
        .get(left)
        .ok_or_else(|| "left alias index is out of bounds".to_string())?;
    let right_role = instruction
        .accounts
        .get(right)
        .ok_or_else(|| "right alias index is out of bounds".to_string())?;
    require(
        left_role.name != right_role.name,
        "alias roles are not distinct",
    )?;
    // The seeded application probe paired with this structural check executes
    // Cicada's real duplicate-meta guard on identical and conflicting flags.
    // Here we prove the generated pair names two real, independently declared
    // roles instead of treating a duplicated index as alias coverage.
    Ok(())
}

fn validate_write_boundary(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-authorize")?;
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    require(suffix.is_none(), "write target unexpectedly names a field")?;
    let ranges = effective_write_ranges(instruction, manifest);
    let account_index = parameter_u8(case, "accountIndex")?;
    let offset = parameter_u32(case, "offset")?;
    let size = parameter_u32(case, "size")?;
    require(
        ranges.iter().any(|range| {
            range.account_index == account_index
                && range.offset == offset
                && range.size == size
                && range.contains(offset, size)
        }),
        "exact write probe differs from live authorized range",
    )
}

fn validate_write_escape(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-reject-before-write")?;
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    require(suffix.is_none(), "write escape unexpectedly names a field")?;
    let ranges = effective_write_ranges(instruction, manifest);
    let account_index = parameter_u8(case, "accountIndex")?;
    let offset = parameter_u32(case, "offset")?;
    let size = parameter_u32(case, "size")?;
    require(size == 1, "write escape is not a one-byte boundary probe")?;
    require(
        !ranges
            .iter()
            .any(|range| range.account_index == account_index && range.contains(offset, size)),
        "write escape is authorized by the effective range union",
    )?;
    require(
        ranges.iter().any(|range| {
            if range.account_index != account_index {
                return false;
            }
            let request = u64::from(offset);
            request + 1 == u64::from(range.offset)
                || (range.size != u32::MAX
                    && request == u64::from(range.offset) + u64::from(range.size))
        }),
        "write escape is not adjacent to an authorized boundary",
    )
}

fn validate_parametric(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    let segment = suffix.ok_or_else(|| "parametric target lacks segment".to_string())?;
    let context = matching_context(instruction, manifest.contexts);
    let ranges = if instruction.parametric_write_ranges.is_empty() {
        context.map_or(&[][..], |context| context.parametric_write_ranges)
    } else {
        instruction.parametric_write_ranges
    };
    let account_index = parameter_u8(case, "accountIndex")?;
    let count = parameter_u32(case, "count")?;
    let stride = parameter_u32(case, "stride")?;
    let cell_size = parameter_u32(case, "cellSize")?;
    let range = ranges
        .iter()
        .find(|range| {
            range.segment_name == segment
                && range.account_index == account_index
                && range.count == count
                && range.stride == stride
                && range.cell_size == cell_size
        })
        .ok_or_else(|| "parametric probe differs from live selector".to_string())?;
    let selected = parameter_u32(case, "selected")?;
    if selected < range.count {
        expect(case, "must-authorize-only-selected-cell")?;
        let selected_start =
            u64::from(range.base_offset) + u64::from(selected) * u64::from(range.stride);
        let selected_end = selected_start + u64::from(range.cell_size);
        require(
            selected_end > selected_start && selected_end <= u64::from(u32::MAX) + 1,
            "selected parametric cell is invalid",
        )
    } else {
        expect(case, "must-reject-before-write")?;
        require(
            selected == range.count,
            "overflow selector is not the first invalid cell",
        )
    }
}

fn validate_remaining(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    require(suffix.is_none(), "remaining-account target names a field")?;
    let remaining = instruction
        .remaining_accounts
        .ok_or_else(|| "instruction has no remaining-account suffix".to_string())?;
    let count = parameter(case, "count")?;
    let max = parameter(case, "max")?;
    require(
        max == u64::from(remaining.max),
        "remaining-account max differs",
    )?;
    expect(
        case,
        if count <= max {
            "must-accept-count"
        } else {
            "must-reject-before-dispatch"
        },
    )?;
    require(
        count <= max || count == max + 1,
        "overflow count is not a boundary",
    )
}

fn validate_receipt(case: &FuzzCase, manifest: &ProgramManifest) -> Result<(), String> {
    expect(case, "must-emit-verifiable-receipt-on-success")?;
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    require(suffix.is_none(), "receipt target names a field")?;
    let context = matching_context(instruction, manifest.contexts);
    require(
        instruction.receipt_expected || context.is_some_and(|context| context.receipts_expected),
        "receipt probe targets an instruction without a receipt contract",
    )
}

fn find_layout<'a>(
    manifest: &'a ProgramManifest,
    target: &str,
) -> Result<&'a hopper_schema::LayoutManifest, String> {
    let name = target
        .strip_prefix("layout:")
        .ok_or_else(|| "layout target lacks prefix".to_string())?;
    manifest
        .layouts
        .iter()
        .find(|layout| layout.name == name)
        .ok_or_else(|| format!("unknown layout `{name}`"))
}

fn find_instruction<'a>(
    manifest: &'a ProgramManifest,
    target: &'a str,
) -> Result<(&'a InstructionDescriptor, Option<&'a str>), String> {
    let target = target
        .strip_prefix("instruction:")
        .ok_or_else(|| "instruction target lacks prefix".to_string())?;
    let (name, suffix) = target
        .split_once('.')
        .map_or((target, None), |(name, suffix)| (name, Some(suffix)));
    let instruction = manifest
        .instructions
        .iter()
        .find(|instruction| instruction.name == name)
        .ok_or_else(|| format!("unknown instruction `{name}`"))?;
    Ok((instruction, suffix))
}

fn account_target<'a>(
    case: &'a FuzzCase,
    manifest: &'a ProgramManifest,
) -> Result<(&'a InstructionDescriptor, &'a str, usize), String> {
    let (instruction, suffix) = find_instruction(manifest, &case.target)?;
    let suffix = suffix.ok_or_else(|| "account case lacks an account target".to_string())?;
    let index = parameter(case, "accountIndex")? as usize;
    if index >= instruction.accounts.len() {
        return Err("account index is out of bounds".to_string());
    }
    Ok((instruction, suffix, index))
}

fn effective_write_ranges<'a>(
    instruction: &'a InstructionDescriptor,
    manifest: &'a ProgramManifest,
) -> &'a [WriteRange] {
    if instruction.write_ranges.is_empty() {
        matching_context(instruction, manifest.contexts)
            .map_or(&[][..], |context| context.write_ranges)
    } else {
        instruction.write_ranges
    }
}

fn matching_context<'a>(
    instruction: &InstructionDescriptor,
    contexts: &'a [ContextDescriptor],
) -> Option<&'a ContextDescriptor> {
    let instruction_name = canonical_identifier(instruction.name);
    if let Some(named) = contexts
        .iter()
        .filter(|context| context_matches_instruction(instruction, context))
        .find(|context| canonical_identifier(context.name) == instruction_name)
    {
        return Some(named);
    }
    let mut candidates = contexts
        .iter()
        .filter(|context| context_matches_instruction(instruction, context));
    let only = candidates.next()?;
    candidates.next().is_none().then_some(only)
}

fn context_matches_instruction(
    instruction: &InstructionDescriptor,
    context: &ContextDescriptor,
) -> bool {
    context.accounts.len() == instruction.accounts.len()
        && instruction.accounts.iter().zip(context.accounts).all(
            |(instruction_account, context_account)| {
                instruction_account.name == context_account.name
                    && instruction_account.writable == context_account.writable
                    && instruction_account.signer == context_account.signer
                    && (instruction_account.layout_ref.is_empty()
                        || context_account.layout_ref.is_empty()
                        || instruction_account.layout_ref == context_account.layout_ref)
            },
        )
}

fn canonical_identifier(value: &str) -> String {
    value
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn parameter(case: &FuzzCase, name: &str) -> Result<u64, String> {
    case.parameters
        .get(name)
        .copied()
        .ok_or_else(|| format!("case is missing `{name}`"))
}

fn parameter_u8(case: &FuzzCase, name: &str) -> Result<u8, String> {
    u8::try_from(parameter(case, name)?).map_err(|_| format!("`{name}` does not fit u8"))
}

fn parameter_u32(case: &FuzzCase, name: &str) -> Result<u32, String> {
    u32::try_from(parameter(case, name)?).map_err(|_| format!("`{name}` does not fit u32"))
}

fn expect(case: &FuzzCase, expected: &str) -> Result<(), String> {
    require(
        case.expectation == expected,
        &format!(
            "expectation `{}` differs from semantic result `{expected}`",
            case.expectation
        ),
    )
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

fn parse_seed(seed: &str) -> Result<[u8; 16], String> {
    if seed.len() != 32 {
        return Err("case seed is not 128-bit lowercase hex".to_string());
    }
    let mut out = [0u8; 16];
    for (index, byte) in out.iter_mut().enumerate() {
        let high = hex_nibble(seed.as_bytes()[index * 2])?;
        let low = hex_nibble(seed.as_bytes()[index * 2 + 1])?;
        *byte = high << 4 | low;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err("case seed is not lowercase hexadecimal".to_string()),
    }
}

fn failed(case: &FuzzCase, detail: &str) -> ResultRecord {
    ResultRecord {
        id: case.id.clone(),
        outcome: Outcome::Failed,
        checked_invariants: Vec::new(),
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_request() -> Request {
        let plan = committed_plan().unwrap();
        Request {
            schema: REQUEST_SCHEMA.to_string(),
            program: plan.program,
            program_version: plan.program_version,
            contract_commitment: plan.contract_commitment,
            cases: plan
                .cases
                .into_iter()
                .map(|mut case| {
                    case.required_invariants
                        .push(BUSINESS_INVARIANT.to_string());
                    case.required_invariants.sort();
                    case
                })
                .collect(),
        }
    }

    #[test]
    fn committed_plan_runs_without_skips_or_blanket_acceptance() {
        let request = complete_request();
        let expected = request.cases.len();
        let response = evaluate_request(request).unwrap();
        assert_eq!(response.results.len(), expected);
        assert!(response
            .results
            .iter()
            .all(|result| result.outcome == Outcome::Passed));
        assert!(response.results.iter().all(|result| {
            result
                .checked_invariants
                .contains(&BUSINESS_INVARIANT.to_string())
                && result.detail.contains("not SBF execution")
        }));
        let kinds: BTreeSet<_> = committed_plan()
            .unwrap()
            .cases
            .into_iter()
            .map(|case| case.kind)
            .collect();
        assert_eq!(kinds.len(), 17);
    }

    #[test]
    fn modified_unknown_and_duplicate_cases_fail_closed() {
        let mut modified = complete_request();
        modified.cases.truncate(1);
        modified.cases[0].expectation = "accept-everything".to_string();
        let response = evaluate_request(modified).unwrap();
        assert_eq!(response.results[0].outcome, Outcome::Failed);
        assert!(response.results[0].checked_invariants.is_empty());

        let mut unknown_invariant = complete_request();
        unknown_invariant.cases.truncate(1);
        unknown_invariant.cases[0]
            .required_invariants
            .push("adapter-echo".to_string());
        // The canonical mismatch itself is fail-closed before semantic hooks.
        let response = evaluate_request(unknown_invariant).unwrap();
        assert_eq!(response.results[0].outcome, Outcome::Failed);

        let mut duplicate = complete_request();
        duplicate.cases.truncate(1);
        duplicate.cases.push(duplicate.cases[0].clone());
        assert!(evaluate_request(duplicate)
            .unwrap_err()
            .contains("duplicate case"));
    }

    #[test]
    fn every_emitted_write_escape_is_outside_the_live_union() {
        let plan = committed_plan().unwrap();
        for case in plan
            .cases
            .iter()
            .filter(|case| case.kind == "write-range-escape")
        {
            validate_write_escape(case, &hopper_cicada::PROGRAM_MANIFEST)
                .unwrap_or_else(|error| panic!("{}: {error}", case.id));
        }
    }
}
