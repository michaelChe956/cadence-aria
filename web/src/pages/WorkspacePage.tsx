import { ArrowLeft, Check, TriangleAlert, Wifi, WifiOff, X } from "lucide-react";
import { useEffect, type ReactNode } from "react";
import type { ArtifactVersion, ProviderConfigSnapshot, WorkspaceProviderName } from "../api/types";
import {
  DisconnectBanner,
  loadAcknowledgedAbortedNodes,
} from "../components/workspace/DisconnectBanner";
import { NodeDetailPanel } from "../components/workspace/NodeDetailPanel";
import { PrepareContextPanel } from "../components/workspace/PrepareContextPanel";
import { ProviderConfigPanel } from "../components/workspace/ProviderConfigPanel";
import { StageActionsBar } from "../components/workspace/StageActionsBar";
import { WorkspaceHeader } from "../components/workspace/WorkspaceHeader";
import { HumanConfirmStagePanel } from "../components/workspace/stages/HumanConfirmStagePanel";
import { ReviewDecisionStagePanel } from "../components/workspace/stages/ReviewDecisionStagePanel";
import { useStageUI } from "../hooks/useStageUI";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import {
  selectPrepareContextNotes,
  useWorkspaceStore,
  type PermissionRequest,
  type TimelineNode,
  type TimelineNodeDetail,
  type WorkspaceWsState,
} from "../state/workspace-ws-store";

const STAGE_EMPTY_TEXT: Record<string, string> = {
  running: "运行中，节点输出会进入 Timeline",
  cross_review: "审核中，等待 reviewer verdict",
  revision: "修订中，等待 author 输出",
  completed: "已完成",
};
const UNLOAD_GUARDED_STAGES = new Set(["running", "cross_review", "revision"]);
const UNLOAD_GUARD_MESSAGE = "运行中。刷新/关闭将中止当前 Provider 运行，是否继续？";

export function WorkspacePage({
  sessionId,
  onBack,
}: {
  sessionId: string;
  onBack: () => void;
}) {
  const {
    sendContextNote,
    sendStartGeneration,
    sendSelectRevisionPath,
    sendHumanConfirm,
    abort,
    selectProvider,
    respondPermission,
    connectionStatus,
    isReconnecting,
    reconnectAttemptCount,
    retryNow,
  } = useWorkspaceWs(sessionId);
  const store = useWorkspaceStore();
  const stageConfig = useStageUI(store.stage);
  const contextNotes = selectPrepareContextNotes(store);
  const selectedNode = selectTimelineNode(store.timelineNodes, store.selectedNodeId, store.activeNodeId);
  const selectedNodeDetail = selectedNode ? store.nodeDetails[selectedNode.node_id] ?? null : null;
  const latestArtifact = latestArtifactVersion(store.artifactVersions, store.artifact);
  const previousArtifact = previousArtifactVersion(store.artifactVersions);
  const abortedByDisconnectNode = latestUnacknowledgedAbortedNode(
    store.timelineNodes,
    store.acknowledgedAbortedNodes,
  );

  useEffect(() => {
    const acknowledgedNodes = loadAcknowledgedAbortedNodes();
    if (acknowledgedNodes.length > 0) {
      useWorkspaceStore.getState().setAcknowledgedAbortedNodes(acknowledgedNodes);
    }
  }, []);

  useUnloadGuard({
    enabled: UNLOAD_GUARDED_STAGES.has(store.stage),
    message: UNLOAD_GUARD_MESSAGE,
  });
  const sessionReady = store.sessionId === sessionId;

  function handleStartGeneration() {
    sendStartGeneration(
      providerConfigFor(store.providers, store.reviewerEnabled, store.reviewRounds),
      store.reviewerEnabled,
    );
  }

  const providerPanel = (
    <ProviderConfigPanel
      providers={store.providers}
      editable={stageConfig.providerEditable}
      onSelectProvider={(role, provider) => selectProvider(role, provider)}
      reviewerEnabled={store.reviewerEnabled}
      onToggleReviewer={(enabled) => useWorkspaceStore.setState({ reviewerEnabled: enabled })}
      rounds={store.reviewRounds}
      onChangeRounds={(rounds) =>
        useWorkspaceStore.setState({ reviewRounds: clampReviewRounds(rounds) })
      }
    />
  );

  return (
    <div className="flex h-screen min-w-0 flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]">
      <div className="flex h-11 min-w-0 shrink-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3">
        <button
          type="button"
          onClick={onBack}
          className="inline-flex h-8 shrink-0 items-center gap-2 rounded-md px-2 text-sm text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
        >
          <ArrowLeft className="h-4 w-4" />
          返回
        </button>
        {connectionStatus === "connected" ? (
          <Wifi className="h-4 w-4 text-emerald-600" />
        ) : (
          <WifiOff className="h-4 w-4 text-red-600" />
        )}
      </div>

      <DisconnectBanner
        isReconnecting={isReconnecting}
        attemptCount={reconnectAttemptCount}
        onManualReconnect={retryNow}
        abortedByDisconnect={
          abortedByDisconnectNode
            ? {
                nodeId: abortedByDisconnectNode.node_id,
                ts: abortedByDisconnectNode.completed_at ?? abortedByDisconnectNode.started_at,
              }
            : null
        }
        onAcknowledge={(nodeIds) =>
          useWorkspaceStore.getState().setAcknowledgedAbortedNodes(nodeIds)
        }
        onViewTimeline={
          abortedByDisconnectNode
            ? () => useWorkspaceStore.getState().setSelectedNode(abortedByDisconnectNode.node_id)
            : undefined
        }
      />

      <WorkspaceHeader
        entityType={entityTypeLabel(store.workspaceType)}
        entityId={store.sessionId ?? sessionId}
        author={store.providers?.author ?? "claude_code"}
        reviewer={store.providers?.reviewer ?? null}
        rounds={store.reviewRounds}
        stage={store.stage}
        providerLocked={store.providerLocked}
        lockedAt={store.providerLockedAt}
      />

      {store.protocolError ? (
        <div
          role="alert"
          data-testid="protocol-error-alert"
          className="flex min-h-10 min-w-0 items-start gap-2 border-b border-red-200 bg-red-50 px-4 py-2 text-sm text-red-800"
        >
          <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="min-w-0 break-words">
            <span className="font-mono text-xs font-semibold">{store.protocolError.code}</span>
            <span className="mx-2 text-red-300">/</span>
            <span>{store.protocolError.message}</span>
          </div>
        </div>
      ) : null}

      <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[minmax(18rem,0.9fr)_minmax(0,1.1fr)]">
        <section className="min-h-0 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] lg:border-b-0 lg:border-r">
          <div className="h-full min-h-0 overflow-auto p-3">
            {store.timelineNodes.length > 0 ? (
              <div className="space-y-2">
                {store.timelineNodes.map((node) => (
                  <TimelineNodeButton
                    key={node.node_id}
                    node={node}
                    selected={node.node_id === selectedNode?.node_id}
                    onSelect={() => useWorkspaceStore.getState().setSelectedNode(node.node_id)}
                  />
                ))}
              </div>
            ) : (
              <div className="rounded-md border border-[var(--aria-line)] bg-white p-3 text-sm text-[var(--aria-ink-muted)]">
                暂无 Timeline 节点
              </div>
            )}
          </div>
        </section>

        <section className="grid min-h-0 grid-rows-[minmax(0,1fr)_minmax(15rem,0.85fr)] bg-[var(--aria-panel)]">
          <div className="min-h-0 overflow-auto border-b border-[var(--aria-line)] p-3">
            {renderStagePanel({
              panel: stageConfig.panel,
              providerPanel,
              contextNotes,
              sendContextNote,
              handleStartGeneration,
              sessionReady,
              selectedNodeDetail,
              store,
              latestArtifact,
              previousArtifact,
              sendSelectRevisionPath,
              sendHumanConfirm,
              respondPermission,
            })}
          </div>

          <div className="min-h-0 p-3">
            {selectedNode ? (
              <NodeDetailPanel
                node={selectedNode}
                detail={selectedNodeDetail}
                artifactVersions={store.artifactVersions}
              />
            ) : (
              <div className="h-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-sm text-[var(--aria-ink-muted)]">
                选择 Timeline 节点查看详情
              </div>
            )}
          </div>
        </section>
      </div>

      <StageActionsBar
        stage={store.stage}
        disabled={!sessionReady}
        onStartGeneration={handleStartGeneration}
        onAbort={abort}
        onConfirm={() => sendHumanConfirm("confirm")}
        onRequestChange={() => sendHumanConfirm("request-change")}
        onTerminate={() => sendHumanConfirm("terminate")}
        onSelectRevisionPath={(path) => sendSelectRevisionPath(path, undefined)}
      />
    </div>
  );
}

function renderStagePanel({
  panel,
  providerPanel,
  contextNotes,
  sendContextNote,
  handleStartGeneration,
  sessionReady,
  selectedNodeDetail,
  store,
  latestArtifact,
  previousArtifact,
  sendSelectRevisionPath,
  sendHumanConfirm,
  respondPermission,
}: {
  panel: string;
  providerPanel: ReactNode;
  contextNotes: string[];
  sendContextNote: (content: string) => void;
  handleStartGeneration: () => void;
  sessionReady: boolean;
  selectedNodeDetail: TimelineNodeDetail | null;
  store: WorkspaceWsState;
  latestArtifact: ArtifactVersion;
  previousArtifact: ArtifactVersion | null;
  sendSelectRevisionPath: (path: "revise" | "revise-with-context" | "skip-to-human", ctx?: string) => void;
  sendHumanConfirm: (decision: "confirm" | "request-change" | "terminate", payload?: unknown) => void;
  respondPermission: (id: string, approved: boolean, reason?: string) => void;
}) {
  if (panel === "PrepareContextPanel") {
    return (
      <div className="space-y-4">
        {providerPanel}
        <PrepareContextPanel
          onSendContextNote={sendContextNote}
          onStartGeneration={handleStartGeneration}
          contextNotes={contextNotes}
          disabled={!sessionReady}
        />
        <PendingPermissionCards permissions={store.pendingPermissions} onRespond={respondPermission} />
      </div>
    );
  }

  if (panel === "ReviewDecisionPanel") {
    return (
      <div className="space-y-3">
        {providerPanel}
        <ReviewDecisionStagePanel
          reviewer={store.providers?.reviewer ?? "codex"}
          verdict={store.pendingReviewDecision?.verdict ?? selectedNodeDetail?.verdict?.verdict ?? "revise"}
          summary={store.pendingReviewDecision?.summary ?? selectedNodeDetail?.verdict?.summary ?? ""}
          onSelectPath={(path, ctx) => sendSelectRevisionPath(path, ctx)}
        />
        <PendingPermissionCards permissions={store.pendingPermissions} onRespond={respondPermission} />
      </div>
    );
  }

  if (panel === "HumanConfirmPanel") {
    return (
      <div className="space-y-3">
        {providerPanel}
        <HumanConfirmStagePanel
          artifactVersion={latestArtifact}
          prevVersion={previousArtifact}
          reviewerSummary={
            store.pendingReviewerSummary ?? {
              verdict: selectedNodeDetail?.verdict?.verdict ?? "pass",
              points: [selectedNodeDetail?.verdict?.summary ?? ""].filter(
                (point) => point.trim().length > 0,
              ),
            }
          }
          onConfirm={() => sendHumanConfirm("confirm")}
          onRequestChange={(feedback) => sendHumanConfirm("request-change", feedback)}
          onTerminate={() => sendHumanConfirm("terminate")}
        />
        <PendingPermissionCards permissions={store.pendingPermissions} onRespond={respondPermission} />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {providerPanel}
      <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-4 text-sm text-[var(--aria-ink-muted)]">
        {STAGE_EMPTY_TEXT[store.stage] ?? store.stage}
      </div>
      <PendingPermissionCards permissions={store.pendingPermissions} onRespond={respondPermission} />
    </div>
  );
}

function PendingPermissionCards({
  permissions,
  onRespond,
}: {
  permissions: PermissionRequest[];
  onRespond: (id: string, approved: boolean, reason?: string) => void;
}) {
  if (permissions.length === 0) {
    return null;
  }

  return (
    <div className="space-y-2">
      {permissions.map((permission) => (
        <div
          key={permission.id}
          className="rounded-md border border-amber-300 bg-amber-50 p-3 text-sm"
        >
          <div className="font-semibold text-amber-900">{permission.tool_name}</div>
          <div className="mt-1 text-amber-800">{permission.description}</div>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <span className="rounded border border-amber-300 px-2 py-0.5 text-xs text-amber-800">
              {permission.risk_level}
            </span>
            <button
              type="button"
              onClick={() => onRespond(permission.id, false, undefined)}
              className="inline-flex h-7 items-center gap-1 rounded-md border border-red-300 bg-red-50 px-2 text-xs font-semibold text-red-700"
            >
              <X className="h-3.5 w-3.5" />
              拒绝
            </button>
            <button
              type="button"
              onClick={() => onRespond(permission.id, true, undefined)}
              className="inline-flex h-7 items-center gap-1 rounded-md border border-green-500 bg-green-50 px-2 text-xs font-semibold text-green-700"
            >
              <Check className="h-3.5 w-3.5" />
              允许
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}

function TimelineNodeButton({
  node,
  selected,
  onSelect,
}: {
  node: TimelineNode;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      data-testid={`timeline-node-${node.node_type}`}
      onClick={onSelect}
      className={
        selected
          ? "block w-full rounded-md border border-[var(--aria-primary)] bg-white px-3 py-2 text-left ring-1 ring-[var(--aria-primary)]"
          : "block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-left hover:border-[var(--aria-primary)]"
      }
    >
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className="truncate text-sm font-semibold text-[var(--aria-ink)]">{node.title}</span>
        <span className="shrink-0 rounded bg-[var(--aria-panel-muted)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--aria-ink-muted)]">
          {node.status}
        </span>
      </div>
      {node.summary ? (
        <p className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">{node.summary}</p>
      ) : null}
    </button>
  );
}

function selectTimelineNode(
  nodes: TimelineNode[],
  selectedNodeId: string | null,
  activeNodeId: string | null,
) {
  return (
    nodes.find((node) => node.node_id === selectedNodeId) ??
    nodes.find((node) => node.node_id === activeNodeId) ??
    nodes.at(-1) ??
    null
  );
}

function latestUnacknowledgedAbortedNode(nodes: TimelineNode[], acknowledgedNodeIds: string[]) {
  const acknowledged = new Set(acknowledgedNodeIds);
  const latest = nodes.at(-1);
  if (latest?.node_type !== "aborted_by_disconnect") {
    return null;
  }
  return acknowledged.has(latest.node_id) ? null : latest;
}

function providerConfigFor(
  providers: { author: string; reviewer?: string | null } | null,
  reviewerEnabled: boolean,
  reviewRounds: number,
): ProviderConfigSnapshot {
  const reviewer = reviewerEnabled ? providerNameFor(providers?.reviewer, "codex") : null;
  return {
    author: providerNameFor(providers?.author, "claude_code"),
    reviewer,
    review_rounds: reviewer ? clampReviewRounds(reviewRounds) : 0,
  };
}

function latestArtifactVersion(versions: ArtifactVersion[], artifact: string | null): ArtifactVersion {
  return (
    versions.at(-1) ?? {
      version: 0,
      markdown: artifact ?? "等待 Artifact",
      generated_by: "fake",
      created_at: "",
      source_node_id: "",
    }
  );
}

function previousArtifactVersion(versions: ArtifactVersion[]) {
  return versions.length > 1 ? versions[versions.length - 2] : null;
}

function entityTypeLabel(workspaceType: string | null) {
  if (workspaceType === "story") return "Story Spec";
  if (workspaceType === "design") return "Design Spec";
  if (workspaceType === "work_item") return "Work Item";
  return "Workspace";
}

function clampReviewRounds(value: number) {
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
