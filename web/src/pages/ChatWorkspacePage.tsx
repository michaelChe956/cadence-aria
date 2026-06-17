import {
  ArrowLeft,
  Settings,
  TriangleAlert,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentProps } from "react";
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceEventOutput,
  fetchWorkspaceNodeDetail,
  fetchWorkspacePrompt,
} from "../api/workspace-content";
import type { RevisionPath, WorkspaceProviderName } from "../api/types";
import { ArtifactPane } from "../components/chat-workspace/ArtifactPane";
import { ChatEntryList, type ChatEntryListHandle } from "../components/chat-workspace/ChatEntryList";
import { ChatInputBar } from "../components/chat-workspace/ChatInputBar";
import { ReviewDecisionActions } from "../components/chat-workspace/ReviewDecisionActions";
import { TimelineNodeList } from "../components/chat-workspace/TimelineNodeList";
import {
  DisconnectBanner,
  loadAcknowledgedAbortedNodes,
} from "../components/workspace/DisconnectBanner";
import { ProviderConfigPanel } from "../components/workspace/ProviderConfigPanel";
import { WorkItemPlanCandidatePanel } from "../components/workspace/WorkItemPlanCandidatePanel";
import { WorkspaceHeader } from "../components/workspace/WorkspaceHeader";
import { useStageUI } from "../hooks/useStageUI";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import type { ChatEntry, ChoiceResponsePayload, WorkspaceContentRef } from "../state/chat-entries";
import {
  useWorkspaceStore,
  type ProviderConfigSnapshot,
  type TimelineNode,
} from "../state/workspace-ws-store";
import { workspaceContentCacheValues } from "../state/workspace-content-cache";

const UNLOAD_GUARDED_STAGES = new Set(["running", "cross_review", "revision"]);
const UNLOAD_GUARD_MESSAGE = "运行中。刷新/关闭将中止当前 Provider 运行，是否继续？";

export function ChatWorkspacePage({
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
    sendAuthorDecision,
    sendRequestRevision,
    sendRevertWorkItem,
    sendHumanConfirm,
    abort,
    selectProvider,
    respondPermission,
    sendChoiceResponse,
    connectionStatus,
    isReconnecting,
    reconnectAttemptCount,
    retryNow,
  } = useWorkspaceWs(sessionId);
  const storeSessionId = useWorkspaceStore((state) => state.sessionId);
  const workspaceType = useWorkspaceStore((state) => state.workspaceType);
  const stage = useWorkspaceStore((state) => state.stage);
  const providers = useWorkspaceStore((state) => state.providers);
  const reviewRounds = useWorkspaceStore((state) => state.reviewRounds);
  const providerLocked = useWorkspaceStore((state) => state.providerLocked);
  const providerLockedAt = useWorkspaceStore((state) => state.providerLockedAt);
  const superpowersEnabled = useWorkspaceStore((state) => state.superpowersEnabled);
  const openSpecEnabled = useWorkspaceStore((state) => state.openSpecEnabled);
  const chatEntries = useWorkspaceStore((state) => state.chatEntries);
  const contentCache = useWorkspaceStore((state) => state.contentCache);
  const selectedNodeId = useWorkspaceStore((state) => state.selectedNodeId);
  const timelineNodes = useWorkspaceStore((state) => state.timelineNodes);
  const activeNodeId = useWorkspaceStore((state) => state.activeNodeId);
  const artifactVersions = useWorkspaceStore((state) => state.artifactVersions);
  const artifactContentCache = useWorkspaceStore((state) => state.artifactContentCache);
  const artifact = useWorkspaceStore((state) => state.artifact);
  const workItemPlanCandidate = useWorkspaceStore((state) => state.workItemPlanCandidate);
  const protocolError = useWorkspaceStore((state) => state.protocolError);
  const acknowledgedAbortedNodes = useWorkspaceStore((state) => state.acknowledgedAbortedNodes);
  const reviewerEnabled = useWorkspaceStore((state) => state.reviewerEnabled);
  const stageConfig = useStageUI(stage);
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const hydratedNodeIdsRef = useRef<Set<string>>(new Set());
  const [activePanel, setActivePanel] = useState<"chat" | "artifact">("chat");
  const sessionReady = storeSessionId === sessionId;
  const inputDisabled = !sessionReady || connectionStatus !== "connected";
  const selectedEntryId = useMemo(
    () =>
      selectedNodeId
        ? scrollTargetEntryIdForNode(chatEntries, selectedNodeId)
        : null,
    [chatEntries, selectedNodeId],
  );
  const contentCacheValues = useMemo(
    () => workspaceContentCacheValues(contentCache),
    [contentCache],
  );
  const artifactContentCacheValues = useMemo(
    () => numericContentCacheValues(artifactContentCache),
    [artifactContentCache],
  );
  const abortedByDisconnectNode = latestUnacknowledgedAbortedNode(
    timelineNodes,
    acknowledgedAbortedNodes,
  );

  useEffect(() => {
    const acknowledgedNodes = loadAcknowledgedAbortedNodes();
    if (acknowledgedNodes.length > 0) {
      useWorkspaceStore.getState().setAcknowledgedAbortedNodes(acknowledgedNodes);
    }
  }, []);

  useEffect(() => {
    if (selectedEntryId) {
      chatListRef.current?.scrollToEntry(selectedEntryId);
    }
  }, [selectedEntryId]);

  useEffect(() => {
    hydratedNodeIdsRef.current.clear();
  }, [sessionId]);

  useEffect(() => {
    if (!sessionReady) {
      return;
    }
    const nodeIds = [selectedNodeId, activeNodeId].filter(
      (nodeId): nodeId is string => typeof nodeId === "string" && nodeId.length > 0,
    );
    for (const nodeId of nodeIds) {
      if (hydratedNodeIdsRef.current.has(nodeId)) {
        continue;
      }
      if (!useWorkspaceStore.getState().nodeDetails[nodeId]) {
        continue;
      }
      hydratedNodeIdsRef.current.add(nodeId);
      fetchWorkspaceNodeDetail(sessionId, nodeId)
        .then((detail) => {
          const state = useWorkspaceStore.getState();
          if (state.sessionId !== sessionId) {
            return;
          }
          state.setNodeDetail(detail);
        })
        .catch(() => {
          hydratedNodeIdsRef.current.delete(nodeId);
        });
    }
  }, [activeNodeId, selectedNodeId, sessionId, sessionReady]);

  useUnloadGuard({
    enabled: UNLOAD_GUARDED_STAGES.has(stage),
    message: UNLOAD_GUARD_MESSAGE,
  });

  function handleStartGeneration() {
    const { providers, reviewerEnabled, reviewRounds } = useWorkspaceStore.getState();
    sendStartGeneration(
      providerConfigFor(providers, reviewerEnabled, reviewRounds),
      reviewerEnabled,
    );
  }

  function handlePermissionResponse(entry: ChatEntry, approved: boolean) {
    const requestId = requestIdFromEntry(entry);
    if (!requestId) {
      return;
    }
    respondPermission(requestId, approved, undefined);
  }

  function handleChoiceResponse(entry: ChatEntry, response: ChoiceResponsePayload) {
    const requestId = requestIdFromEntry(entry);
    if (!requestId) {
      return;
    }
    sendChoiceResponse(requestId, response.selected_option_ids, response.free_text);
  }

  function handleSelectNode(nodeId: string) {
    useWorkspaceStore.getState().setSelectedNode(nodeId);
  }

  function handleSelectRevisionPath(path: RevisionPath, extraContext?: string) {
    sendSelectRevisionPath(path, extraContext);
  }

  function handleHumanConfirm(
    decision: "confirm" | "request-change" | "terminate",
    payload?: unknown,
  ) {
    if (payload === undefined) {
      sendHumanConfirm(decision);
      return;
    }
    sendHumanConfirm(decision, payload);
  }

  function handleAuthorDecision(decision: "accept" | "reject") {
    sendAuthorDecision(decision);
  }

  const handleLoadContent = useCallback(async (currentSessionId: string, ref: WorkspaceContentRef) => {
    if (ref.kind === "execution_output") {
      const response = await fetchWorkspaceEventOutput(currentSessionId, ref.nodeId, ref.eventId);
      return response.output;
    }
    if (ref.kind === "provider_prompt") {
      const response = await fetchWorkspacePrompt(currentSessionId, ref.nodeId);
      return response.prompt;
    }
    throw new Error("不支持加载该内容类型");
  }, []);

  const handleCacheContent = useCallback((key: string, value: string) => {
    const state = useWorkspaceStore.getState();
    if (state.sessionId !== sessionId) {
      return;
    }
    state.setContentCacheEntry(key, value);
  }, [sessionId]);

  const handleLoadArtifactVersion = useCallback(async (version: number) => {
    const response = await fetchWorkspaceArtifactVersion(sessionId, version);
    return response.markdown;
  }, [sessionId]);

  const handleCacheArtifactContent = useCallback((version: number, value: string) => {
    const state = useWorkspaceStore.getState();
    if (state.sessionId !== sessionId) {
      return;
    }
    state.setArtifactContentCacheEntry(version, value);
  }, [sessionId]);

  const providerPanel = (
    <ProviderConfigDialogButton
      providers={providers}
      editable={stageConfig.providerEditable}
      onSelectProvider={(role, provider) => selectProvider(role, provider)}
      reviewerEnabled={reviewerEnabled}
      onToggleReviewer={(enabled) => useWorkspaceStore.setState({ reviewerEnabled: enabled })}
      rounds={reviewRounds}
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
        <div className="min-w-0 flex-1 truncate text-center text-sm font-semibold text-[var(--aria-ink)]">
          {entityTypeLabel(workspaceType)} #{storeSessionId ?? sessionId}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {providerPanel}
          {connectionStatus === "connected" ? (
            <Wifi aria-label="已连接" className="h-4 w-4 text-emerald-600" />
          ) : (
            <WifiOff aria-label="未连接" className="h-4 w-4 text-red-600" />
          )}
        </div>
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
        entityType={entityTypeLabel(workspaceType)}
        entityId={storeSessionId ?? sessionId}
        author={providers?.author ?? "claude_code"}
        reviewer={providers?.reviewer ?? null}
        rounds={reviewRounds}
        stage={stage}
        providerLocked={providerLocked}
        lockedAt={providerLockedAt}
        superpowers={superpowersEnabled}
        openSpec={openSpecEnabled}
      />

      {protocolError ? (
        <div
          role="alert"
          data-testid="protocol-error-alert"
          className="flex min-h-10 min-w-0 items-start gap-2 border-b border-red-200 bg-red-50 px-4 py-2 text-sm text-red-800"
        >
          <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="min-w-0 break-words">
            <span className="font-mono text-xs font-semibold">{protocolError.code}</span>
            <span className="mx-2 text-red-300">/</span>
            <span>{protocolError.message}</span>
          </div>
        </div>
      ) : null}

      <main className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[16rem_minmax(0,1fr)]">
        <TimelineNodeList
          nodes={timelineNodes}
          activeNodeId={activeNodeId}
          selectedNodeId={selectedNodeId}
          onSelectNode={handleSelectNode}
          className="border-b border-[var(--aria-line)] md:border-b-0 md:border-r"
        />
        <section className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] bg-[var(--aria-panel)]">
          <WorkspacePanelTabs
            activePanel={activePanel}
            onSelectPanel={setActivePanel}
            artifactCount={artifactVersions.length}
          />
          {activePanel === "artifact" ? (
            workspaceType === "work_item_plan" ? (
              workItemPlanCandidate ? (
                <WorkItemPlanCandidatePanel
                  candidate={workItemPlanCandidate}
                  stage={stage}
                  onRevert={sendRevertWorkItem}
                  onRequestRevision={sendRequestRevision}
                  onAccept={() => sendAuthorDecision("accept")}
                  className="min-h-0"
                />
              ) : (
                <div className="flex min-h-0 flex-col items-center justify-center p-6 text-sm text-[var(--aria-ink-muted)]">
                  <p>尚未生成候选，请点击开始生成</p>
                </div>
              )
            ) : (
              <ArtifactPane
                artifactVersions={artifactVersions}
                artifact={artifact}
                sessionId={sessionReady ? sessionId : null}
                artifactContentCache={artifactContentCacheValues}
                loadArtifactVersion={handleLoadArtifactVersion}
                onCacheArtifactContent={handleCacheArtifactContent}
                className="min-h-0 border-l-0"
              />
            )
          ) : (
            <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_auto_auto]">
              <ChatEntryList
                ref={chatListRef}
                entries={chatEntries}
                onPermissionResponse={handlePermissionResponse}
                onChoiceResponse={handleChoiceResponse}
                onHumanConfirm={handleHumanConfirm}
                sessionId={sessionReady ? sessionId : null}
                contentCache={contentCacheValues}
                loadContent={handleLoadContent}
                onCacheContent={handleCacheContent}
              />
              {stage === "review_decision" ? (
                <ReviewDecisionActionBar onSelectRevisionPath={handleSelectRevisionPath} />
              ) : null}
              <ChatInputBar
                stage={stage}
                disabled={inputDisabled}
                onSendContextNote={sendContextNote}
                onStartGeneration={handleStartGeneration}
                onSendHumanDecision={(content) => sendHumanConfirm("request-change", content)}
                onAuthorDecision={handleAuthorDecision}
                onAbort={abort}
              />
            </div>
          )}
        </section>
      </main>

      <StatusBar
        stage={stage}
        timelineNodes={timelineNodes}
        activeNodeId={activeNodeId}
        connectionStatus={connectionStatus}
      />
    </div>
  );
}

function ReviewDecisionActionBar({
  onSelectRevisionPath,
}: {
  onSelectRevisionPath: (path: RevisionPath, extraContext?: string) => void;
}) {
  return (
    <div className="border-t border-amber-200 bg-amber-50/80 px-3 py-2">
      <ReviewDecisionActions onSelectPath={onSelectRevisionPath} />
    </div>
  );
}

function WorkspacePanelTabs({
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

function numericContentCacheValues(
  cache: ReturnType<typeof useWorkspaceStore.getState>["artifactContentCache"],
) {
  return Object.fromEntries(
    Object.entries(workspaceContentCacheValues(cache)).map(([version, markdown]) => [
      Number(version),
      markdown,
    ]),
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

type ProviderConfigDialogButtonProps = ComponentProps<typeof ProviderConfigPanel>;

function ProviderConfigDialogButton(props: ProviderConfigDialogButtonProps) {
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

function StatusBar({
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

function requestIdFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata;
  const requestId = metadata?.request_id;
  return typeof requestId === "string" ? requestId : null;
}

function latestUnacknowledgedAbortedNode(nodes: TimelineNode[], acknowledgedNodeIds: string[]) {
  const acknowledged = new Set(acknowledgedNodeIds);
  const latest = nodes.at(-1);
  if (latest?.node_type !== "aborted_by_disconnect") {
    return null;
  }
  return acknowledged.has(latest.node_id) ? null : latest;
}

function scrollTargetEntryIdForNode(entries: ChatEntry[], nodeId: string) {
  const nodeEntries = entries.filter((entry) => entry.node_id === nodeId);
  return (
    nodeEntries.find((entry) => entry.type === "provider_stream")?.id ??
    nodeEntries[0]?.id ??
    null
  );
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

function entityTypeLabel(workspaceType: string | null) {
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
  const endedAt = node.completed_at ? Date.parse(node.completed_at) : Date.now();
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
