import type { WorkspaceContentRef } from "./chat-entries";
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
  impact?: unknown;
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
        `   impact: ${asTrimmedString(finding.impact)}`,
        `   required_action: ${asTrimmedString(finding.required_action)}`,
      ].join("\n");
    })
    .join("\n");
}

export function selectLatestReviewReport(state: WorkspaceWsState): string | undefined {
  const entry = state.chatEntries
    .filter((candidate) => candidate.type === "review_verdict")
    .at(-1);
  const content = typeof entry?.content === "string" ? entry.content.trim() : "";
  if (!entry) {
    return undefined;
  }
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

