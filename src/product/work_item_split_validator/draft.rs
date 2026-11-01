use std::collections::BTreeSet;

use crate::product::work_item_contract::{
    BlockerRoute, ContractFindingSeverity, validate_canonical_contract,
};

use super::*;

pub(crate) fn validate_canonical_contract_candidate(
    current: &WorkItemDraftCandidate,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    for finding in validate_canonical_contract(&current.canonical_contract_candidate).findings {
        let mut affected_ids = vec![current.logical_work_item_id.clone()];
        affected_ids.extend(finding.contract_ref.iter().cloned());
        affected_ids.extend(finding.capability_ref.iter().cloned());
        affected_ids.sort();
        affected_ids.dedup();
        // F2b：canonical 既有 typed kind 原样传递（F2a 供需族 payload）；
        // None 按旧 JSON 投影规则折叠 Unknown，不反推、不丢 finding。
        let kind = finding
            .kind
            .clone()
            .unwrap_or_else(|| PlanningFailureKind::Unknown {
                code: finding.code.clone(),
            });
        findings.push(match finding.severity {
            ContractFindingSeverity::Error => {
                error(kind, &finding.code, finding.message, affected_ids)
            }
            ContractFindingSeverity::Warning => {
                warning(kind, &finding.code, finding.message, affected_ids)
            }
        });
    }
}

pub(crate) fn validate_draft_identity_and_find_outline<'a>(
    current: &WorkItemDraftCandidate,
    outline: &'a WorkItemPlanOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) -> Option<&'a WorkItemOutline> {
    validate_outline_identity_uniqueness(outline, findings);

    let canonical_identity = &current.canonical_contract_candidate.identity;
    if current.logical_work_item_id != canonical_identity.logical_work_item_id {
        // F2b：identity/归属一致性缺口缺封闭 L1 kind 事实（TargetMismatch 语义
        // 归 SG1 授权成员/Plan Intent 解析），折叠 Unknown。
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "draft_logical_identity_mismatch".to_string(),
            },
            "draft_logical_identity_mismatch",
            format!(
                "draft logical_work_item_id {} does not match canonical contract identity {}",
                current.logical_work_item_id, canonical_identity.logical_work_item_id
            ),
            vec![
                current.logical_work_item_id.clone(),
                canonical_identity.logical_work_item_id.clone(),
            ],
        ));
    }

    let mut matching_outlines = outline
        .work_item_outlines
        .iter()
        .filter(|item| item.outline_id == current.outline_id);
    let Some(current_outline) = matching_outlines.next() else {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "draft_outline_not_found".to_string(),
            },
            "draft_outline_not_found",
            format!(
                "draft outline_id {} does not exist in the confirmed outline",
                current.outline_id
            ),
            vec![current.outline_id.clone()],
        ));
        return None;
    };
    if matching_outlines.next().is_some() {
        return None;
    }

    if current.logical_work_item_id != current_outline.logical_work_item_id {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "draft_logical_identity_mismatch".to_string(),
            },
            "draft_logical_identity_mismatch",
            format!(
                "draft logical_work_item_id {} does not match outline {} logical identity {}",
                current.logical_work_item_id,
                current_outline.outline_id,
                current_outline.logical_work_item_id
            ),
            vec![
                current.outline_id.clone(),
                current.logical_work_item_id.clone(),
                current_outline.logical_work_item_id.clone(),
            ],
        ));
    }

    if current.target_repository_id != current_outline.target_repository_id {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "draft_target_repository_mismatch".to_string(),
            },
            "draft_target_repository_mismatch",
            format!(
                "draft target_repository_id {:?} does not match outline {} target {:?}",
                current.target_repository_id,
                current_outline.outline_id,
                current_outline.target_repository_id
            ),
            vec![
                current.outline_id.clone(),
                current.logical_work_item_id.clone(),
            ],
        ));
    }

    if canonical_identity.title != current_outline.title
        || canonical_identity.kind != current_outline.kind.as_str()
    {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "draft_outline_identity_mismatch".to_string(),
            },
            "draft_outline_identity_mismatch",
            format!(
                "canonical identity ({}, {}) does not match outline {} identity ({}, {})",
                canonical_identity.title,
                canonical_identity.kind,
                current_outline.outline_id,
                current_outline.title,
                current_outline.kind.as_str()
            ),
            vec![
                current.outline_id.clone(),
                current.logical_work_item_id.clone(),
            ],
        ));
    }

    Some(current_outline)
}

fn validate_outline_identity_uniqueness(
    outline: &WorkItemPlanOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let mut outline_ids = BTreeSet::new();
    let mut duplicate_outline_ids = BTreeSet::new();
    let mut logical_ids = BTreeSet::new();
    let mut duplicate_logical_ids = BTreeSet::new();
    for item in &outline.work_item_outlines {
        if !outline_ids.insert(item.outline_id.as_str()) {
            duplicate_outline_ids.insert(item.outline_id.as_str());
        }
        if !logical_ids.insert(item.logical_work_item_id.as_str()) {
            duplicate_logical_ids.insert(item.logical_work_item_id.as_str());
        }
    }
    for outline_id in duplicate_outline_ids {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "duplicate_outline_id".to_string(),
            },
            "duplicate_outline_id",
            format!("outline id {outline_id} is duplicated"),
            vec![outline_id.to_string()],
        ));
    }
    for logical_work_item_id in duplicate_logical_ids {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "duplicate_logical_work_item_identity".to_string(),
            },
            "duplicate_logical_work_item_identity",
            format!("logical work item identity {logical_work_item_id} is duplicated"),
            vec![logical_work_item_id.to_string()],
        ));
    }
}

pub(crate) fn validate_draft_provider_logical_ids(
    current: &WorkItemDraftCandidate,
    outline: &WorkItemPlanOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let outline_logical_ids = outline
        .work_item_outlines
        .iter()
        .map(|item| item.logical_work_item_id.as_str())
        .collect::<BTreeSet<_>>();
    for input in &current.canonical_contract_candidate.input_contracts {
        if !outline_logical_ids.contains(input.provider_logical_work_item_id.as_str()) {
            // F2b（REQ-PLN-09）：outline 级供需缺口与 canonical 同构——
            // provider/consumer/contract 三元 ID 完整保留，唯一具 L1 资格的
            // split 预检 kind；真实 Inputs 引用不因检出被删除。
            findings.push(error(
                PlanningFailureKind::UnknownProvider {
                    provider: input.provider_logical_work_item_id.clone(),
                    consumer: current.logical_work_item_id.clone(),
                    contract_id: input.contract_id.clone(),
                },
                "unknown_provider_logical_work_item",
                format!(
                    "input contract {} references provider {} which is not present in the outline",
                    input.contract_id, input.provider_logical_work_item_id
                ),
                vec![
                    current.logical_work_item_id.clone(),
                    input.provider_logical_work_item_id.clone(),
                ],
            ));
        }
    }
}

pub(crate) fn validate_draft_scopes(
    current: &WorkItemDraftCandidate,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    if current
        .canonical_contract_candidate
        .write_policy
        .exclusive_scopes
        .is_empty()
    {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "write_scope_required".to_string(),
            },
            "write_scope_required",
            format!(
                "draft {} must include at least one canonical exclusive write scope",
                current.outline_id
            ),
            vec![current.outline_id.clone()],
        ));
    }
}

pub(crate) fn validate_draft_verification_plan(
    current: &WorkItemDraftCandidate,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    if current.verification_plan.checks != current.canonical_contract_candidate.verification_checks
    {
        findings.push(error(
            PlanningFailureKind::Unknown {
                code: "verification_plan_not_derived_from_contract".to_string(),
            },
            "verification_plan_not_derived_from_contract",
            format!(
                "draft {} verification plan must exactly preserve canonical verification checks",
                current.outline_id
            ),
            vec![
                current.outline_id.clone(),
                current.logical_work_item_id.clone(),
            ],
        ));
    }
}

pub(crate) fn validate_draft_trusted_verification_commands(
    current: &WorkItemDraftCandidate,
    current_outline: &WorkItemOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let trusted_commands = current_outline
        .trusted_verification_commands
        .iter()
        .map(|entry| entry.command.as_str())
        .collect::<BTreeSet<_>>();

    if trusted_commands.is_empty()
        && !current
            .canonical_contract_candidate
            .blocker_rules
            .iter()
            .any(|blocker| blocker.route == BlockerRoute::OperationalGate)
    {
        // F2b（REQ-SG-05）：可信目录是用户确认的授权事实——缺失走
        // security operational 等待面（用户确认目录或加 operational_gate
        // blocker），机器不得补权限、不得驱 L1 author。
        findings.push(error(
            PlanningFailureKind::Operational {
                stable_code: "missing_trusted_verification_command_catalog".to_string(),
                cause: "trusted verification command catalog is empty without an operational_gate blocker"
                    .to_string(),
            },
            "missing_trusted_verification_command_catalog",
            format!(
                "draft {} has no confirmed trusted verification command catalog; add an operational_gate blocker instead of inventing a required command",
                current.outline_id
            ),
            vec![
                current.outline_id.clone(),
                current.logical_work_item_id.clone(),
            ],
        ));
    }

    for check in current
        .canonical_contract_candidate
        .verification_checks
        .iter()
        .filter(|check| check.required)
    {
        let Some(command) = check.command.as_deref() else {
            continue;
        };
        if !trusted_commands.contains(command) {
            // F2b（REQ-SG-05）：required command 缺已确认授权——security
            // operational 等待面，机器不得把它补进可信目录。
            findings.push(error(
                PlanningFailureKind::Operational {
                    stable_code: "untrusted_required_verification_command".to_string(),
                    cause: "required verification command is outside the confirmed trusted catalog"
                        .to_string(),
                },
                "untrusted_required_verification_command",
                format!(
                    "draft {} required verification check {} command is not in the confirmed trusted catalog",
                    current.outline_id, check.check_id
                ),
                vec![
                    current.outline_id.clone(),
                    current.logical_work_item_id.clone(),
                    check.check_id.clone(),
                ],
            ));
        }
    }
}

pub(crate) fn validate_draft_direct_dependency_scopes(
    current: &WorkItemDraftCandidate,
    accepted_dependencies: &[WorkItemDraftCandidate],
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let direct_dependency_ids = current
        .canonical_contract_candidate
        .input_contracts
        .iter()
        .map(|input| input.provider_logical_work_item_id.as_str())
        .collect::<BTreeSet<_>>();
    for dependency in accepted_dependencies {
        if !direct_dependency_ids.contains(dependency.logical_work_item_id.as_str()) {
            continue;
        }
        if scopes_may_overlap(
            &current
                .canonical_contract_candidate
                .write_policy
                .exclusive_scopes,
            &dependency
                .canonical_contract_candidate
                .write_policy
                .exclusive_scopes,
            true,
        ) {
            findings.push(error(
                PlanningFailureKind::Unknown {
                    code: "direct_dependency_scope_conflict".to_string(),
                },
                "direct_dependency_scope_conflict",
                format!(
                    "draft {} overlaps exclusive write scopes with direct dependency {}",
                    current.outline_id, dependency.outline_id
                ),
                vec![current.outline_id.clone(), dependency.outline_id.clone()],
            ));
        }
    }
}
