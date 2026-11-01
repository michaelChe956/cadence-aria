use std::path::PathBuf;

/// N3（REQ-CS-01）：多仓 confirmed plan 的一键显式 Coding 启动请求——
/// `command_id` 为稳定幂等键，`expected_plan_revision_id` 与 GET 聚合返回的
/// bound revision 互证（漂移即拒）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingStartRequest {
    pub command_id: String,
    pub expected_plan_revision_id: String,
}

/// N3（REQ-CS-06）：plan 聚合中的 target 仓投影。`lease_state` 沿现 repo
/// worktree lock 事实（`absent|held|contested`），不造 provider_running 猜测。
///
/// S3（Q5 逐仓 PR 面板数据源 A）：九个 additive 字段承载逐仓交付事实，
/// 类型均 `string | null`——`None` = 无对应事实（成员 attempt 已删/attempt
/// 面字段缺失、无 ReviewRequest、无人工恢复记录）。`stage`/`push_status`
/// 为 snake_case 文本（`coding_execution_stage_text`/`push_status_text`
/// 值域，`push_status` 含 `skipped_not_configured`）；`repository_name`
/// 解析自 checkout 路径末段（`resolve_plan_target_repository_name` 同款，
/// 回落 id 字符串）；`manual_recovery_reason` 为开放值集原样透传（如
/// `coding_runner_failed_while_running`）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingTargetDto {
    pub target_repository_id: String,
    pub attempt_id: String,
    pub attempt_status: crate::product::coding_models::CodingAttemptStatus,
    pub worktree_path: Option<PathBuf>,
    pub lease_state: String,
    pub repository_name: Option<String>,
    pub branch_name: Option<String>,
    pub base_branch: Option<String>,
    pub stage: Option<String>,
    pub head_commit: Option<String>,
    pub push_status: Option<String>,
    pub push_error: Option<String>,
    pub review_request_id: Option<String>,
    pub manual_recovery_reason: Option<String>,
}

/// N3（REQ-CS-06）：plan 聚合中的 WI 进度投影。`execution_state` 值域
/// `initialized|running|waiting_dependency|waiting_slot|paused|completed|invalidated`
///（invalidated=run 授权失效，完成事实仍投影 completed），provider 事实
///（claim 相位 ≥ ProviderMayHaveStarted）与初始化 unit Running 区分；
/// S3（Q3 投影真实化，additive）：`provider_started` 需 attempt.status==
/// Running 才投影 `running`；attempt 级屏幕优先于 unit 级等待投影——
/// `awaiting_manual_recovery`（AMR 现场 unit 仍 Running，不改写底层事实）、
/// `waiting_human`（Blocked/WaitingForHuman）、`failed`、`aborted`。
/// `waiting_reason` 编排等待子集值域
/// `dependency_not_ready|execution_slot_occupied|plan_coding_continuation_not_enabled`；
/// `awaiting_manual_recovery` 时为 `manual_recovery_reason` 开放值集透传。
/// Slice B（仪表盘跨仓依赖投影）：另带全量直接依赖（权威绑定，跨桶不失真）、
/// 全图拓扑序 `order_index` 与桶内 `summary`，前端据此重建 plan 全图拓扑/依赖链。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingUnitProgressDto {
    pub unit_id: String,
    pub logical_work_item_id: String,
    pub target_repository_id: String,
    pub attempt_id: String,
    pub unit_status: crate::product::coding_models::CodingExecutionUnitStatus,
    pub execution_state: String,
    pub unsatisfied_dependency_ids: Vec<String>,
    pub waiting_reason: Option<String>,
    pub dependency_logical_work_item_ids: Vec<String>,
    pub order_index: usize,
    pub summary: Option<String>,
}

/// N3（REQ-CS-06）：按 bound target attempts 聚合的 plan 进度（GET/POST 共用
/// 读函数产出）。primary 仅表示打开入口，不代表整 plan 执行或完成。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingProgressDto {
    pub plan_id: String,
    pub bound_plan_revision_id: String,
    pub command_id: Option<String>,
    pub mode: Option<String>,
    pub phase: Option<String>,
    pub primary_attempt_id: Option<String>,
    pub selected_unit_id: Option<String>,
    pub selected_attempt_id: Option<String>,
    pub target_attempts: Vec<PlanCodingTargetDto>,
    pub units: Vec<PlanCodingUnitProgressDto>,
    /// P1-a（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认就绪投影。additive：
    /// 旧客户端反序列化缺省为 not_ready 空投影；GET 只看持久事实，不检查
    /// git（owner/范围/git 仍由逐项 `handle_final_confirm` 提交时校验）。
    #[serde(default)]
    pub final_confirm: PlanCodingFinalConfirmDto,
    /// P4-c（REQ-CRO-12 / 契约 A1）：本 plan 同源的待处理事项（choice/
    /// triage/blocked/stalled/manual_recovery/plan_final_confirm；多 target
    /// 聚合抑制逐仓 final_confirm）。additive：旧客户端缺省为空。
    #[serde(default)]
    pub attention_items: Vec<crate::web::coding_attention_info::CodingAttentionItemDto>,
    pub plan_complete: bool,
    /// H4（设计 C 节，H5 冻结形状对齐）：workspace_lifecycle 事实——
    /// generation 单调、现存/缺失成员计数与状态；数据来自 run/advance/
    /// journal 交叉核验，不从 attempt_status 猜物理存在性。
    pub workspace_lifecycle: PlanWorkspaceLifecycleDto,
}

/// H4（设计 C 节）：plan Coding 工作区生命周期事实投影（GET/DELETE 共用）。
/// `state` 值域 `clean|complete|incomplete|clearing`——clean=无执行内容、
/// complete=成员齐全、incomplete=成员缺失（部分删除后）、clearing=清理进行中。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanWorkspaceLifecycleDto {
    pub generation: u64,
    pub existing_member_count: usize,
    pub missing_member_count: usize,
    pub state: String,
}

/// P1-a（REQ-MTG-11 / 契约 A2）：未齐备的冻结 target（GET `blocking` 条目；
/// POST not_ready 回执的失败项同构）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmBlockingDto {
    pub attempt_id: String,
    pub reason_code: String,
    pub message: String,
}

/// T18（REQ-GFR-CR-05 / REQ-GFR-CR-06）：issue 级跨仓最终评审的 advisory
/// 警告——**仅呈现**，不参与 ready/blocked 判定与 state（advisory 红线）。
/// 结论为通过（含 Failed/Skipped 记录，其 verdict 恒为 approve）时不出现。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CrossRepoReviewWarning {
    /// 值域 `request_changes|blocked`。
    pub verdict: String,
    pub summary: String,
    pub findings_count: usize,
}

/// P1-a（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认投影——
/// `state` 值域 `not_ready|ready|partial|done`：Confirmed 视为已确认；
/// 全 Completed → done；部分 Completed 且剩余全就绪 → partial；零确认且
/// 全就绪 → ready；其余（含 blocking 非空/plan 未执行完成）→ not_ready。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmDto {
    pub state: String,
    pub ready_attempt_ids: Vec<String>,
    pub confirmed_attempt_ids: Vec<String>,
    pub blocking: Vec<PlanCodingFinalConfirmBlockingDto>,
    /// T18（REQ-GFR-CR-05/06）：跨仓最终评审 advisory 警告；仅呈现，
    /// 不参与 ready/blocked 与 state 判定。单仓 issue / 无评审记录 / 结论为
    /// 通过时为 `None`（单仓路径序列化不变）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_repo_review_warning: Option<CrossRepoReviewWarning>,
    /// T18（REQ-GFR-CR-06）：该 issue 的跨仓评审是否已有「已知风险，仍然
    /// 确认」留痕。仅存在评审记录时才可能为 true；`false` 视为可选字段缺省
    /// （Plan 7.2「最终确认卡片两个可选字段」），单仓 GET 不得多出该键。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cross_repo_review_acknowledged: bool,
}

impl Default for PlanCodingFinalConfirmDto {
    fn default() -> Self {
        Self {
            state: "not_ready".to_string(),
            ready_attempt_ids: Vec::new(),
            confirmed_attempt_ids: Vec::new(),
            blocking: Vec::new(),
            cross_repo_review_warning: None,
            cross_repo_review_acknowledged: false,
        }
    }
}

/// P1-a（契约 A2）：POST `.../coding-final-confirm` 回执逐仓结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmResultItemDto {
    pub attempt_id: String,
    pub target_repository_id: String,
    /// 值域 `confirmed|already_completed|failed|not_attempted`。
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// P1-a（契约 A2）：POST `.../coding-final-confirm` 回执——
/// `outcome` 值域 `all_confirmed|partially_confirmed|not_ready`；partial 只
/// 表示本次未全成，成功数按 `results` 计算。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingFinalConfirmResultDto {
    pub plan_id: String,
    pub outcome: String,
    pub results: Vec<PlanCodingFinalConfirmResultItemDto>,
}

/// H4（设计 C 节，H5 冻结名对齐）：DELETE …/coding-workspaces 成功回执——
/// `operation_state: accepted|replayed`（replayed=同 command 幂等重放，不扫
/// 下一代刚创建的 attempts）；`generation` 为操作完成后的当前轮次；
/// `deleted/remaining_attempt_ids` 为本次操作权威范围结果（服务端枚举，
/// 不收前端 attempt ID 列表）；`workspace_lifecycle` 为操作后事实投影。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingWorkspaceDeletionResultDto {
    pub plan_id: String,
    pub command_id: String,
    pub operation_state: String,
    pub generation: u64,
    pub deleted_attempt_ids: Vec<String>,
    pub remaining_attempt_ids: Vec<String>,
    pub workspace_lifecycle: PlanWorkspaceLifecycleDto,
}

/// H4（设计 C 节）：组删 DELETE 的 query 参数——稳定 `command_id`（同动作
/// 重试复用）、`expected_generation`（GET workspace_lifecycle.generation 互证，
/// 未知时省略）、`confirmed=true`（仅确认弹窗同意后显式携带；query 布尔不是
/// 安全授权，服务端分类才是权威）。
#[derive(Debug, Default, serde::Deserialize)]
pub struct PlanCodingWorkspaceDeletionQuery {
    pub command_id: String,
    pub expected_generation: Option<u64>,
    pub confirmed: Option<bool>,
}

/// N3（REQ-CS-01/02）：一键启动结果——`operation_state` 沿现
/// `accepted|replayed|needs_human|rejected` 语义；`start_outcome` 为冻结的
/// 首启真实回执。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanCodingStartResultDto {
    pub plan_id: String,
    pub bound_plan_revision_id: String,
    pub command_id: String,
    pub operation_state: crate::product::models::automation::OperationState,
    pub primary_attempt_id: Option<String>,
    pub target_attempts: Vec<PlanCodingTargetDto>,
    pub selected_unit_id: Option<String>,
    pub selected_attempt_id: Option<String>,
    pub start_outcome: Option<crate::product::coding_plan_run::CodingPlanStartOutcome>,
    pub waiting_units: Vec<PlanCodingUnitProgressDto>,
    pub progress: PlanCodingProgressDto,
}
