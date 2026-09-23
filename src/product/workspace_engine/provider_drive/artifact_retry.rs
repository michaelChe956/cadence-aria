//! artifact gate 失败后的自动续写（REQ-ACS-01）与候选选择诊断事件（REQ-ACS-02）。
//!
//! 诊断复用 `NodeDetail.execution_events` 的既有 upsert 通道，但只 durable——不广播
//! `EngineEvent`，因此不新增 WebSocket/UI 公开语义（change 全局边界）。稳定 event id 为
//! `artifact_diag_{node_id 前 8}_{raw_output_sha256 前 8}`，`output=None`、detail 只带
//! 候选边界/字节范围/hash 与截断后的阻断原因：原文零复制（原文已持久化于 session
//! assistant message / streaming content）。

use super::*;

/// REQ-ACS-02 诊断 detail 的 schema 版本（调查者脚本的契约）。
const ARTIFACT_DIAGNOSTIC_VERSION: u32 = 1;
/// detail 列出的候选条数上界：超出只记前 N 条，并置 `candidates_truncated`。
const ARTIFACT_DIAGNOSTIC_MAX_CANDIDATES: usize = 16;
/// 单候选保留的阻断原因条数上界（既有 gate 词汇表固定，此值只作防御）。
const ARTIFACT_DIAGNOSTIC_MAX_REASONS: usize = 16;
/// 单条阻断原因的字符上界：gate 的「待确认项未解决」类原因内嵌正文片段，必须截断。
const ARTIFACT_DIAGNOSTIC_REASON_MAX_CHARS: usize = 256;
/// 诊断持久化错误并入失败摘要时的字符上界（有界提示）。
const ARTIFACT_DIAGNOSTIC_ERROR_MAX_CHARS: usize = 200;
/// spec 逐字钉死的诊断持久化失败提示。
const ARTIFACT_DIAGNOSTIC_PERSISTENCE_FAILED: &str = "artifact diagnostic persistence failed";

/// REQ-ACS-02：候选选择诊断事件（`kind=artifact`、`output=None`、detail=有界 JSON）。
pub(crate) fn artifact_selection_diagnostic_event(
    node_id: &str,
    selection: &CandidateSelection,
    workspace_type: &WorkspaceType,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: artifact_diagnostic_event_id(node_id, &selection.raw_output_sha256),
        kind: ProviderExecutionEventKind::Artifact,
        status: match selection.verdict {
            SelectionVerdict::Unique => ProviderExecutionEventStatus::Completed,
            SelectionVerdict::NoPassing | SelectionVerdict::Ambiguous => {
                ProviderExecutionEventStatus::Failed
            }
        },
        title: "Artifact candidate selection".to_string(),
        detail: Some(artifact_diagnostic_detail(selection, workspace_type).to_string()),
        command: None,
        cwd: None,
        // 原文已持久化于 session assistant message：诊断零复制。
        output: None,
        exit_code: None,
    }
}

/// 稳定 event id：`artifact_diag_{node_id 前 8}_{raw_output_sha256 前 8}`。
///
/// 同一 node 上同一原文重复落诊断即同 id，`upsert_execution_event_json` 覆盖而非追加；
/// 诊断始终写入所属 node 自己的 detail，故不同 node 的同名 id 不会互相覆盖。
fn artifact_diagnostic_event_id(node_id: &str, raw_output_sha256: &str) -> String {
    format!(
        "artifact_diag_{}_{}",
        diagnostic_id_prefix(node_id),
        diagnostic_id_prefix(raw_output_sha256),
    )
}

/// REQ-ACS-02 诊断 detail：workspace type、原文身份（字符数 + sha256）、逐候选
/// 行号/字节范围/正文 hash/gate 与阻断原因，以及 candidate_count/passing_count/selection。
///
/// 有界：候选条数、单候选原因条数与单条原因长度都有上界；不含任何完整原文。
fn artifact_diagnostic_detail(
    selection: &CandidateSelection,
    workspace_type: &WorkspaceType,
) -> serde_json::Value {
    let listed = selection
        .candidates
        .len()
        .min(ARTIFACT_DIAGNOSTIC_MAX_CANDIDATES);
    let candidates = selection.candidates[..listed]
        .iter()
        .map(|candidate| {
            let reasons = candidate
                .blocking_reasons
                .iter()
                .take(ARTIFACT_DIAGNOSTIC_MAX_REASONS)
                .map(|reason| {
                    truncate_diagnostic_text(reason, ARTIFACT_DIAGNOSTIC_REASON_MAX_CHARS)
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "opening_line": candidate.opening_line,
                "closing_line": candidate.closing_line,
                "opening_byte": candidate.opening_byte,
                "closing_byte_end": candidate.closing_byte_end,
                "sha256": candidate.sha256,
                "passed": candidate.passed,
                "blocking_reasons": reasons,
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "diagnostic_version": ARTIFACT_DIAGNOSTIC_VERSION,
        "workspace_type": workspace_type_diagnostic_text(workspace_type),
        "raw_output_chars": selection.raw_output_chars,
        "raw_output_sha256": selection.raw_output_sha256,
        "used_legacy_fallback": selection.used_legacy_fallback,
        "candidates": candidates,
        "candidate_count": selection.candidates.len(),
        "passing_count": selection.candidates.iter().filter(|candidate| candidate.passed).count(),
        "candidates_truncated": listed < selection.candidates.len(),
        "selection": selection_verdict_label(selection.verdict),
    })
}

/// 诊断 JSON 的 workspace type 词表（与 `WorkspaceType` 的 serde `snake_case` 一致）。
fn workspace_type_diagnostic_text(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "story",
        WorkspaceType::Design => "design",
        WorkspaceType::WorkItem => "work_item",
        WorkspaceType::WorkItemPlan => "work_item_plan",
    }
}

/// 有界截断：按字符切，绝不落在 UTF-8 边界中间。
fn truncate_diagnostic_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}

/// id 短前缀（ASCII 安全：过短或非 ASCII 输入原样返回）。
fn diagnostic_id_prefix(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

/// 失败摘要：既有 gate 失败原因 +（诊断未落盘时的）有界提示。
///
/// REQ-ACS-02 scenario「诊断写失败不放宽 gate」：只追加一条提示，gate 结论与失败关闭
/// 行为不变，也不触发额外 provider 启动。
pub(crate) fn artifact_failure_reasons_with_diagnostic(
    selection: &CandidateSelection,
    diagnostic_error: Option<&str>,
) -> Vec<String> {
    let mut reasons = selection_failure_reasons(selection);
    if let Some(error) = diagnostic_error {
        reasons.push(format!(
            "{ARTIFACT_DIAGNOSTIC_PERSISTENCE_FAILED}：{}",
            truncate_diagnostic_text(error, ARTIFACT_DIAGNOSTIC_ERROR_MAX_CHARS),
        ));
    }
    reasons
}

impl WorkspaceEngine {
    /// REQ-ACS-02：把候选选择诊断 upsert 进指定 timeline node 的 execution events。
    ///
    /// durable-only（`update_node_detail` + `execution_event_json` upsert 的既有通道）；
    /// `node_id` 为 `None`（无 active node）时无处可写，视为成功。
    ///
    /// 返回 `Err` 表示诊断本身未落盘——调用方须把失败提示并入失败摘要（见
    /// [`artifact_failure_reasons_with_diagnostic`]），但 gate 结论不受影响。
    pub(crate) async fn record_artifact_selection_diagnostic(
        &mut self,
        node_id: Option<&str>,
        selection: &CandidateSelection,
    ) -> Result<(), String> {
        let Some(node_id) = node_id else {
            return Ok(());
        };
        let event =
            artifact_selection_diagnostic_event(node_id, selection, &self.session.workspace_type);
        let event_json = execution_event_json(&event);
        self.update_node_detail(node_id, |detail| {
            upsert_execution_event_json(&mut detail.execution_events, event_json);
        })
        .await
    }

    pub(crate) async fn begin_artifact_retry_node(
        &mut self,
        previous_node_id: Option<&str>,
        agent: Option<ProviderName>,
        blocking_reasons: &[String],
    ) -> Option<String> {
        let previous_node = previous_node_id.and_then(|node_id| {
            self.timeline_nodes
                .iter()
                .find(|node| node.node_id == node_id)
                .cloned()
        })?;
        let _ = self.flush_stream_buffer(&previous_node.node_id).await;
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let summary = if blocking_reasons.is_empty() {
            format!("Provider 未返回有效的 {artifact_name} artifact")
        } else {
            format!(
                "Provider 未返回有效的 {artifact_name} artifact：{}",
                blocking_reasons.join("；")
            )
        };
        self.update_timeline_node(
            &previous_node.node_id,
            TimelineNodeStatus::Failed,
            Some(summary),
        )
        .await;

        let retry_attempt = previous_node
            .retry
            .as_ref()
            .map(|retry| retry.retry_attempt + 1)
            .unwrap_or(1);
        let retry_error_message = if blocking_reasons.is_empty() {
            format!("缺失或无效的 {artifact_name} artifact")
        } else {
            blocking_reasons.join("；")
        };
        Some(
            self.create_timeline_node_with_retry(
                TimelineNodeDraft {
                    node_type: previous_node.node_type.clone(),
                    agent,
                    stage: workspace_stage_from_ws_stage(&previous_node.stage),
                    round: previous_node.round,
                    title: format!("{artifact_name} 自动续写"),
                    summary: None,
                    status: TimelineNodeStatus::Active,
                },
                Some(TimelineNodeRetry {
                    retry_of_node_id: previous_node.node_id.clone(),
                    retry_attempt,
                    retry_reason: "自动续写缺失或无效 artifact".to_string(),
                    retry_error: TimelineNodeRetryError {
                        code: "workspace_artifact_invalid".to_string(),
                        message: retry_error_message,
                    },
                }),
            )
            .await,
        )
    }

    /// 自动续写 prompt 构造。`blocking_reasons` 由调用方在同一 selection 上一次算出
    /// （失败摘要、诊断事件与 prompt 共用同一份 gate 结果，不重复校验同一原文）。
    pub(crate) fn build_artifact_retry_input(
        &self,
        base_input: &StreamingProviderInput,
        previous_output: &str,
        blocking_reasons: &[String],
        provider_session_id: Option<String>,
    ) -> StreamingProviderInput {
        let mut input = base_input.clone();
        input.prompt = build_artifact_retry_prompt(
            &self.session.workspace_type,
            previous_output,
            blocking_reasons,
        );
        if let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        {
            input.resume_provider_session_id = Some(provider_session_id);
        }
        input
    }
}
