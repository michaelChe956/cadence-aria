//! P4-c（REQ-CRO-12 / 契约 A1）：durable 事实的「待处理事项」只读投影。
//!
//! 不建通知表、无 ack——事项直接从 attempt 持久事实派生：open choice gate /
//! open blocked gate（人工分诊·挂起·其他阻塞）/ open stage gate / AMR 等待 /
//! FinalConfirm 等待 / plan 级聚合确认（调块 1 纯 `build_plan_final_confirm`）。
//! 事实关闭（作答/resolve/确认/恢复）后投影自然消失。稳定身份 =
//! `project/issue/key`，key 取 gate/choice/node/run/plan-run；`occurred_at`
//! 一律取事实时间（gate/choice/role run 的 created_at、timeline 节点
//! started_at），不用 GET now。投影核心只读 store，不反调 GET/lifecycle
//!（无递归）；不复制旧 FinalConfirm 投影的 Enrolled-only 限制——单仓/手动
//! attempt 同样投影。
//!
//! kind 语义（契约 A1）：`choice|triage|blocked|stalled|manual_recovery|
//! final_confirm|plan_final_confirm`。多 target（≥2 冻结 target）plan 仅在
//! A2 ready/partial 时聚一条 `plan_final_confirm`（target=coding_plan）并抑制
//! 该 plan 逐 attempt 的 `final_confirm` 催点；单仓/非聚合待确认仍逐
//! attempt 投影。

use std::collections::BTreeMap;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingChoiceGate, CodingExecutionStage, CodingGateRequired,
    CodingStageGateState, CodingStageGateStatus, CodingTimelineNodeStatus,
};
use crate::product::json_store::ProductStoreError;
use crate::web::coding_final_confirm_info::CODING_FINAL_CONFIRM_WAITING_TITLE;
use crate::web::handlers::build_plan_final_confirm;
use crate::web::types::PlanCodingFinalConfirmDto;

/// 待确认 choice 的固定标题（`kind=choice`）。
pub const CODING_ATTENTION_CHOICE_TITLE: &str = "AI 有问题需要你选择";
/// 人工分诊门（`kind=triage`）固定标题。
pub const CODING_ATTENTION_TRIAGE_TITLE: &str = "Coder 输出需要人工分诊";
/// 挂起门（`kind=stalled`）固定标题。
pub const CODING_ATTENTION_STALLED_TITLE: &str = "AI 运行挂起，需要处理";
/// 其他 blocked/stage 门（`kind=blocked`）固定标题。
pub const CODING_ATTENTION_BLOCKED_TITLE: &str = "执行被阻塞，需要处理";
/// AMR（`kind=manual_recovery`）固定标题。
pub const CODING_ATTENTION_MANUAL_RECOVERY_TITLE: &str = "AI 中断，等待人工恢复";
/// plan 级聚合确认（`kind=plan_final_confirm`）标题（ready 态）。
pub const CODING_ATTENTION_PLAN_FINAL_READY_TITLE: &str = "全部执行完成，待一键最终确认";
/// plan 级聚合确认（`kind=plan_final_confirm`）标题（partial 态）。
pub const CODING_ATTENTION_PLAN_FINAL_PARTIAL_TITLE: &str = "部分仓库已确认，剩余待一键确认";

const ATTENTION_SUMMARY_TEXT_LIMIT: usize = 200;

/// 契约 A1：事项定位信息（只定位不写操作）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttentionFocusDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
}

/// 契约 A1：事项目标——`coding_attempt|coding_plan|workspace_session`
/// （project/issue 取条目顶层，地址域不重复）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttentionTargetDto {
    #[serde(rename = "type")]
    pub target_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl CodingAttentionTargetDto {
    pub fn coding_attempt(attempt_id: &str) -> Self {
        Self {
            target_type: "coding_attempt".to_string(),
            attempt_id: Some(attempt_id.to_string()),
            plan_id: None,
            session_id: None,
        }
    }

    pub fn coding_plan(plan_id: &str) -> Self {
        Self {
            target_type: "coding_plan".to_string(),
            attempt_id: None,
            plan_id: Some(plan_id.to_string()),
            session_id: None,
        }
    }
}

/// 契约 A1：coding 待处理事项 DTO（`IssueLifecycleResponse
/// .coding_attention_items` 条目与 plan progress `attention_items` 同源）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttentionItemDto {
    /// 稳定身份第三段（前两段为 project/issue）。
    pub key: String,
    /// `choice|triage|blocked|stalled|manual_recovery|final_confirm|plan_final_confirm`。
    pub kind: String,
    pub project_id: String,
    pub issue_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_work_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub title: String,
    pub summary: String,
    /// RFC3339 事实时间（gate/choice created_at、node started_at 等）。
    pub occurred_at: String,
    pub target: CodingAttentionTargetDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<CodingAttentionFocusDto>,
}

/// open blocked 门按 reason_code 的 kind 分类（块 4 §4.2 表）：
/// `coding_output_human_triage` → triage；挂起门（P4-A5 reason 字符串，
/// 常量随后续包合入）→ stalled；其余（含环境/CR/推送门）→ blocked。
fn blocked_gate_attention_kind(reason_code: Option<&str>) -> &'static str {
    match reason_code {
        Some("coding_output_human_triage") => "triage",
        Some("coder_provider_stalled") | Some("code_review_provider_stalled") => "stalled",
        _ => "blocked",
    }
}

fn truncate_attention_text(text: &str) -> String {
    text.chars()
        .take(ATTENTION_SUMMARY_TEXT_LIMIT)
        .collect::<String>()
}

fn attempt_item(
    kind: &str,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    key: String,
    reason_code: Option<String>,
    title: &str,
    summary: String,
    occurred_at: String,
    focus: Option<CodingAttentionFocusDto>,
) -> CodingAttentionItemDto {
    CodingAttentionItemDto {
        key,
        kind: kind.to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        plan_id: attempt.work_item_group_id.clone(),
        attempt_id: Some(attempt.id.clone()),
        logical_work_item_id: None,
        repository_name: None,
        reason_code,
        title: title.to_string(),
        summary: truncate_attention_text(&summary),
        occurred_at,
        target: CodingAttentionTargetDto::coding_attempt(&attempt.id),
        focus,
    }
}

fn choice_item(
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    gate: &CodingChoiceGate,
) -> CodingAttentionItemDto {
    attempt_item(
        "choice",
        attempt,
        format!("choice:{}:{}", attempt.id, gate.choice_id),
        None,
        CODING_ATTENTION_CHOICE_TITLE,
        gate.prompt.clone(),
        gate.created_at.clone(),
        Some(CodingAttentionFocusDto {
            gate_id: None,
            choice_id: Some(gate.choice_id.clone()),
        }),
    )
}

fn blocked_gate_item(
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    gate: &CodingGateRequired,
    created_at: &str,
) -> CodingAttentionItemDto {
    let kind = blocked_gate_attention_kind(gate.reason_code.as_deref());
    let title = match kind {
        "triage" => CODING_ATTENTION_TRIAGE_TITLE,
        "stalled" => CODING_ATTENTION_STALLED_TITLE,
        _ => CODING_ATTENTION_BLOCKED_TITLE,
    };
    attempt_item(
        kind,
        attempt,
        format!("gate:{}:{}", attempt.id, gate.gate_id),
        gate.reason_code.clone(),
        title,
        if gate.title.is_empty() {
            gate.description.clone()
        } else {
            gate.title.clone()
        },
        created_at.to_string(),
        Some(CodingAttentionFocusDto {
            gate_id: Some(gate.gate_id.clone()),
            choice_id: None,
        }),
    )
}

fn stage_gate_item(
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    gate: &CodingStageGateState,
) -> CodingAttentionItemDto {
    let stage_text =
        crate::web::handlers::dto::coding_execution_stage_text(&gate.stage).to_string();
    attempt_item(
        "blocked",
        attempt,
        format!("stage:{}:{}", attempt.id, gate.gate_id),
        None,
        CODING_ATTENTION_BLOCKED_TITLE,
        format!("阶段 {stage_text} 的确认门等待处理"),
        gate.created_at.clone(),
        Some(CodingAttentionFocusDto {
            gate_id: Some(gate.gate_id.clone()),
            choice_id: None,
        }),
    )
}

/// AMR：取该 attempt 最新 role run（run_no 最大）作 key 与事实时间；无
/// role run 的孤立 AMR fail-closed 不投影（不伪造事实）。
fn manual_recovery_item(
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    store: &CodingAttemptStore,
) -> Result<Option<CodingAttentionItemDto>, ProductStoreError> {
    let mut runs = store.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    runs.sort_by(|left, right| {
        left.run_no
            .cmp(&right.run_no)
            .then_with(|| left.id.cmp(&right.id))
    });
    let Some(latest) = runs.into_iter().next_back() else {
        return Ok(None);
    };
    let occurred_at = latest
        .completed_at
        .clone()
        .unwrap_or_else(|| latest.started_at.clone());
    Ok(Some(attempt_item(
        "manual_recovery",
        attempt,
        format!("recovery:{}:{}", attempt.id, latest.id),
        attempt.manual_recovery_reason.clone(),
        CODING_ATTENTION_MANUAL_RECOVERY_TITLE,
        attempt
            .manual_recovery_reason
            .clone()
            .unwrap_or_else(|| "attempt 进入人工恢复等待".to_string()),
        occurred_at,
        None,
    )))
}

/// 枚举 issue 下全部 coding 待处理事项（只读持久事实；读取错误传播，
/// 单条事实的判据不满足按 fail-closed 跳过）。
pub fn issue_coding_attention_items(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Vec<CodingAttentionItemDto>, ProductStoreError> {
    let store = CodingAttemptStore::new(paths.clone());
    let attempts = store.list_attempts_for_issue(project_id, issue_id)?;

    // 聚合 plan 判定（A2）：≥2 冻结 target 才算聚合；ready/partial 才投
    // plan_final_confirm。plan 绑定不可解析（无 lineage/旧数据）按非聚合
    // 降级——逐 attempt final_confirm 仍投影，不让单个坏 plan 炸掉整issue
    // 目录；其余读取错误 fail-closed 上抛。
    let mut plan_aggregates: BTreeMap<String, Option<PlanCodingFinalConfirmDto>> = BTreeMap::new();
    let mut plan_final_node_times: BTreeMap<String, String> = BTreeMap::new();
    for attempt in &attempts {
        let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
            continue;
        };
        if !plan_aggregates.contains_key(plan_id) {
            plan_aggregates.insert(
                plan_id.to_string(),
                build_plan_final_confirm(paths, project_id, issue_id, plan_id).ok(),
            );
        }
        // 聚合事项的事实时间：成员 FinalConfirm 节点 started_at 的最大值
        //（最后一个仓进入等待的时刻，即整体可确认的时刻）。
        for node in store.get_timeline_nodes(project_id, issue_id, &attempt.id)? {
            if node.stage == CodingExecutionStage::FinalConfirm {
                let current = plan_final_node_times
                    .get(plan_id)
                    .cloned()
                    .unwrap_or_default();
                if node.started_at > current {
                    plan_final_node_times.insert(plan_id.to_string(), node.started_at.clone());
                }
            }
        }
    }
    let plan_target_count = |dto: &PlanCodingFinalConfirmDto| {
        dto.ready_attempt_ids.len() + dto.confirmed_attempt_ids.len() + dto.blocking.len()
    };

    let mut items = Vec::new();
    for attempt in &attempts {
        let has_open_gate_facts = {
            let choices = store.list_open_choice_gates(project_id, issue_id, &attempt.id)?;
            let blocked = store.list_open_blocked_gate_facts(project_id, issue_id, &attempt.id)?;
            let stages = store
                .list_open_stage_gates(project_id, issue_id, &attempt.id)?
                .into_iter()
                .filter(|gate| gate.status == CodingStageGateStatus::Open)
                .collect::<Vec<_>>();

            for gate in &choices {
                items.push(choice_item(attempt, gate));
            }
            for (gate, created_at) in &blocked {
                items.push(blocked_gate_item(attempt, gate, created_at));
            }
            for gate in &stages {
                items.push(stage_gate_item(attempt, gate));
            }
            !choices.is_empty() || !blocked.is_empty() || !stages.is_empty()
        };

        // AMR 且无更具体的 open 门事实时才投 manual_recovery（有门时门
        // 事项已覆盖提醒，不重复）。
        if attempt.status == CodingAttemptStatus::AwaitingManualRecovery && !has_open_gate_facts {
            if let Some(item) = manual_recovery_item(attempt, &store)? {
                items.push(item);
            }
        }

        // FinalConfirm 等待：WaitingForHuman ∧ 同 attempt FinalConfirm 节点
        // Pending（fail-closed：中间态不投影，等 durable 事实一致）。
        if attempt.status == CodingAttemptStatus::WaitingForHuman {
            let nodes = store.get_timeline_nodes(project_id, issue_id, &attempt.id)?;
            let waiting_node = nodes.iter().find(|node| {
                node.stage == CodingExecutionStage::FinalConfirm
                    && node.status == CodingTimelineNodeStatus::Pending
            });
            if let Some(node) = waiting_node {
                // 多 target 聚合 plan：抑制逐仓催点（A2 聚一条）。
                let aggregate_suppressed = attempt
                    .work_item_group_id
                    .as_deref()
                    .and_then(|plan_id| plan_aggregates.get(plan_id))
                    .and_then(|dto| dto.as_ref())
                    .is_some_and(|dto| plan_target_count(dto) >= 2);
                if !aggregate_suppressed {
                    items.push(attempt_item(
                        "final_confirm",
                        attempt,
                        format!("final:{}:{}", attempt.id, node.id),
                        None,
                        CODING_FINAL_CONFIRM_WAITING_TITLE,
                        "编码执行完成，等待人工最终确认".to_string(),
                        node.started_at.clone(),
                        None,
                    ));
                }
            }
        }
    }

    // plan 级聚合：ready/partial 各一条（多 target plan）。
    for (plan_id, aggregate) in &plan_aggregates {
        let Some(dto) = aggregate else {
            continue;
        };
        if plan_target_count(dto) < 2 {
            continue;
        }
        let (title, summary) = match dto.state.as_str() {
            "ready" => (
                CODING_ATTENTION_PLAN_FINAL_READY_TITLE,
                format!(
                    "{} 个仓库均已执行完成，可一次确认全部",
                    dto.ready_attempt_ids.len()
                ),
            ),
            "partial" => (
                CODING_ATTENTION_PLAN_FINAL_PARTIAL_TITLE,
                format!(
                    "已确认 {} 个仓库，剩余 {} 个可继续一键确认",
                    dto.confirmed_attempt_ids.len(),
                    dto.ready_attempt_ids.len()
                ),
            ),
            _ => continue,
        };
        let run_discriminator =
            CodingPlanRunDiscriminator::from_paths(paths, project_id, issue_id, plan_id);
        items.push(CodingAttentionItemDto {
            key: format!("plan-final:{}:{}", plan_id, run_discriminator.0),
            kind: "plan_final_confirm".to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: Some(plan_id.clone()),
            attempt_id: None,
            logical_work_item_id: None,
            repository_name: None,
            reason_code: None,
            title: title.to_string(),
            summary: truncate_attention_text(&summary),
            occurred_at: plan_final_node_times
                .get(plan_id)
                .cloned()
                .unwrap_or_default(),
            target: CodingAttentionTargetDto::coding_plan(plan_id),
            focus: None,
        });
    }

    items.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.key.cmp(&right.key))
    });
    Ok(items)
}

/// plan-final key 的 run 判别段：当前 plan-run 的 command_id（区分重启轮
/// 次）；无 run 记录时用 `-`。
struct CodingPlanRunDiscriminator(pub String);

impl CodingPlanRunDiscriminator {
    fn from_paths(
        paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Self {
        let command_id = crate::product::coding_plan_run::CodingPlanRunStore::new(paths.clone())
            .get(project_id, issue_id, plan_id)
            .ok()
            .flatten()
            .map(|run| run.command_id);
        Self(command_id.unwrap_or_else(|| "-".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::coding_attempt_store::{
        CodingAttemptStore, CreateBlockedGateInput, CreateChoiceGateInput,
        CreateCodingAttemptInput, CreateGroupCodingAttemptInput, ResolveChoiceGateInput,
    };
    use crate::product::coding_models::{
        CodingAttemptScope, CodingChoiceOption, CodingExecutionAttempt,
        CodingRoleProviderConfigSnapshot, CodingRoleRunStatus, CodingRoleRunTrigger,
        CodingStartRunPolicy, CodingTimelineNode,
    };
    use crate::product::models::{ProviderName, WorkspaceRolePermissionModes};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";

    struct AttentionFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        store: CodingAttemptStore,
    }

    fn attention_fixture() -> AttentionFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        AttentionFixture {
            _root: root,
            paths,
            store,
        }
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: WorkspaceRolePermissionModes::default(),
        }
    }

    fn group_attempt(
        store: &CodingAttemptStore,
        plan_id: &str,
        work_item_id: &str,
    ) -> CodingExecutionAttempt {
        store
            .create_group_attempt(CreateGroupCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                plan_id: plan_id.to_string(),
                current_work_item_id: work_item_id.to_string(),
                base_branch: "main".to_string(),
                branch_name: format!("aria/issues/{ISSUE_ID}"),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 2,
                start_run_policy: CodingStartRunPolicy::Manual,
            })
            .unwrap()
    }

    fn manual_attempt(store: &CodingAttemptStore, work_item_id: &str) -> CodingExecutionAttempt {
        store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: work_item_id.to_string(),
                base_branch: "main".to_string(),
                branch_name: format!("aria/work-items/{work_item_id}"),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .unwrap()
    }

    fn blocked_input(attempt_id: &str, reason_code: &str, title: &str) -> CreateBlockedGateInput {
        CreateBlockedGateInput {
            attempt_id: attempt_id.to_string(),
            stage: CodingExecutionStage::Coding,
            node_id: None,
            role: Some(crate::product::coding_models::CodingProviderRole::Coder),
            title: title.to_string(),
            description: format!("{title}（测试描述）"),
            reason_code: Some(reason_code.to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: Vec::new(),
        }
    }

    fn pending_final_confirm_node(attempt_id: &str) -> CodingTimelineNode {
        CodingTimelineNode {
            id: "timeline_node_final_0001".to_string(),
            attempt_id: attempt_id.to_string(),
            stage: CodingExecutionStage::FinalConfirm,
            title: "等待人工最终确认".to_string(),
            status: CodingTimelineNodeStatus::Pending,
            agent_role: None,
            summary: None,
            started_at: "2026-10-15T08:00:00Z".to_string(),
            completed_at: None,
            artifact_refs: Vec::new(),
        }
    }

    fn force_waiting_final_confirm(store: &CodingAttemptStore, attempt_id: &str) {
        let mut stored = store.get_attempt(PROJECT_ID, ISSUE_ID, attempt_id).unwrap();
        stored.status = CodingAttemptStatus::WaitingForHuman;
        stored.stage = CodingExecutionStage::FinalConfirm;
        store.write_coding_attempt_for_test(&stored).unwrap();
        store
            .save_timeline_node(&stored, pending_final_confirm_node(attempt_id))
            .unwrap();
    }

    /// ① 各类 durable 事实都有稳定 key；occurred_at 取事实时间非 GET now。
    #[test]
    fn attention_items_cover_durable_facts_with_stable_keys() {
        let fixture = attention_fixture();
        let store = &fixture.store;

        let gates = group_attempt(store, "work_item_plan_gates", "wi_gates");
        let choice = store
            .create_choice_gate(
                &gates,
                CreateChoiceGateInput {
                    attempt_id: gates.id.clone(),
                    choice_id: "choice_primary_db".to_string(),
                    stage: CodingExecutionStage::Coding,
                    node_id: None,
                    role: crate::product::coding_models::CodingProviderRole::Coder,
                    provider: ProviderName::Fake,
                    source: "test".to_string(),
                    prompt: "选择主库还是只读副本？".to_string(),
                    options: vec![CodingChoiceOption {
                        id: "primary".to_string(),
                        label: "主库".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: Vec::new(),
                },
            )
            .unwrap();
        let triage = store
            .create_blocked_gate(
                &gates,
                blocked_input(
                    &gates.id,
                    "coding_output_human_triage",
                    "Coder 输出需要人工分诊",
                ),
            )
            .unwrap();
        let stalled = store
            .create_blocked_gate(
                &gates,
                blocked_input(&gates.id, "coder_provider_stalled", "AI 运行挂起"),
            )
            .unwrap();
        let env = store
            .create_blocked_gate(
                &gates,
                blocked_input(&gates.id, "coding_operational_blocker", "运行环境阻塞"),
            )
            .unwrap();
        let stage_gate = store
            .create_stage_gate(
                &gates,
                CodingExecutionStage::CodeReview,
                crate::product::coding_models::CodingProviderRole::CodeReviewer,
                "2099-01-01T00:00:00Z".to_string(),
                CodingRoleProviderConfigSnapshot {
                    coder: ProviderName::Fake,
                    code_reviewer: Some(ProviderName::Fake),
                    internal_reviewer: None,
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
            )
            .unwrap();
        // 门事实已落盘：attempt 置终态，避开 store 的同桶单 active 创建约束
        //（门/选择/阶段门投影只依赖事实，不依赖 attempt 状态）。
        let mut gates_stored = store.get_attempt(PROJECT_ID, ISSUE_ID, &gates.id).unwrap();
        gates_stored.status = CodingAttemptStatus::Completed;
        store.write_coding_attempt_for_test(&gates_stored).unwrap();
        // AMR：先有 Failed role run（事实时间），attempt 再进人工恢复等待。
        let amr = group_attempt(store, "work_item_plan_amr", "wi_amr");
        let failed_run = store
            .create_role_run(
                &amr,
                CodingExecutionStage::Coding,
                crate::product::coding_models::CodingProviderRole::Coder,
                CodingRoleRunTrigger::Initial,
                None,
            )
            .unwrap();
        let failed_run = store
            .update_role_run_status(
                PROJECT_ID,
                ISSUE_ID,
                &amr.id,
                &failed_run.id,
                CodingRoleRunStatus::Failed,
                Some("provider_transport_exhausted".to_string()),
            )
            .unwrap();
        store
            .transition_to_awaiting_manual_recovery(&amr.id, "coding_runner_failed_while_running")
            .unwrap();

        // 单仓/手动 FinalConfirm 等待（无 Enrolled 前置）。
        let waiting = manual_attempt(store, "wi_manual");
        force_waiting_final_confirm(store, &waiting.id);

        let items = issue_coding_attention_items(&fixture.paths, PROJECT_ID, ISSUE_ID).unwrap();
        let find = |key: &str| {
            items
                .iter()
                .find(|item| item.key == key)
                .unwrap_or_else(|| panic!("attention item {key} missing: {items:?}"))
        };

        let choice_item = find(&format!("choice:{}:choice_primary_db", gates.id));
        assert_eq!(choice_item.kind, "choice");
        assert_eq!(
            choice_item
                .focus
                .as_ref()
                .and_then(|f| f.choice_id.as_deref()),
            Some("choice_primary_db")
        );
        assert_eq!(choice_item.occurred_at, choice.created_at);
        assert_eq!(choice_item.target.target_type, "coding_attempt");
        assert_eq!(
            choice_item.target.attempt_id.as_deref(),
            Some(gates.id.as_str())
        );

        let triage_item = find(&format!("gate:{}:{}", gates.id, triage.gate_id));
        assert_eq!(triage_item.kind, "triage");
        assert_eq!(
            triage_item
                .focus
                .as_ref()
                .and_then(|f| f.gate_id.as_deref()),
            Some(triage.gate_id.as_str())
        );
        assert_eq!(
            triage_item.reason_code.as_deref(),
            Some("coding_output_human_triage")
        );

        let stalled_item = find(&format!("gate:{}:{}", gates.id, stalled.gate_id));
        assert_eq!(stalled_item.kind, "stalled");

        let env_item = find(&format!("gate:{}:{}", gates.id, env.gate_id));
        assert_eq!(env_item.kind, "blocked");

        let stage_item = find(&format!("stage:{}:{}", gates.id, stage_gate.gate_id));
        assert_eq!(stage_item.kind, "blocked");
        assert_eq!(stage_item.occurred_at, stage_gate.created_at);

        let recovery_item = find(&format!("recovery:{}:{}", amr.id, failed_run.id));
        assert_eq!(recovery_item.kind, "manual_recovery");
        assert_eq!(
            recovery_item.occurred_at,
            failed_run.completed_at.clone().unwrap()
        );

        let final_item = find(&format!("final:{}:timeline_node_final_0001", waiting.id));
        assert_eq!(final_item.kind, "final_confirm");
        // R5 同款判据：occurred_at 取节点首次 started_at（事实时间）。
        assert_eq!(final_item.occurred_at, "2026-10-15T08:00:00Z");
        assert_eq!(final_item.plan_id, None);

        assert_eq!(items.len(), 7, "每条 durable 事实恰一条: {items:?}");
    }

    /// ② 事实关闭（作答/resolve/确认/恢复）后投影消失，无 ack。
    #[test]
    fn attention_items_disappear_when_facts_close() {
        let fixture = attention_fixture();
        let store = &fixture.store;
        let gates = group_attempt(store, "work_item_plan_gates", "wi_gates");
        store
            .create_choice_gate(
                &gates,
                CreateChoiceGateInput {
                    attempt_id: gates.id.clone(),
                    choice_id: "choice_primary_db".to_string(),
                    stage: CodingExecutionStage::Coding,
                    node_id: None,
                    role: crate::product::coding_models::CodingProviderRole::Coder,
                    provider: ProviderName::Fake,
                    source: "test".to_string(),
                    prompt: "选择主库还是只读副本？".to_string(),
                    options: Vec::new(),
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: Vec::new(),
                },
            )
            .unwrap();
        let triage = store
            .create_blocked_gate(
                &gates,
                blocked_input(&gates.id, "coding_output_human_triage", "需要分诊"),
            )
            .unwrap();
        // 门事实已落盘：attempt 置终态，避开同桶单 active 创建约束。
        let mut gates_stored = store.get_attempt(PROJECT_ID, ISSUE_ID, &gates.id).unwrap();
        gates_stored.status = CodingAttemptStatus::Completed;
        store.write_coding_attempt_for_test(&gates_stored).unwrap();

        // 先建 AMR（终态后再建 waiting，避免同桶单 active 约束）。
        let amr = group_attempt(store, "work_item_plan_amr", "wi_amr");
        store
            .create_role_run(
                &amr,
                CodingExecutionStage::Coding,
                crate::product::coding_models::CodingProviderRole::Coder,
                CodingRoleRunTrigger::Initial,
                None,
            )
            .unwrap();
        store
            .transition_to_awaiting_manual_recovery(&amr.id, "coding_runner_failed_while_running")
            .unwrap();

        let waiting = manual_attempt(store, "wi_manual");
        force_waiting_final_confirm(store, &waiting.id);

        let before = issue_coding_attention_items(&fixture.paths, PROJECT_ID, ISSUE_ID).unwrap();
        assert_eq!(before.len(), 4);

        // 作答 choice → 消失。
        store
            .resolve_choice_gate(
                PROJECT_ID,
                ISSUE_ID,
                &gates.id,
                "choice_primary_db",
                ResolveChoiceGateInput {
                    selected_option_ids: vec!["primary".to_string()],
                    free_text: None,
                    answers: Vec::new(),
                },
            )
            .unwrap();
        // resolve 分诊门 → 消失。
        store
            .resolve_blocked_gate(PROJECT_ID, ISSUE_ID, &gates.id, &triage.gate_id)
            .unwrap();
        // AMR 恢复（回到 Running）→ 消失。
        let mut recovered = store.get_attempt(PROJECT_ID, ISSUE_ID, &amr.id).unwrap();
        recovered.status = CodingAttemptStatus::Running;
        recovered.manual_recovery_reason = None;
        store.write_coding_attempt_for_test(&recovered).unwrap();
        // FinalConfirm 人工确认（attempt Completed + 节点 Completed）→ 消失。
        let mut confirmed = store
            .get_attempt(PROJECT_ID, ISSUE_ID, &waiting.id)
            .unwrap();
        confirmed.status = CodingAttemptStatus::Completed;
        store.write_coding_attempt_for_test(&confirmed).unwrap();
        store
            .update_timeline_node_status(
                PROJECT_ID,
                ISSUE_ID,
                &waiting.id,
                "timeline_node_final_0001",
                CodingTimelineNodeStatus::Completed,
                Some("用户已确认完成".to_string()),
                Some("2026-10-15T09:00:00Z".to_string()),
            )
            .unwrap();

        let after = issue_coding_attention_items(&fixture.paths, PROJECT_ID, ISSUE_ID).unwrap();
        assert!(after.is_empty(), "全部事实关闭后投影清空: {after:?}");
    }

    /// 投影核心只读 store（无 WebAppState/GET/lifecycle 依赖，结构性无递归），
    /// 且 plan 绑定不可解析（无 lineage）时降级非聚合，逐 attempt 投影照常。
    #[test]
    fn attention_projection_tolerates_unresolvable_plan_bindings() {
        let fixture = attention_fixture();
        let store = &fixture.store;
        let waiting = group_attempt(store, "work_item_plan_ghost", "wi_group");
        assert_eq!(
            store
                .get_attempt(PROJECT_ID, ISSUE_ID, &waiting.id)
                .unwrap()
                .scope,
            CodingAttemptScope::WorkItemGroup
        );
        force_waiting_final_confirm(store, &waiting.id);

        let items = issue_coding_attention_items(&fixture.paths, PROJECT_ID, ISSUE_ID).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "final_confirm");
        assert_eq!(items[0].plan_id.as_deref(), Some("work_item_plan_ghost"));
    }
}
