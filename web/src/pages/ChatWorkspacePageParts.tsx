import { Settings, X } from "lucide-react";
import { useState, type ComponentProps } from "react";
import type {
  RevisionPath,
  WorkItemPlanArtifactPayload,
  WorkspaceArtifactVersionResponse,
  WorkspaceProviderName,
} from "../api/types";
import { ReviewDecisionActions } from "../components/chat-workspace/ReviewDecisionActions";
import { ProviderConfigPanel } from "../components/workspace/ProviderConfigPanel";
import type { ChatEntry } from "../state/chat-entries";
import { workspaceContentCacheValues } from "../state/workspace-content-cache";
import {
  useWorkspaceStore,
  type ProviderConfigSnapshot,
  type TimelineNode,
} from "../state/workspace-ws-store";

export function ReviewDecisionActionBar({
  onSelectRevisionPath,
  onSelectDecision,
  options,
}: {
  onSelectRevisionPath: (path: RevisionPath, extraContext?: string) => void;
  onSelectDecision: (decision: string) => void;
  options?: string[];
}) {
  return (
    <div className="border-t border-amber-200 bg-amber-50/80 px-3 py-2">
      <ReviewDecisionActions
        options={options}
        onSelectDecision={onSelectDecision}
        onSelectPath={onSelectRevisionPath}
      />
    </div>
  );
}

export function optionalWorkItemPlanReviewDecisionOptions(
  workspaceType: string | null,
  entries: ChatEntry[],
): string[] | undefined {
  if (workspaceType !== "work_item_plan") {
    return undefined;
  }

  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index];
    if (entry.type !== "review_verdict") {
      continue;
    }
    if (isOptionalWorkItemPlanPassVerdict(entry.metadata)) {
      return ["apply_optional_findings", "skip_optional_findings"];
    }
    return undefined;
  }

  return undefined;
}

function isOptionalWorkItemPlanPassVerdict(
  metadata: Record<string, unknown> | undefined,
): boolean {
  if (!metadata) {
    return false;
  }
  if (
    metadata.verdict !== "pass" ||
    metadata.review_gate !== "user_confirm_allowed"
  ) {
    return false;
  }
  const findings = metadata.findings;
  return (
    Array.isArray(findings) &&
    findings.length > 0 &&
    findings.every(isOptionalReviewFinding)
  );
}

function isOptionalReviewFinding(finding: unknown): boolean {
  if (!finding || typeof finding !== "object") {
    return false;
  }
  const severity = (finding as { severity?: unknown }).severity;
  return (
    severity === "suggestion" || severity === "minor" || severity === "optional"
  );
}

export function normalizeWorkItemPlanArtifactResponse(
  artifact: WorkspaceArtifactVersionResponse["artifact"],
): WorkItemPlanArtifactPayload | null {
  if (!isRecord(artifact)) {
    return null;
  }
  const artifactRecord: Record<string, unknown> = artifact;
  const artifactType =
    typeof artifactRecord.type === "string" ? artifactRecord.type : null;
  if (
    artifactType &&
    "payload" in artifactRecord &&
    [
      "outline_candidate",
      "context_blocker",
      "draft_candidate",
      "batch_state",
      "compile_report",
    ].includes(artifactType)
  ) {
    return artifact as WorkItemPlanArtifactPayload;
  }
  if ("outline_candidate" in artifactRecord) {
    return {
      type: "outline_candidate",
      payload: artifactRecord.outline_candidate,
    } as WorkItemPlanArtifactPayload;
  }
  if ("context_blocker" in artifactRecord) {
    return {
      type: "context_blocker",
      payload: artifactRecord.context_blocker,
    } as WorkItemPlanArtifactPayload;
  }
  if ("draft_candidate" in artifactRecord) {
    return {
      type: "draft_candidate",
      payload: artifactRecord.draft_candidate,
    } as WorkItemPlanArtifactPayload;
  }
  if ("batch_state" in artifactRecord) {
    return {
      type: "batch_state",
      payload: artifactRecord.batch_state,
    } as WorkItemPlanArtifactPayload;
  }
  if ("compile_report" in artifactRecord) {
    return {
      type: "compile_report",
      payload: artifactRecord.compile_report,
    } as WorkItemPlanArtifactPayload;
  }
  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function WorkspacePanelTabs({
  activePanel,
  onSelectPanel,
  artifactCount,
}: {
  activePanel: "chat" | "artifact";
  onSelectPanel: (panel: "chat" | "artifact") => void;
  artifactCount: number;
}) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2">
      <div className="flex min-w-0 items-center gap-1">
        <button
          type="button"
          onClick={() => onSelectPanel("chat")}
          className={panelTabClass(activePanel === "chat")}
        >
          对话
        </button>
        <button
          type="button"
          onClick={() => onSelectPanel("artifact")}
          className={panelTabClass(activePanel === "artifact")}
        >
          Artifact
        </button>
      </div>
      <span className="shrink-0 rounded border border-[var(--aria-line)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
        artifacts {artifactCount}
      </span>
    </div>
  );
}

export function numericContentCacheValues(
  cache: ReturnType<typeof useWorkspaceStore.getState>["artifactContentCache"],
) {
  return Object.fromEntries(
    Object.entries(workspaceContentCacheValues(cache)).map(
      ([version, markdown]) => [Number(version), markdown],
    ),
  );
}

function panelTabClass(active: boolean) {
  return [
    "inline-flex h-8 items-center rounded-md px-3 text-xs font-semibold transition-colors",
    active
      ? "bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
      : "text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
  ].join(" ");
}

type ProviderConfigDialogButtonProps = ComponentProps<
  typeof ProviderConfigPanel
>;

export function ProviderConfigDialogButton(props: ProviderConfigDialogButtonProps) {
  const [open, setOpen] = useState(false);

  return (
    <div className="flex items-center justify-between gap-2">
      <button
        type="button"
        aria-label="Provider 配置"
        onClick={() => setOpen(true)}
        className="inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
      >
        <Settings className="h-4 w-4 text-[var(--aria-primary)]" />
        Provider 配置
      </button>

      {open ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
          <div
            role="dialog"
            aria-modal="true"
            aria-label="Provider 配置"
            className="w-full max-w-md rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl"
          >
            <div className="flex items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
              <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
                Provider 配置
              </h2>
              <button
                type="button"
                aria-label="关闭 Provider 配置"
                onClick={() => setOpen(false)}
                className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="p-4">
              <ProviderConfigPanel {...props} />
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function StatusBar({
  stage,
  timelineNodes,
  activeNodeId,
  connectionStatus,
}: {
  stage: string;
  timelineNodes: TimelineNode[];
  activeNodeId: string | null;
  connectionStatus: string;
}) {
  const activeNode =
    timelineNodes.find((node) => node.node_id === activeNodeId) ??
    timelineNodes.at(-1) ??
    null;
  return (
    <footer
      data-testid="workspace-status-bar"
      className="flex h-8 shrink-0 items-center justify-between gap-3 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs text-[var(--aria-ink-muted)]"
    >
      <span>阶段 {stage}</span>
      <span>连接 {connectionStatus}</span>
      <span>耗时 {activeNode ? elapsedText(activeNode) : "--"}</span>
    </footer>
  );
}

export function requestIdFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata;
  const requestId = metadata?.request_id;
  return typeof requestId === "string" ? requestId : null;
}

export function latestUnacknowledgedAbortedNode(
  nodes: TimelineNode[],
  acknowledgedNodeIds: string[],
) {
  const acknowledged = new Set(acknowledgedNodeIds);
  const latest = nodes.at(-1);
  if (latest?.node_type !== "aborted_by_disconnect") {
    return null;
  }
  return acknowledged.has(latest.node_id) ? null : latest;
}

export function scrollTargetEntryIdForNode(entries: ChatEntry[], nodeId: string) {
  const nodeEntries = entries.filter((entry) => entry.node_id === nodeId);
  return (
    nodeEntries.find((entry) => entry.type === "provider_stream")?.id ??
    nodeEntries[0]?.id ??
    null
  );
}

export function providerConfigFor(
  providers: { author: string; reviewer?: string | null } | null,
  reviewerEnabled: boolean,
  reviewRounds: number,
): ProviderConfigSnapshot {
  const reviewer = reviewerEnabled
    ? providerNameFor(providers?.reviewer, "codex")
    : null;
  return {
    author: providerNameFor(providers?.author, "claude_code"),
    reviewer,
    review_rounds: reviewer ? clampReviewRounds(reviewRounds) : 0,
  };
}

export function clampReviewRounds(value: number) {
  if (!Number.isFinite(value)) return 1;
  return Math.min(3, Math.max(1, Math.trunc(value)));
}

function providerNameFor(
  value: string | null | undefined,
  fallback: WorkspaceProviderName,
): WorkspaceProviderName {
  if (value === "claude_code" || value === "codex" || value === "fake") {
    return value;
  }
  return fallback;
}

export function entityTypeLabel(workspaceType: string | null) {
  if (workspaceType === "story") return "Story Spec";
  if (workspaceType === "design") return "Design Spec";
  if (workspaceType === "work_item") return "Work Item";
  if (workspaceType === "work_item_plan") return "Work Item Plan";
  return "Workspace";
}

function elapsedText(node: TimelineNode) {
  if (node.duration_ms !== null && node.duration_ms !== undefined) {
    return formatDuration(node.duration_ms);
  }
  const startedAt = Date.parse(node.started_at);
  if (Number.isNaN(startedAt)) {
    return "--";
  }
  const endedAt = node.completed_at
    ? Date.parse(node.completed_at)
    : Date.now();
  if (Number.isNaN(endedAt)) {
    return "--";
  }
  return formatDuration(Math.max(0, endedAt - startedAt));
}

function formatDuration(durationMs: number) {
  const totalSeconds = Math.floor(durationMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes === 0) {
    return `${seconds}s`;
  }
  return `${minutes}m${seconds.toString().padStart(2, "0")}s`;
}
