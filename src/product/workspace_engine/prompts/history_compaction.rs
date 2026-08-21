use crate::product::workspace_engine::SessionMessage;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ReviewFindingSeverity, ReviewVerdict, ReviewVerdictType,
    TimelineNode,
};

const RECENT_RAW_ROUNDS: usize = 2;
const MAX_DIFF_LINES_PER_SIDE: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryCompactionMode {
    Author,
    Reviewer,
}

pub(super) struct HistoryCompactionInput<'a> {
    pub(super) messages: &'a [SessionMessage],
    pub(super) artifact_versions: &'a [ArtifactVersion],
    pub(super) timeline_nodes: &'a [TimelineNode],
    pub(super) latest_review_verdict: Option<&'a ReviewVerdict>,
    pub(super) mode: HistoryCompactionMode,
}

pub(super) struct CompactedHistory {
    pub(super) rendered: String,
    /// `true` 表示压缩过程 fail-closed 退回了全量重放（未做任何有损压缩）。
    /// 生产调用方当前只消费 `rendered`；此字段由单测断言 fail-closed 语义，并作为
    /// 未来的可观测性信号保留，故显式豁免 dead_code 警告。
    #[allow(dead_code)]
    pub(super) used_full_replay_fallback: bool,
}

/// Compresses only history whose round can be proven from a persisted artifact version.
///
/// This deliberately fails closed: uncertain message/version correlation, malformed audit
/// content, or a summary rendering error returns the exact full history rather than silently
/// omitting a constraint. Choice audits and canonical reviewer inputs are always replayed.
pub(super) fn compact_history(input: HistoryCompactionInput<'_>) -> CompactedHistory {
    let full_replay = || CompactedHistory {
        rendered: render_history_for_mode(input.messages, input.mode),
        used_full_replay_fallback: true,
    };

    if input
        .messages
        .iter()
        .any(|message| message.content.contains('\0'))
        || input
            .artifact_versions
            .iter()
            .any(|version| version.markdown().contains('\0'))
    {
        return full_replay();
    }

    let Some(rounds) = round_boundaries(input.messages, input.artifact_versions) else {
        return CompactedHistory {
            rendered: render_history_for_mode(input.messages, input.mode),
            used_full_replay_fallback: false,
        };
    };
    if rounds.len() <= RECENT_RAW_ROUNDS {
        return CompactedHistory {
            rendered: render_history_for_mode(input.messages, input.mode),
            used_full_replay_fallback: false,
        };
    }

    match render_compacted_history(&input, &rounds) {
        Ok(rendered) => CompactedHistory {
            rendered,
            used_full_replay_fallback: false,
        },
        Err(()) => full_replay(),
    }
}

/// Renders full, actionable required findings separately so callers can place the block next
/// to the current artifact. `latest_review_verdict` is authoritative for whether a finding is
/// still open; prior reviewer prose alone is never guessed to be open.
pub(super) fn render_open_required_findings(
    latest_review_verdict: Option<&ReviewVerdict>,
) -> String {
    let Some(verdict) = latest_review_verdict else {
        return String::new();
    };
    if verdict.review_gate != crate::web::workspace_ws_types::ReviewGate::RequiresRevision {
        return String::new();
    }

    let findings = verdict
        .findings
        .iter()
        .enumerate()
        .filter(|(_, finding)| is_required(&finding.severity))
        .map(|(index, finding)| {
            format!(
                "- finding_id: finding-{}\n  severity: {}\n  message: {}\n  evidence: {}\n  required_action: {}\n",
                index + 1,
                severity_name(&finding.severity),
                finding.message.trim(),
                finding.evidence.trim(),
                finding.required_action.trim(),
            )
        })
        .collect::<Vec<_>>();

    if findings.is_empty() {
        String::new()
    } else {
        format!(
            "[未关闭 blocking/must_fix finding（全文）]\n{}",
            findings.join("")
        )
    }
}

/// Provides deterministic adjacent-version context for reviewer prompts. Every pair is a
/// concise line-based diff; when a diff cannot be generated, both artifacts are replayed in
/// full for that pair rather than dropped.
pub(super) fn render_intermediate_artifact_diffs(artifact_versions: &[ArtifactVersion]) -> String {
    let mut versions = artifact_versions
        .iter()
        .filter(|version| !version.markdown().is_empty())
        .collect::<Vec<_>>();
    versions.sort_by_key(|version| version.version);
    if versions.len() <= 2 {
        return String::new();
    }

    versions
        .windows(2)
        .map(|pair| render_adjacent_artifact_diff(pair[0], pair[1]))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_compacted_history(
    input: &HistoryCompactionInput<'_>,
    rounds: &[RoundBoundary],
) -> Result<String, ()> {
    let earliest_recent_round = rounds.len() - RECENT_RAW_ROUNDS + 1;
    let latest_artifact = input
        .artifact_versions
        .iter()
        .max_by_key(|version| version.version)
        .filter(|version| !version.markdown().is_empty());
    let mut rendered = String::new();
    let mut cursor = 0;

    for boundary in rounds {
        let round_messages = &input.messages[cursor..boundary.next_start];
        if boundary.round_index >= earliest_recent_round {
            render_messages(&mut rendered, round_messages, input.mode);
        } else {
            rendered.push_str(&render_round_summary(
                boundary,
                round_messages,
                input.timeline_nodes,
            )?);
            for message in round_messages {
                if must_replay_raw(message, input.mode, latest_artifact) {
                    render_messages(&mut rendered, std::slice::from_ref(message), input.mode);
                }
            }
        }
        cursor = boundary.next_start;
    }

    if cursor < input.messages.len() {
        render_messages(&mut rendered, &input.messages[cursor..], input.mode);
    }
    let open_findings = render_open_required_findings(input.latest_review_verdict);
    if !open_findings.is_empty() {
        rendered.push('\n');
        rendered.push_str(&open_findings);
    }
    Ok(rendered)
}

#[derive(Debug, Clone, Copy)]
struct RoundBoundary {
    round_index: usize,
    artifact_version: u32,
    artifact_review_verdict: Option<&'static str>,
    next_start: usize,
}

/// Returns contiguous history slices ending immediately before each next artifact output.
/// An artifact version must have one unambiguous message containing its markdown. Otherwise
/// preserving semantic history is safer than attempting a lossy window.
fn round_boundaries(
    messages: &[SessionMessage],
    artifact_versions: &[ArtifactVersion],
) -> Option<Vec<RoundBoundary>> {
    if artifact_versions.len() < 3 {
        return None;
    }

    let mut positions = Vec::with_capacity(artifact_versions.len());
    for artifact in artifact_versions {
        let markdown = artifact.markdown();
        if markdown.trim().is_empty() {
            return None;
        }
        let mut matches = messages.iter().enumerate().filter_map(|(index, message)| {
            message_contains_artifact(message, markdown).then_some(index)
        });
        let position = matches.next()?;
        if matches.next().is_some() || positions.last().is_some_and(|last| *last >= position) {
            return None;
        }
        positions.push(position);
    }

    Some(
        positions
            .iter()
            .enumerate()
            .map(|(index, position)| {
                let next_artifact_position =
                    positions.get(index + 1).copied().unwrap_or(messages.len());
                let next_start = messages
                    .iter()
                    .enumerate()
                    .skip(position + 1)
                    .take(next_artifact_position.saturating_sub(position + 1))
                    .find_map(|(message_index, message)| {
                        (message.role == "user").then_some(message_index)
                    })
                    .unwrap_or(next_artifact_position);
                RoundBoundary {
                    round_index: index + 1,
                    artifact_version: artifact_versions[index].version,
                    artifact_review_verdict: artifact_versions[index]
                        .review_verdict
                        .as_ref()
                        .map(review_verdict_name),
                    next_start,
                }
            })
            .collect(),
    )
}

fn message_contains_artifact(message: &SessionMessage, markdown: &str) -> bool {
    message.content == markdown || message.content.contains(markdown)
}

fn render_round_summary(
    boundary: &RoundBoundary,
    messages: &[SessionMessage],
    timeline_nodes: &[TimelineNode],
) -> Result<String, ()> {
    let mut summary = String::new();
    summary.push_str(&format!("[历史压缩摘要 round={}]\n", boundary.round_index));
    summary.push_str(&format!(
        "- artifact_version: v{}\n",
        boundary.artifact_version
    ));
    summary.push_str(&format!(
        "- review_verdict: {}\n",
        boundary.artifact_review_verdict.unwrap_or("unavailable")
    ));

    let verdicts = messages
        .iter()
        .filter(|message| message.role == "reviewer")
        .filter_map(|message| extract_review_summary(&message.content))
        .collect::<Vec<_>>();
    for verdict in verdicts {
        summary.push_str(&format!("- review_summary: {verdict}\n"));
    }

    let findings = messages
        .iter()
        .filter(|message| message.role == "reviewer")
        .flat_map(|message| extract_findings(&message.content))
        .collect::<Vec<_>>();
    if findings.is_empty() {
        summary.push_str("- findings: none_recorded\n");
    } else {
        for (index, finding) in findings.iter().enumerate() {
            summary.push_str(&format!(
                "- finding_id: round-{}-finding-{}\n  severity: {}\n  required_action: {}\n",
                boundary.round_index,
                index + 1,
                finding.severity,
                finding.required_action,
            ));
        }
    }

    for audit in messages
        .iter()
        .filter(|message| is_choice_audit(message))
        .map(|message| summarize_choice_audit(&message.content))
    {
        summary.push_str(&audit?);
    }

    for retry in timeline_nodes
        .iter()
        .filter(|node| node.round == Some(boundary.round_index as u32))
        .filter_map(|node| node.retry.as_ref())
    {
        summary.push_str(&format!(
            "- retry_failure_reason: {} — {}\n",
            one_line(&retry.retry_reason),
            one_line(&retry.retry_error.message)
        ));
    }

    Ok(summary)
}

fn must_replay_raw(
    message: &SessionMessage,
    mode: HistoryCompactionMode,
    latest_artifact: Option<&ArtifactVersion>,
) -> bool {
    is_choice_audit(message)
        || (mode == HistoryCompactionMode::Reviewer && has_canonical_inputs(&message.content))
        || latest_artifact.is_some_and(|artifact| {
            !artifact.markdown().is_empty()
                && message_contains_artifact(message, artifact.markdown())
        })
}

fn is_choice_audit(message: &SessionMessage) -> bool {
    message
        .content
        .contains("结构化交互审计记录（daemon 捕获）")
        || message.content.contains("AskUserQuestion")
}

fn has_canonical_inputs(content: &str) -> bool {
    content.contains("[canonical_inputs]\n")
}

fn extract_review_summary(content: &str) -> Option<String> {
    let value = section_value(content, "[review_summary]")?;
    Some(one_line(value))
}

#[derive(Debug)]
struct FindingSummary {
    severity: String,
    required_action: String,
}

fn extract_findings(content: &str) -> Vec<FindingSummary> {
    let Some(section) = section_value(content, "[review_findings]") else {
        return Vec::new();
    };

    let mut findings = Vec::new();
    let mut severity = None;
    let mut action = None;
    for line in section.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed
            .strip_prefix("severity:")
            .or_else(|| trimmed.split_once("severity:").map(|(_, value)| value))
        {
            if let Some(severity) = severity.take() {
                findings.push(FindingSummary {
                    severity,
                    required_action: action
                        .take()
                        .unwrap_or_else(|| "(not recorded)".to_string()),
                });
            }
            severity = Some(one_line(value));
        } else if let Some(value) = trimmed.strip_prefix("required_action:") {
            action = Some(one_line(value));
        }
    }
    if let Some(severity) = severity {
        findings.push(FindingSummary {
            severity,
            required_action: action.unwrap_or_else(|| "(not recorded)".to_string()),
        });
    }
    findings
}

fn summarize_choice_audit(content: &str) -> Result<String, ()> {
    let choice_id = audit_value(content, "choice_id:").ok_or(())?;
    let answers = content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("selected:"))
        .map(one_line)
        .collect::<Vec<_>>();
    let impacts = content
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("impacts:").or_else(|| {
                trimmed
                    .strip_prefix("impact:")
                    .filter(|_| trimmed.contains("REQ-") || trimmed.contains("AC-"))
            })
        })
        .map(one_line)
        .unwrap_or_else(|| "not_recorded".to_string());
    Ok(format!(
        "- choice_audit: id={choice_id}; answers={}; impacts={impacts}\n",
        if answers.is_empty() {
            "not_recorded".to_string()
        } else {
            answers.join("; ")
        }
    ))
}

fn audit_value<'a>(content: &'a str, key: &str) -> Option<&'a str> {
    content
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("- ")
                .unwrap_or(line.trim())
                .strip_prefix(key)
                .map(str::trim)
        })
        .filter(|value| !value.is_empty())
}

fn section_value<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let start = content.find(heading)? + heading.len();
    let remainder = &content[start..];
    let end = remainder.find("\n\n[").unwrap_or(remainder.len());
    Some(remainder[..end].trim())
}

fn render_adjacent_artifact_diff(previous: &ArtifactVersion, current: &ArtifactVersion) -> String {
    match adjacent_artifact_diff(previous, current) {
        Ok(diff) => format!(
            "[artifact v{} -> v{} 相邻版本差异摘要]\n{}\n",
            previous.version, current.version, diff
        ),
        Err(()) => format!(
            "[artifact v{} -> v{} 相邻版本 diff 生成失败；保留全文]\n\
             ### artifact v{}\n{}\n\
             ### artifact v{}\n{}\n",
            previous.version,
            current.version,
            previous.version,
            previous.markdown(),
            current.version,
            current.markdown(),
        ),
    }
}

fn adjacent_artifact_diff(
    previous: &ArtifactVersion,
    current: &ArtifactVersion,
) -> Result<String, ()> {
    if previous.markdown().contains('\0') || current.markdown().contains('\0') {
        return Err(());
    }
    if let ArtifactPayload::Markdown {
        diff: Some(diff), ..
    } = &current.payload
        && !diff.trim().is_empty()
    {
        return Ok(format!("- provided_diff: {}", one_line(diff)));
    }

    let previous_lines = previous.markdown().lines().collect::<Vec<_>>();
    let current_lines = current.markdown().lines().collect::<Vec<_>>();
    let removed = previous_lines
        .iter()
        .filter(|line| !current_lines.contains(line))
        .take(MAX_DIFF_LINES_PER_SIDE)
        .map(|line| format!("- {}", diff_line_preview(line)))
        .collect::<Vec<_>>();
    let added = current_lines
        .iter()
        .filter(|line| !previous_lines.contains(line))
        .take(MAX_DIFF_LINES_PER_SIDE)
        .map(|line| format!("+ {}", diff_line_preview(line)))
        .collect::<Vec<_>>();

    if removed.is_empty() && added.is_empty() {
        Ok("- no textual changes".to_string())
    } else {
        Ok([removed, added]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn diff_line_preview(line: &str) -> String {
    const MAX_DIFF_LINE_CHARS: usize = 160;
    let preview = line.chars().take(MAX_DIFF_LINE_CHARS).collect::<String>();
    if line.chars().count() > MAX_DIFF_LINE_CHARS {
        format!("{preview}…")
    } else {
        preview
    }
}

fn is_required(severity: &ReviewFindingSeverity) -> bool {
    matches!(
        severity,
        ReviewFindingSeverity::Blocking | ReviewFindingSeverity::MustFix
    )
}

fn review_verdict_name(verdict: &ReviewVerdictType) -> &'static str {
    match verdict {
        ReviewVerdictType::Pass => "pass",
        ReviewVerdictType::Revise => "revise",
        ReviewVerdictType::NeedsHuman => "needs_human",
    }
}

fn severity_name(severity: &ReviewFindingSeverity) -> &'static str {
    match severity {
        ReviewFindingSeverity::Blocking => "blocking",
        ReviewFindingSeverity::MustFix => "must_fix",
        ReviewFindingSeverity::Suggestion => "suggestion",
    }
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_messages(
    rendered: &mut String,
    messages: &[SessionMessage],
    mode: HistoryCompactionMode,
) {
    for message in messages {
        let content = match mode {
            HistoryCompactionMode::Author => Some(message.content.clone()),
            HistoryCompactionMode::Reviewer => {
                super::reviewer_context_filter::reviewer_context_content(message)
            }
        };
        if let Some(content) = content {
            rendered.push_str(&format!("[{}]: {content}\n", message.role));
        }
    }
}

fn render_history_for_mode(messages: &[SessionMessage], mode: HistoryCompactionMode) -> String {
    let mut rendered = String::new();
    render_messages(&mut rendered, messages, mode);
    rendered
}

/// 全量重放的参考实现，仅作为 fail-closed / 字符数下降断言的测试基准。
/// 生产路径统一走 `compact_history` 内的 `render_history_for_mode`，不再单独调用。
#[cfg(test)]
fn render_full_history(messages: &[SessionMessage]) -> String {
    render_history_for_mode(messages, HistoryCompactionMode::Author)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::{
        ArtifactPayload, ProviderConfigSnapshot, ReviewFinding, ReviewGate, ReviewVerdictType,
        TimelineNode, TimelineNodeRetry, TimelineNodeRetryError, TimelineNodeStatus,
        TimelineNodeType, WorkspaceStage,
    };

    fn message(id: usize, role: &str, content: impl Into<String>) -> SessionMessage {
        SessionMessage {
            id: format!("msg_{id:03}"),
            role: role.to_string(),
            content: content.into(),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:00:{id:02}Z"),
        }
    }

    fn artifact(version: u32, markdown: impl Into<String>, is_current: bool) -> ArtifactVersion {
        ArtifactVersion {
            version,
            payload: ArtifactPayload::Markdown {
                markdown: markdown.into(),
                diff: None,
            },
            generated_by: ProviderName::Codex,
            reviewed_by: Some(ProviderName::ClaudeCode),
            review_verdict: Some(ReviewVerdictType::Revise),
            confirmed_by: None,
            is_current,
            created_at: format!("2026-08-21T00:00:{version:02}Z"),
            source_node_id: format!("author_{version}"),
        }
    }

    fn four_round_history() -> (Vec<SessionMessage>, Vec<ArtifactVersion>) {
        let artifacts = (1..=4)
            .map(|round| {
                artifact(
                    round,
                    format!(
                        "# Artifact v{round}\n\n- [REQ-{round:03}] {}\n- [AC-{round:03}] {}",
                        "early artifact body ".repeat(20),
                        "acceptance detail ".repeat(20),
                    ),
                    round == 4,
                )
            })
            .collect::<Vec<_>>();
        let mut messages = vec![message(
            1,
            "system",
            "Workspace 生成任务已准备\n\n[canonical_inputs]\n完整 canonical input：不得省略。\n\n[constraint_summary]\n约束。",
        )];
        for round in 1..=4 {
            messages.push(message(
                round as usize * 3 - 1,
                "user",
                format!("ROUND-{round}-USER 原文 {}", "detail ".repeat(20)),
            ));
            messages.push(message(
                round as usize * 3,
                "assistant",
                artifacts[round as usize - 1].markdown().to_string(),
            ));
            messages.push(message(
                round as usize * 3 + 1,
                "reviewer",
                format!(
                    "[review_summary]\nROUND-{round}-REVIEW 原文\n\n[review_findings]\n1. severity: suggestion\n   message: 仅作建议\n   evidence: evidence\n   required_action: optional"
                ),
            ));
        }
        (messages, artifacts)
    }

    fn open_must_fix() -> ReviewVerdict {
        ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "必须修复 REQ-001 的完整追踪关系。".to_string(),
            summary: "REQ-001 仍未关闭".to_string(),
            findings: vec![ReviewFinding {
                severity: ReviewFindingSeverity::MustFix,
                message: "ROUND-1-MUST-FIX-FULL-TEXT：REQ-001 缺少来源到 AC-001 的追踪。"
                    .to_string(),
                evidence: "artifact v1, REQ-001".to_string(),
                required_action: "补齐 REQ-001 -> AC-001 追踪并保留 source id。".to_string(),
            }],
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        }
    }

    #[test]
    fn compacts_early_rounds_but_keeps_two_recent_rounds_choice_audit_and_latest_artifact() {
        let (mut messages, artifacts) = four_round_history();
        messages.insert(
            4,
            message(
                99,
                "system",
                "结构化交互审计记录（daemon 捕获）\n- choice_id: choice_rollout\n- answers:\n  - question_id: q1\n    selected: gradual = 分批发布\n- impacts: REQ-001, AC-001",
            ),
        );

        assert!(
            round_boundaries(&messages, &artifacts).is_some(),
            "fixture artifact messages must correlate to persisted versions"
        );
        let compacted = compact_history(HistoryCompactionInput {
            messages: &messages,
            artifact_versions: &artifacts,
            timeline_nodes: &[],
            latest_review_verdict: Some(&open_must_fix()),
            mode: HistoryCompactionMode::Author,
        });

        assert!(
            compacted.rendered.contains("[历史压缩摘要 round=1]"),
            "早期轮必须以确定性结构化摘要出现：{}",
            compacted.rendered
        );
        assert!(
            !compacted.rendered.contains("ROUND-1-USER 原文"),
            "早期轮原文不得完整重放：{}",
            compacted.rendered
        );
        assert!(compacted.rendered.contains("ROUND-3-USER 原文"));
        assert!(compacted.rendered.contains("ROUND-4-USER 原文"));
        assert!(compacted.rendered.contains("choice_rollout"));
        assert!(compacted.rendered.contains("gradual = 分批发布"));
        assert!(compacted.rendered.contains("REQ-001, AC-001"));
        assert!(
            compacted.rendered.contains("# Artifact v4"),
            "最新 artifact 必须全文保留"
        );
        assert!(
            compacted.rendered.len() < render_full_history(&messages).len(),
            "字符数下降仅是代理指标，不是 release gate"
        );
    }

    #[test]
    fn failed_summary_generation_fails_closed_to_full_replay() {
        let messages = vec![
            message(1, "user", "正常历史"),
            message(2, "assistant", "包含 NUL 的历史\0，不允许静默丢弃"),
            message(3, "user", "最近一轮"),
        ];

        let compacted = compact_history(HistoryCompactionInput {
            messages: &messages,
            artifact_versions: &[],
            timeline_nodes: &[],
            latest_review_verdict: None,
            mode: HistoryCompactionMode::Author,
        });

        assert!(compacted.used_full_replay_fallback);
        assert_eq!(compacted.rendered, render_full_history(&messages));
    }

    #[test]
    fn reviewer_keeps_open_required_finding_and_summarizes_intermediate_artifact_diffs() {
        let (messages, artifacts) = four_round_history();
        let verdict = open_must_fix();
        let compacted = compact_history(HistoryCompactionInput {
            messages: &messages,
            artifact_versions: &artifacts,
            timeline_nodes: &[],
            latest_review_verdict: Some(&verdict),
            mode: HistoryCompactionMode::Reviewer,
        });
        let open_findings = render_open_required_findings(Some(&verdict));
        let diffs = render_intermediate_artifact_diffs(&artifacts);

        assert!(compacted.rendered.contains("[历史压缩摘要 round=1]"));
        assert!(
            open_findings.contains("ROUND-1-MUST-FIX-FULL-TEXT"),
            "第 4 轮 reviewer 仍必须定位未关闭强返修项"
        );
        assert!(open_findings.contains("finding_id: finding-1"));
        assert!(diffs.contains("artifact v1 -> v2"));
        assert!(diffs.contains("相邻版本差异摘要"));
        assert!(
            !diffs.contains(artifacts[0].markdown()),
            "中间版本必须以 diff 摘要而非 artifact 全文呈现"
        );
    }

    #[test]
    fn early_round_summary_includes_retry_failure_reason() {
        let (messages, artifacts) = four_round_history();
        let retry_node = TimelineNode {
            node_id: "review_round_1_retry".to_string(),
            node_type: TimelineNodeType::ReviewerRun,
            agent: Some(ProviderName::ClaudeCode),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            status: TimelineNodeStatus::Failed,
            title: "retry".to_string(),
            summary: None,
            started_at: "2026-08-21T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 4,
                permission_modes: Default::default(),
            },
            retry: Some(TimelineNodeRetry {
                retry_of_node_id: "review_round_1".to_string(),
                retry_attempt: 1,
                retry_reason: "schema parse failed".to_string(),
                retry_error: TimelineNodeRetryError {
                    code: "invalid_schema".to_string(),
                    message: "required_action missing".to_string(),
                },
            }),
        };
        let compacted = compact_history(HistoryCompactionInput {
            messages: &messages,
            artifact_versions: &artifacts,
            timeline_nodes: &[retry_node],
            latest_review_verdict: None,
            mode: HistoryCompactionMode::Author,
        });

        assert!(
            compacted
                .rendered
                .contains("retry_failure_reason: schema parse failed — required_action missing")
        );
    }

    #[test]
    fn reviewer_canonical_input_is_replayed_even_when_its_round_is_compacted() {
        let (messages, artifacts) = four_round_history();
        let compacted = compact_history(HistoryCompactionInput {
            messages: &messages,
            artifact_versions: &artifacts,
            timeline_nodes: &[],
            latest_review_verdict: None,
            mode: HistoryCompactionMode::Reviewer,
        });

        assert!(
            compacted
                .rendered
                .contains("完整 canonical input：不得省略。")
        );
    }

    #[test]
    fn diff_generation_failure_replays_both_adjacent_artifact_bodies() {
        let previous = artifact(1, "# v1\0", false);
        let current = artifact(2, "# v2", false);
        let latest = artifact(3, "# v3", true);
        let rendered = render_intermediate_artifact_diffs(&[previous, current, latest]);

        assert!(rendered.contains("diff 生成失败；保留全文"));
        assert!(rendered.contains("# v1\0"));
        assert!(rendered.contains("# v2"));
    }
}
