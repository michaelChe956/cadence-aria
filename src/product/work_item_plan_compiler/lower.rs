use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};

use crate::product::models::{TrustedDraftVerificationCommand, WorkItemDraftVerificationPlan};
use crate::product::work_item_contract::{
    AcceptanceCriterion, BlockerRoute, BlockerRule, CanonicalWorkItemContract,
    ContractCompatibilityPolicy, DesignTraceabilityRef, EvidenceKind, HandoffContract,
    PromisedOutputContract, RequiredInputContract, VerificationCheck, WorkItemContractIdentity,
    WorkItemGoal, WorkItemTask, WorkItemWritePolicy,
};

use super::{
    grammar::WORK_ITEM_PLAN_COMPILER_VERSION,
    types::{CompilerDiagnostic, WorkItemPlanAst, WorkItemPlanFieldAst, WorkItemPlanItemAst},
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidateIr {
    pub source_revision_hash: String,
    pub compiler_version: String,
    pub items: Vec<PlanCandidateItemIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidateItemIr {
    pub target_repository_id: String,
    pub contract: CanonicalWorkItemContract,
    pub verification_plan: WorkItemDraftVerificationPlan,
    pub trusted_commands: Vec<TrustedDraftVerificationCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanSourceContext {
    pub target_repository_id: String,
    pub trusted_command_catalog: Vec<TrustedDraftVerificationCommand>,
}

type TaskState = (String, String, Vec<String>, Vec<String>, usize);
type VerificationState = (
    String,
    Option<String>,
    Option<usize>,
    Option<String>,
    Option<bool>,
    Option<bool>,
    usize,
);

struct LoweredVerificationCheck {
    check: VerificationCheck,
    command_line: usize,
}
type InputState = (
    String,
    Option<String>,
    Vec<String>,
    Option<ContractCompatibilityPolicy>,
    usize,
);

pub fn lower_work_item_plan(
    source: &str,
    ast: WorkItemPlanAst,
    context: &WorkItemPlanSourceContext,
) -> Result<PlanCandidateIr, Vec<CompilerDiagnostic>> {
    let mut diagnostics = Vec::new();
    if context.target_repository_id.trim().is_empty() {
        diagnostics.push(diagnostic(
            "target_repository_id",
            "session context 缺少 target repository。",
            1,
            "target_repository_id: repo-001",
        ));
    }

    let mut catalog_by_ref = HashMap::new();
    for command in &context.trusted_command_catalog {
        if command.source_ref.trim().is_empty() {
            diagnostics.push(diagnostic(
                "trusted_commands",
                "trusted command 的 source_ref 不得为空。",
                1,
                "source_ref: catalog-entry-001",
            ));
        }
        if catalog_by_ref
            .insert(command.source_ref.as_str(), command)
            .is_some()
        {
            diagnostics.push(diagnostic(
                "trusted_commands",
                "trusted command source_ref 不得重复。",
                1,
                "source_ref: catalog-entry-001",
            ));
        }
    }

    let mut items = Vec::with_capacity(ast.items.len());
    for item in &ast.items {
        if let Some(item) = lower_item(
            item,
            &catalog_by_ref,
            context.target_repository_id.clone(),
            &mut diagnostics,
        ) {
            items.push(item);
        }
    }
    if !diagnostics.is_empty() {
        diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.line,
                diagnostic.field.clone(),
                diagnostic.code.clone(),
            )
        });
        return Err(diagnostics);
    }

    Ok(PlanCandidateIr {
        source_revision_hash: hex::encode(Sha256::digest(source.as_bytes())),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        items,
    })
}

pub fn compile_work_item_plan(
    source: &str,
    context: &WorkItemPlanSourceContext,
) -> Result<PlanCandidateIr, Vec<CompilerDiagnostic>> {
    let ast = super::parse_work_item_plan(source)?;
    lower_work_item_plan(source, ast, context)
}

fn lower_item(
    item: &WorkItemPlanItemAst,
    catalog: &HashMap<&str, &TrustedDraftVerificationCommand>,
    target_repository_id: String,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Option<PlanCandidateItemIr> {
    let mut fields = Vec::new();
    for section in &item.sections {
        for field in &section.fields {
            if !super::grammar::STRUCTURED_KEYS.contains(&field.key.value.as_str()) {
                diagnostics.push(diagnostic(
                    &field.key.value,
                    "未知结构化 key 必须拒绝。",
                    field.key.line,
                    "- kind: backend",
                ));
            } else {
                fields.push((section.name.value.as_str(), field));
            }
        }
    }
    let value = |section: &str, key: &str| {
        fields.iter().find_map(|(name, field)| {
            (*name == section && field.key.value == key).then_some(field.value.value.as_str())
        })
    };
    let values = |section: &str, key: &str| {
        fields
            .iter()
            .filter_map(|(name, field)| {
                (*name == section && field.key.value == key).then_some(field.value.value.as_str())
            })
            .collect::<Vec<_>>()
    };
    let line = |section: &str, key: &str| {
        fields
            .iter()
            .find_map(|(name, field)| {
                (*name == section && field.key.value == key).then_some(field.value.line)
            })
            .unwrap_or(item.id.line)
    };
    let required = |section: &str, key: &str, diagnostics: &mut Vec<CompilerDiagnostic>| {
        let result = value(section, key);
        if result.is_none_or(str::is_empty) {
            diagnostics.push(diagnostic(
                &format!("contract.{key}"),
                "必填 markdown 字段缺失。",
                line(section, key),
                &format!("- {key}: value"),
            ));
        }
        result
    };
    let string = |section: &str, key: &str, diagnostics: &mut Vec<CompilerDiagnostic>| {
        required(section, key, diagnostics).map(str::to_string)
    };

    let schema_version = required("Identity", "schema_version", diagnostics)
        .and_then(|value| value.parse::<u32>().ok());
    if schema_version.is_none() && value("Identity", "schema_version").is_some() {
        diagnostics.push(diagnostic(
            "contract.schema_version",
            "schema_version 必须是无符号整数。",
            line("Identity", "schema_version"),
            "- schema_version: 1",
        ));
    }
    let logical_id = string("Identity", "logical_work_item_id", diagnostics);
    let title = string("Identity", "title", diagnostics);
    let kind = string("Identity", "kind", diagnostics);
    let summary = string("Goal", "summary", diagnostics);
    let tasks = lower_tasks(&fields, diagnostics);
    let acceptance_criteria = lower_acceptance_criteria(&fields, diagnostics);
    let verification_checks = lower_verification_checks(&fields, diagnostics);
    let blocker_rules = lower_blocker_rules(&fields, diagnostics);
    let design_traceability = lower_traceability(&fields, diagnostics);
    let input_contracts = lower_inputs(&fields, diagnostics);
    let output_contracts = lower_outputs(&fields, diagnostics);

    let contract = match (schema_version, logical_id, title, kind, summary) {
        (Some(schema_version), Some(logical_id), Some(title), Some(kind), Some(summary)) => {
            CanonicalWorkItemContract {
                schema_version,
                identity: WorkItemContractIdentity {
                    logical_work_item_id: logical_id,
                    title,
                    kind,
                },
                goal: WorkItemGoal { summary },
                non_goals: split_values(values("Non Goals", "non_goals")),
                depends_on: split_values(values("Dependencies", "depends_on")),
                input_contracts,
                output_contracts,
                tasks,
                write_policy: WorkItemWritePolicy {
                    exclusive_scopes: split_values(values("Write Policy", "exclusive_scopes")),
                    forbidden_scopes: split_values(values("Write Policy", "forbidden_scopes")),
                },
                acceptance_criteria,
                verification_checks: verification_checks
                    .iter()
                    .map(|entry| entry.check.clone())
                    .collect(),
                handoff_contract: HandoffContract {
                    required_fields: split_values(values("Handoff Schema", "required_fields")),
                    provided_contract_refs: split_values(values(
                        "Handoff Schema",
                        "provided_contract_refs",
                    )),
                    reviewer_check_refs: split_values(values(
                        "Handoff Schema",
                        "reviewer_check_refs",
                    )),
                },
                blocker_rules,
                design_traceability,
            }
        }
        _ => return None,
    };

    let trusted_commands = trusted_commands_for_checks(&verification_checks, catalog, diagnostics);
    Some(PlanCandidateItemIr {
        target_repository_id,
        verification_plan: WorkItemDraftVerificationPlan {
            checks: contract.verification_checks.clone(),
        },
        contract,
        trusted_commands,
    })
}

fn lower_tasks(
    fields: &[(&str, &WorkItemPlanFieldAst)],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<WorkItemTask> {
    let mut entries = Vec::new();
    let mut current: Option<TaskState> = None;
    for field in section_fields(fields, "Tasks") {
        match field.key.value.as_str() {
            "task_id" => {
                flush_task(&mut current, &mut entries, diagnostics);
                current = Some((
                    field.value.value.clone(),
                    String::new(),
                    Vec::new(),
                    Vec::new(),
                    field.value.line,
                ));
            }
            "statement" => {
                if let Some(entry) = current.as_mut() {
                    entry.1 = field.value.value.clone();
                }
            }
            "requirement_refs" => {
                if let Some(entry) = current.as_mut() {
                    entry.2 = split_value(&field.value.value);
                }
            }
            "done_when_refs" => {
                if let Some(entry) = current.as_mut() {
                    entry.3 = split_value(&field.value.value);
                }
            }
            _ => {}
        }
    }
    flush_task(&mut current, &mut entries, diagnostics);
    entries
}

fn flush_task(
    current: &mut Option<TaskState>,
    entries: &mut Vec<WorkItemTask>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    let Some((task_id, statement, requirement_refs, done_when_refs, line)) = current.take() else {
        return;
    };
    if task_id.is_empty() || statement.is_empty() {
        diagnostics.push(diagnostic(
            "Tasks",
            "任务条目必须包含 task_id 与 statement。",
            line,
            "- task_id: TASK-001",
        ));
    } else {
        entries.push(WorkItemTask {
            task_id,
            statement,
            requirement_refs,
            done_when_refs,
        });
    }
}

fn lower_acceptance_criteria(
    fields: &[(&str, &WorkItemPlanFieldAst)],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<AcceptanceCriterion> {
    let mut entries = Vec::new();
    let mut current: Option<(String, String, Vec<EvidenceKind>, usize)> = None;
    for field in section_fields(fields, "Acceptance Criteria") {
        match field.key.value.as_str() {
            "criterion_id" => {
                flush_acceptance(&mut current, &mut entries, diagnostics);
                current = Some((
                    field.value.value.clone(),
                    String::new(),
                    Vec::new(),
                    field.value.line,
                ));
            }
            "statement" => {
                if let Some(entry) = current.as_mut() {
                    entry.1 = field.value.value.clone();
                }
            }
            "required_evidence" => {
                if let Some(entry) = current.as_mut() {
                    entry
                        .2
                        .extend(field.value.value.split(',').filter_map(|value| {
                            parse_evidence_kind(value.trim(), diagnostics, field.value.line)
                        }));
                }
            }
            _ => {}
        }
    }
    flush_acceptance(&mut current, &mut entries, diagnostics);
    entries
}

fn flush_acceptance(
    current: &mut Option<(String, String, Vec<EvidenceKind>, usize)>,
    entries: &mut Vec<AcceptanceCriterion>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    let Some((criterion_id, statement, required_evidence, line)) = current.take() else {
        return;
    };
    if criterion_id.is_empty() || statement.is_empty() {
        diagnostics.push(diagnostic(
            "Acceptance Criteria",
            "验收条目必须包含 criterion_id 与 statement。",
            line,
            "- criterion_id: AC-001",
        ));
    } else {
        entries.push(AcceptanceCriterion {
            criterion_id,
            statement,
            required_evidence,
        });
    }
}

fn lower_verification_checks(
    fields: &[(&str, &WorkItemPlanFieldAst)],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<LoweredVerificationCheck> {
    let mut entries = Vec::new();
    let mut current: Option<VerificationState> = None;
    for field in section_fields(fields, "Verification") {
        match field.key.value.as_str() {
            "check_id" => {
                flush_verification(&mut current, &mut entries, diagnostics);
                current = Some((
                    field.value.value.clone(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    field.value.line,
                ));
            }
            "command" => {
                if let Some(entry) = current.as_mut() {
                    entry.1 = nonempty(field.value.value.as_str());
                    entry.2 = Some(field.value.line);
                }
            }
            "manual_instruction" => {
                if let Some(entry) = current.as_mut() {
                    entry.3 = nonempty(field.value.value.as_str());
                }
            }
            "required" => {
                if let Some(entry) = current.as_mut() {
                    entry.4 = parse_bool_value(field, diagnostics);
                }
            }
            "non_zero_test_execution_required" => {
                if let Some(entry) = current.as_mut() {
                    entry.5 = parse_bool_value(field, diagnostics);
                }
            }
            _ => {}
        }
    }
    flush_verification(&mut current, &mut entries, diagnostics);
    entries
}

fn flush_verification(
    current: &mut Option<VerificationState>,
    entries: &mut Vec<LoweredVerificationCheck>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    let Some((check_id, command, command_line, manual_instruction, required, non_zero, line)) =
        current.take()
    else {
        return;
    };
    let Some(required) = required else {
        diagnostics.push(diagnostic(
            "Verification.required",
            "Verification check 缺少 required。",
            line,
            "- required: true",
        ));
        return;
    };
    let Some(non_zero_test_execution_required) = non_zero else {
        diagnostics.push(diagnostic(
            "Verification.non_zero_test_execution_required",
            "Verification check 缺少 non_zero_test_execution_required。",
            line,
            "- non_zero_test_execution_required: true",
        ));
        return;
    };
    if check_id.is_empty() || (command.is_none() && manual_instruction.is_none()) {
        diagnostics.push(diagnostic(
            "Verification",
            "Verification check 必须包含 check_id，并至少包含 command 或 manual_instruction。",
            line,
            "- check_id: CHECK-001",
        ));
        return;
    }
    entries.push(LoweredVerificationCheck {
        command_line: command_line.unwrap_or(line),
        check: VerificationCheck {
            check_id,
            command,
            manual_instruction,
            required,
            non_zero_test_execution_required,
        },
    });
}

fn parse_bool_value(
    field: &WorkItemPlanFieldAst,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Option<bool> {
    match field.value.value.parse() {
        Ok(value) => Some(value),
        Err(_) => {
            diagnostics.push(diagnostic(
                field.key.value.as_str(),
                "字段必须是 true 或 false。",
                field.value.line,
                &format!("- {}: true", field.key.value),
            ));
            None
        }
    }
}

fn lower_inputs(
    fields: &[(&str, &WorkItemPlanFieldAst)],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<RequiredInputContract> {
    let mut entries = Vec::new();
    let mut current: Option<InputState> = None;
    for field in section_fields(fields, "Inputs") {
        match field.key.value.as_str() {
            "contract_id" => {
                flush_input(&mut current, &mut entries, diagnostics);
                current = Some((
                    field.value.value.clone(),
                    None,
                    Vec::new(),
                    None,
                    field.value.line,
                ));
            }
            "provider_logical_work_item_id" => {
                if let Some(entry) = current.as_mut() {
                    entry.1 = Some(field.value.value.clone());
                }
            }
            "required_capabilities" => {
                if let Some(entry) = current.as_mut() {
                    entry.2 = split_value(&field.value.value);
                }
            }
            "compatibility_policy" => {
                if let Some(entry) = current.as_mut() {
                    entry.3 = parse_compatibility_policy(
                        &field.value.value,
                        diagnostics,
                        field.value.line,
                    );
                }
            }
            _ => {}
        }
    }
    flush_input(&mut current, &mut entries, diagnostics);
    entries
}

fn flush_input(
    current: &mut Option<InputState>,
    entries: &mut Vec<RequiredInputContract>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    let Some((contract_id, provider, required_capabilities, compatibility_policy, line)) =
        current.take()
    else {
        return;
    };
    let Some(provider_logical_work_item_id) = provider else {
        diagnostics.push(diagnostic(
            "Inputs.provider_logical_work_item_id",
            "输入契约缺少 provider_logical_work_item_id。",
            line,
            "- provider_logical_work_item_id: WI-001",
        ));
        return;
    };
    let Some(compatibility_policy) = compatibility_policy else {
        diagnostics.push(diagnostic(
            "Inputs.compatibility_policy",
            "输入契约缺少 compatibility_policy。",
            line,
            "- compatibility_policy: require_all",
        ));
        return;
    };
    entries.push(RequiredInputContract {
        contract_id,
        provider_logical_work_item_id,
        required_capabilities,
        compatibility_policy,
    });
}

fn lower_outputs(
    fields: &[(&str, &WorkItemPlanFieldAst)],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<PromisedOutputContract> {
    let mut entries = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for field in section_fields(fields, "Outputs") {
        if field.key.value == "contract_id" {
            if let Some((contract_id, capabilities)) = current.take() {
                entries.push(PromisedOutputContract {
                    contract_id,
                    capabilities,
                });
            }
            current = Some((field.value.value.clone(), Vec::new()));
        } else if field.key.value == "capabilities"
            && let Some((_, capabilities)) = current.as_mut()
        {
            *capabilities = split_value(&field.value.value);
        }
    }
    if let Some((contract_id, capabilities)) = current {
        entries.push(PromisedOutputContract {
            contract_id,
            capabilities,
        });
    }
    let _ = diagnostics;
    entries
}

fn lower_blocker_rules(
    fields: &[(&str, &WorkItemPlanFieldAst)],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<BlockerRule> {
    let mut entries = Vec::new();
    let mut current: Option<(String, Option<BlockerRoute>, Vec<String>, usize)> = None;
    for field in section_fields(fields, "Blockers") {
        match field.key.value.as_str() {
            "reason_code" => {
                flush_blocker(&mut current, &mut entries, diagnostics);
                current = Some((
                    field.value.value.clone(),
                    None,
                    Vec::new(),
                    field.value.line,
                ));
            }
            "route" => {
                if let Some(entry) = current.as_mut() {
                    entry.1 =
                        parse_blocker_route(&field.value.value, diagnostics, field.value.line);
                }
            }
            "target_contract_refs" => {
                if let Some(entry) = current.as_mut() {
                    entry.2 = split_value(&field.value.value);
                }
            }
            _ => {}
        }
    }
    flush_blocker(&mut current, &mut entries, diagnostics);
    entries
}

fn flush_blocker(
    current: &mut Option<(String, Option<BlockerRoute>, Vec<String>, usize)>,
    entries: &mut Vec<BlockerRule>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    let Some((reason_code, route, target_contract_refs, line)) = current.take() else {
        return;
    };
    let Some(route) = route else {
        diagnostics.push(diagnostic(
            "Blockers.route",
            "blocker rule 缺少 route。",
            line,
            "- route: operational_gate",
        ));
        return;
    };
    entries.push(BlockerRule {
        reason_code,
        route,
        target_contract_refs,
    });
}

fn lower_traceability(
    fields: &[(&str, &WorkItemPlanFieldAst)],
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<DesignTraceabilityRef> {
    let mut entries = Vec::new();
    let mut current: Option<(String, Option<String>, Option<String>, usize)> = None;
    for field in section_fields(fields, "Traceability") {
        match field.key.value.as_str() {
            "source_type" => {
                flush_traceability(&mut current, &mut entries, diagnostics);
                current = Some((field.value.value.clone(), None, None, field.value.line));
            }
            "source_id" => {
                if let Some(entry) = current.as_mut() {
                    entry.1 = Some(field.value.value.clone());
                }
            }
            "requirement_id" => {
                if let Some(entry) = current.as_mut() {
                    entry.2 = Some(field.value.value.clone());
                }
            }
            _ => {}
        }
    }
    flush_traceability(&mut current, &mut entries, diagnostics);
    entries
}

fn flush_traceability(
    current: &mut Option<(String, Option<String>, Option<String>, usize)>,
    entries: &mut Vec<DesignTraceabilityRef>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    let Some((source_type, source_id, requirement_id, line)) = current.take() else {
        return;
    };
    let Some(source_id) = source_id else {
        diagnostics.push(diagnostic(
            "Traceability.source_id",
            "traceability 缺少 source_id。",
            line,
            "- source_id: design_spec_0001",
        ));
        return;
    };
    let Some(requirement_id) = requirement_id else {
        diagnostics.push(diagnostic(
            "Traceability.requirement_id",
            "traceability 缺少 requirement_id。",
            line,
            "- requirement_id: REQ-001",
        ));
        return;
    };
    entries.push(DesignTraceabilityRef {
        source_type,
        source_id,
        requirement_id,
    });
}

fn trusted_commands_for_checks(
    checks: &[LoweredVerificationCheck],
    catalog: &HashMap<&str, &TrustedDraftVerificationCommand>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) -> Vec<TrustedDraftVerificationCommand> {
    let mut trusted_commands = Vec::new();
    let mut refs = HashSet::new();
    for entry in checks {
        let Some(reference) = entry.check.command.as_deref() else {
            continue;
        };
        if !refs.insert(reference) {
            diagnostics.push(diagnostic(
                "trusted_commands",
                "同一 Work Item 不得重复引用 trusted command。",
                entry.command_line,
                "- command: catalog-entry-001",
            ));
            continue;
        }
        match catalog.get(reference) {
            Some(command) => trusted_commands.push((*command).clone()),
            None => diagnostics.push(diagnostic(
                "trusted_commands",
                "Verification command 未在 trusted command catalog 中找到。",
                entry.command_line,
                "- command: catalog-entry-001",
            )),
        }
    }
    trusted_commands
}

fn section_fields<'a>(
    fields: &'a [(&str, &'a WorkItemPlanFieldAst)],
    section: &str,
) -> Vec<&'a WorkItemPlanFieldAst> {
    fields
        .iter()
        .filter_map(|(name, field)| (*name == section).then_some(*field))
        .collect()
}
fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}
fn split_values(values: Vec<&str>) -> Vec<String> {
    values.into_iter().flat_map(split_value).collect()
}
fn split_value(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "[]")
        .map(str::to_string)
        .collect()
}
fn parse_compatibility_policy(
    value: &str,
    diagnostics: &mut Vec<CompilerDiagnostic>,
    line: usize,
) -> Option<ContractCompatibilityPolicy> {
    match value {
        "require_all" => Some(ContractCompatibilityPolicy::RequireAll),
        "require_any" => Some(ContractCompatibilityPolicy::RequireAny),
        _ => {
            diagnostics.push(diagnostic(
                "compatibility_policy",
                "字段值不符合 canonical enum。",
                line,
                "- compatibility_policy: require_all",
            ));
            None
        }
    }
}

fn parse_blocker_route(
    value: &str,
    diagnostics: &mut Vec<CompilerDiagnostic>,
    line: usize,
) -> Option<BlockerRoute> {
    let route = match value {
        "coder_rework" => BlockerRoute::CoderRework,
        "verification_retry" => BlockerRoute::VerificationRetry,
        "plan_repair_current" => BlockerRoute::PlanRepairCurrent,
        "plan_repair_upstream" => BlockerRoute::PlanRepairUpstream,
        "subgraph_replan" => BlockerRoute::SubgraphReplan,
        "story_amendment" => BlockerRoute::StoryAmendment,
        "design_amendment" => BlockerRoute::DesignAmendment,
        "operational_gate" => BlockerRoute::OperationalGate,
        _ => {
            diagnostics.push(diagnostic(
                "route",
                "字段值不符合 canonical enum。",
                line,
                "- route: operational_gate",
            ));
            return None;
        }
    };
    Some(route)
}

fn parse_evidence_kind(
    value: &str,
    diagnostics: &mut Vec<CompilerDiagnostic>,
    line: usize,
) -> Option<EvidenceKind> {
    let evidence = match value {
        "source_diff" => EvidenceKind::SourceDiff,
        "non_zero_test_execution" => EvidenceKind::NonZeroTestExecution,
        "manual_check" => EvidenceKind::ManualCheck,
        "handoff_field" => EvidenceKind::HandoffField,
        _ => {
            diagnostics.push(diagnostic(
                "required_evidence",
                "字段值不符合 canonical enum。",
                line,
                "- required_evidence: source_diff",
            ));
            return None;
        }
    };
    Some(evidence)
}
fn diagnostic(field: &str, message: &str, line: usize, repair_example: &str) -> CompilerDiagnostic {
    CompilerDiagnostic {
        code: "lowering_error".to_string(),
        line,
        field: field.to_string(),
        message: message.to_string(),
        repair_example: repair_example.to_string(),
    }
}
