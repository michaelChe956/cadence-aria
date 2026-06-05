import {
  ArrowLeft,
  Settings,
  TriangleAlert,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ComponentProps } from "react";
import type { RevisionPath, WorkspaceProviderName } from "../api/types";
import { ArtifactPane } from "../components/chat-workspace/ArtifactPane";
import { ChatEntryList, type ChatEntryListHandle } from "../components/chat-workspace/ChatEntryList";
import { ChatInputBar } from "../components/chat-workspace/ChatInputBar";
import { TimelineNodeList } from "../components/chat-workspace/TimelineNodeList";
import {
  DisconnectBanner,
  loadAcknowledgedAbortedNodes,
} from "../components/workspace/DisconnectBanner";
import { ProviderConfigPanel } from "../components/workspace/ProviderConfigPanel";
import { WorkspaceHeader } from "../components/workspace/WorkspaceHeader";
import { useStageUI } from "../hooks/useStageUI";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import type { ChatEntry, ChoiceResponsePayload } from "../state/chat-entries";
import {
  useWorkspaceStore,
  type ProviderConfigSnapshot,
  type TimelineNode,
  type WorkspaceWsState,
} from "../state/workspace-ws-store";

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
  const store = useWorkspaceStore();
  const stageConfig = useStageUI(store.stage);
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const [activePanel, setActivePanel] = useState<"chat" | "artifact">("chat");
  const sessionReady = store.sessionId === sessionId;
  const inputDisabled = !sessionReady || connectionStatus !== "connected";
  const selectedEntryId = useMemo(
    () =>
      store.selectedNodeId
        ? scrollTargetEntryIdForNode(store.chatEntries, store.selectedNodeId)
        : null,
    [store.chatEntries, store.selectedNodeId],
  );
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

  useEffect(() => {
    if (selectedEntryId) {
      chatListRef.current?.scrollToEntry(selectedEntryId);
    }
  }, [selectedEntryId]);

  useUnloadGuard({
    enabled: UNLOAD_GUARDED_STAGES.has(store.stage),
    message: UNLOAD_GUARD_MESSAGE,
  });

  function handleStartGeneration() {
    sendStartGeneration(
      providerConfigFor(store.providers, store.reviewerEnabled, store.reviewRounds),
      store.reviewerEnabled,
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

  function handleHumanConfirm(decision: "confirm" | "request-change" | "terminate") {
    sendHumanConfirm(decision);
  }

  function handleAuthorDecision(decision: "accept" | "reject") {
    sendAuthorDecision(decision);
  }

  const providerPanel = (
    <ProviderConfigDialogButton
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
        <div className="min-w-0 flex-1 truncate text-center text-sm font-semibold text-[var(--aria-ink)]">
          {entityTypeLabel(store.workspaceType)} #{store.sessionId ?? sessionId}
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
        entityType={entityTypeLabel(store.workspaceType)}
        entityId={store.sessionId ?? sessionId}
        author={store.providers?.author ?? "claude_code"}
        reviewer={store.providers?.reviewer ?? null}
        rounds={store.reviewRounds}
        stage={store.stage}
        providerLocked={store.providerLocked}
        lockedAt={store.providerLockedAt}
        superpowers={store.superpowersEnabled}
        openSpec={store.openSpecEnabled}
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

      <main className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[16rem_minmax(0,1fr)]">
        <TimelineNodeList
          nodes={store.timelineNodes}
          activeNodeId={store.activeNodeId}
          selectedNodeId={store.selectedNodeId}
          onSelectNode={handleSelectNode}
          className="border-b border-[var(--aria-line)] md:border-b-0 md:border-r"
        />
        <section className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] bg-[var(--aria-panel)]">
          <WorkspacePanelTabs
            activePanel={activePanel}
            onSelectPanel={setActivePanel}
            artifactCount={store.artifactVersions.length}
          />
          {activePanel === "artifact" ? (
            <ArtifactPane
              artifactVersions={store.artifactVersions}
              artifact={store.artifact}
              className="min-h-0 border-l-0"
            />
          ) : (
            <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_auto]">
              <ChatEntryList
                ref={chatListRef}
                entries={store.chatEntries}
                onPermissionResponse={handlePermissionResponse}
                onChoiceResponse={handleChoiceResponse}
                onSelectRevisionPath={
                  store.stage === "review_decision" ? handleSelectRevisionPath : undefined
                }
                onHumanConfirm={handleHumanConfirm}
              />
              <ChatInputBar
                stage={store.stage}
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

      <StatusBar state={store} connectionStatus={connectionStatus} />
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
  state,
  connectionStatus,
}: {
  state: WorkspaceWsState;
  connectionStatus: string;
}) {
  const activeNode =
    state.timelineNodes.find((node) => node.node_id === state.activeNodeId) ??
    state.timelineNodes.at(-1) ??
    null;
  return (
    <footer
      data-testid="workspace-status-bar"
      className="flex h-8 shrink-0 items-center justify-between gap-3 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs text-[var(--aria-ink-muted)]"
    >
      <span>阶段 {state.stage}</span>
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
