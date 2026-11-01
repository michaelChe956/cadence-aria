import type { ChatEntry, WorkspaceContentRef } from "./chat-entries";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

export const selectWorkspaceHeaderState = (state: WorkspaceWsState) => ({
  sessionId: state.sessionId,
  workspaceType: state.workspaceType,
  providers: state.providers,
  reviewRounds: state.reviewRounds,
  stage: state.stage,
  providerLocked: state.providerLocked,
  providerLockedAt: state.providerLockedAt,
  superpowersEnabled: state.superpowersEnabled,
  openSpecEnabled: state.openSpecEnabled,
});

export function workspaceContentCacheKey(ref: WorkspaceContentRef) {
  if (ref.kind === "provider_prompt") {
    return `provider_prompt:${ref.nodeId}`;
  }
  if (ref.kind === "execution_output") {
    return `execution_output:${ref.nodeId}:${ref.eventId}`;
  }
  if (ref.kind === "node_stream") {
    return `node_stream:${ref.nodeId}`;
  }
  return null;
}

export const selectChatPanelState = (state: WorkspaceWsState) => ({
  chatEntries: state.chatEntries,
  stage: state.stage,
  selectedNodeId: state.selectedNodeId,
});

/**
 * adopt-review-findings T1：取最后一条 review 报告消息的文本。
 * 与对话流 ReviewVerdictEntry 渲染同源（entry.content 为后端推送的 summary，
 * live/rebuild 两路径一致）；不在前端从结构化 verdict 重新格式化。
 */
type ReviewFindingLike = {
  severity?: unknown;
  message?: unknown;
  evidence?: unknown;
  required_action?: unknown;
};

function asTrimmedString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function formatReviewFindings(findings: ReviewFindingLike[]): string {
  return findings
    .map((finding, index) => {
      const severity = asTrimmedString(finding.severity) || String(finding.severity ?? "");
      return [
        `${index + 1}. severity: ${severity}`,
        `   message: ${asTrimmedString(finding.message)}`,
        `   evidence: ${asTrimmedString(finding.evidence)}`,
        `   required_action: ${asTrimmedString(finding.required_action)}`,
      ].join("\n");
    })
    .join("\n");
}

export function selectLatestReviewReport(state: WorkspaceWsState): string | undefined {
  const entry = latestReviewVerdictEntry(state);
  if (!entry) {
    return undefined;
  }
  const content = typeof entry.content === "string" ? entry.content.trim() : "";
  const metadata = entry.metadata;
  const summary = asTrimmedString(metadata?.summary) || content;
  const comments = asTrimmedString(metadata?.comments);
  const findings = Array.isArray(metadata?.findings)
    ? (metadata?.findings as ReviewFindingLike[]).filter(
        (finding) => asTrimmedString(finding.message).length > 0,
      )
    : [];
  if (findings.length === 0) {
    return content.length > 0 ? content : undefined;
  }
  const parts: string[] = [];
  if (summary.length > 0) {
    parts.push(`[review_summary]\n${summary}`);
  }
  if (comments.length > 0) {
    parts.push(`[review_comments]\n${comments}`);
  }
  parts.push(`[review_findings]\n${formatReviewFindings(findings)}`);
  return parts.join("\n\n");
}

/**
 * F-38：最近一次 review 结论里的「可选建议」条数——单版本产物视图的评审结论标签
 * 与 ReviewVerdictEntry 的「可选建议」分组同一判据（severity 非 blocking/must_fix）。
 * 与 selectLatestReviewReport 同源同门：被完成的 revision 顶替即视为过期 → null；
 * 无结论或结论没有 findings → null（渲染面据此不画零条标签，fail-closed 不猜）。
 */
export function selectLatestReviewAdvisoryCount(state: WorkspaceWsState): number | null {
  const entry = latestReviewVerdictEntry(state);
  if (!entry) {
    return null;
  }
  const findings = Array.isArray(entry.metadata?.findings)
    ? (entry.metadata?.findings as ReviewFindingLike[])
    : [];
  if (findings.length === 0) {
    return null;
  }
  return findings.filter(
    (finding) => finding.severity !== "blocking" && finding.severity !== "must_fix",
  ).length;
}

/**
 * 最新 review_verdict 条目；被其后的 completed revision 顶替时视为过期。
 */
function latestReviewVerdictEntry(state: WorkspaceWsState): ChatEntry | undefined {
  const entry = state.chatEntries
    .filter((candidate) => candidate.type === "review_verdict")
    .at(-1);
  if (!entry || reviewIsSupersededByRevision(state, entry.node_id)) {
    return undefined;
  }
  return entry;
}

/**
 * Timeline 顺序是节点的因果顺序，且重建 chat entries 时会按该顺序生成；它比 entry
 * timestamp 可靠，因为后端事件可落在同一秒。只要 review 节点之后已有完成的 revision，
 * review 报告就针对旧 artifact，不能再作为预填反馈。
 */
function reviewIsSupersededByRevision(state: WorkspaceWsState, reviewNodeId?: string) {
  if (!reviewNodeId) {
    return false;
  }
  const timelineNodes = state.timelineNodes ?? [];
  const reviewNodeIndex = timelineNodes.findIndex((node) => node.node_id === reviewNodeId);
  if (reviewNodeIndex < 0) {
    return false;
  }
  return timelineNodes
    .slice(reviewNodeIndex + 1)
    .some((node) => node.node_type === "revision" && node.status === "completed");
}

export function selectPrepareContextNotes(state: WorkspaceWsState) {
  return state.timelineNodes
    .filter((node) => node.node_type === "context_note")
    .map((node) => {
      const detailContent = state.nodeDetails[node.node_id]?.streaming_content;
      return detailContent && detailContent.trim().length > 0
        ? detailContent
        : node.summary ?? "";
    })
    .filter((content) => content.trim().length > 0);
}

