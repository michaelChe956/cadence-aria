import { ArrowLeft, TriangleAlert, Wifi, WifiOff } from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceEventOutput,
  fetchWorkspaceNodeDetail,
  fetchWorkspacePrompt,
} from "../api/workspace-content";
import type {
  RevisionPath,
} from "../api/types";
import { ArtifactPane } from "../components/chat-workspace/ArtifactPane";
import {
  ChatEntryList,
  type ChatEntryListHandle,
} from "../components/chat-workspace/ChatEntryList";
import { ChatInputBar } from "../components/chat-workspace/ChatInputBar";
import { TimelineNodeList } from "../components/chat-workspace/TimelineNodeList";
import {
  DisconnectBanner,
  loadAcknowledgedAbortedNodes,
} from "../components/workspace/DisconnectBanner";
import { WorkItemPlanArtifactPanel } from "../components/workspace/WorkItemPlanArtifactPanel";
import { WorkItemPlanCandidatePanel } from "../components/workspace/WorkItemPlanCandidatePanel";
import { WorkItemPlanStagedPanel } from "../components/workspace/WorkItemPlanStagedPanel";
import { WorkspaceHeader } from "../components/workspace/WorkspaceHeader";
import { useStageUI } from "../hooks/useStageUI";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import type {
  ChatEntry,
  ChoiceResponsePayload,
  WorkspaceContentRef,
} from "../state/chat-entries";
import {
  useWorkspaceStore,
} from "../state/workspace-ws-store";
import { workspaceContentCacheValues } from "../state/workspace-content-cache";
import {
  ProviderConfigDialogButton,
  ReviewDecisionActionBar,
  StatusBar,
  WorkspacePanelTabs,
  clampReviewRounds,
  entityTypeLabel,
  latestUnacknowledgedAbortedNode,
  normalizeWorkItemPlanArtifactResponse,
  numericContentCacheValues,
  optionalWorkItemPlanReviewDecisionOptions,
  providerConfigFor,
  requestIdFromEntry,
  scrollTargetEntryIdForNode,
} from "./ChatWorkspacePageParts";

const UNLOAD_GUARDED_STAGES = new Set(["running", "cross_review", "revision"]);
const UNLOAD_GUARD_MESSAGE =
  "运行中。刷新/关闭将中止当前 Provider 运行，是否继续？";

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
    sendSelectWorkItemGenerationMode,
    sendRequestOutlineRevision,
    sendWorkItemDraftDecision,
    sendWorkItemBatchDecision,
    sendWorkItemPlanCompileRecoveryAction,
    sendHumanConfirm,
    abort,
    selectProvider,
    respondPermission,
    sendChoiceResponse,
    sendReviewDecision,
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
  const superpowersEnabled = useWorkspaceStore(
    (state) => state.superpowersEnabled,
  );
  const openSpecEnabled = useWorkspaceStore((state) => state.openSpecEnabled);
  const chatEntries = useWorkspaceStore((state) => state.chatEntries);
  const pendingDecision = useWorkspaceStore((state) => state.pendingDecision);
  const contentCache = useWorkspaceStore((state) => state.contentCache);
  const selectedNodeId = useWorkspaceStore((state) => state.selectedNodeId);
  const timelineNodes = useWorkspaceStore((state) => state.timelineNodes);
  const activeNodeId = useWorkspaceStore((state) => state.activeNodeId);
  const artifactVersions = useWorkspaceStore((state) => state.artifactVersions);
  const artifactContentCache = useWorkspaceStore(
    (state) => state.artifactContentCache,
  );
  const artifact = useWorkspaceStore((state) => state.artifact);
  const workItemPlanCandidate = useWorkspaceStore(
    (state) => state.workItemPlanCandidate,
  );
  const workItemPlanArtifact = useWorkspaceStore(
    (state) => state.workItemPlanArtifact,
  );
  const workItemPlanArtifactVersions = useWorkspaceStore(
    (state) => state.workItemPlanArtifactVersions,
  );
  const protocolError = useWorkspaceStore((state) => state.protocolError);
  const acknowledgedAbortedNodes = useWorkspaceStore(
    (state) => state.acknowledgedAbortedNodes,
  );
  const reviewerEnabled = useWorkspaceStore((state) => state.reviewerEnabled);
  const stageConfig = useStageUI(stage);
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const hydratedNodeIdsRef = useRef<Set<string>>(new Set());
  const [activePanel, setActivePanel] = useState<"chat" | "artifact">("chat");
  const [
    selectedWorkItemPlanArtifactVersionNumber,
    setSelectedWorkItemPlanArtifactVersionNumber,
  ] = useState<number | null>(null);
  const sessionReady = storeSessionId === sessionId;
  const inputDisabled = !sessionReady || connectionStatus !== "connected";
  const reviewDecisionOptions = useMemo(
    () =>
      pendingDecision?.options ??
      optionalWorkItemPlanReviewDecisionOptions(workspaceType, chatEntries),
    [chatEntries, pendingDecision?.options, workspaceType],
  );
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
  const activeNode = useMemo(
    () => timelineNodes.find((node) => node.node_id === activeNodeId) ?? null,
    [activeNodeId, timelineNodes],
  );
  const selectedWorkItemPlanArtifactVersion = useMemo(
    () =>
      selectedNodeId
        ? workItemPlanArtifactVersions.find(
            (version) => version.source_node_id === selectedNodeId,
          )
        : undefined,
    [selectedNodeId, workItemPlanArtifactVersions],
  );
  const manuallySelectedWorkItemPlanArtifactVersion = useMemo(
    () =>
      selectedWorkItemPlanArtifactVersionNumber === null
        ? undefined
        : workItemPlanArtifactVersions.find(
            (version) =>
              version.version === selectedWorkItemPlanArtifactVersionNumber,
          ),
    [selectedWorkItemPlanArtifactVersionNumber, workItemPlanArtifactVersions],
  );
  const displayedWorkItemPlanArtifact =
    manuallySelectedWorkItemPlanArtifactVersion?.artifact ??
    selectedWorkItemPlanArtifactVersion?.artifact ??
    workItemPlanArtifact;
  const displayedWorkItemPlanArtifactVersion =
    manuallySelectedWorkItemPlanArtifactVersion ??
    selectedWorkItemPlanArtifactVersion;
  const showingHistoricalWorkItemPlanArtifact = Boolean(
    displayedWorkItemPlanArtifactVersion?.artifact &&
      (manuallySelectedWorkItemPlanArtifactVersion
        ? !manuallySelectedWorkItemPlanArtifactVersion.is_current
        : selectedNodeId !== activeNodeId),
  );
  const abortedByDisconnectNode = latestUnacknowledgedAbortedNode(
    timelineNodes,
    acknowledgedAbortedNodes,
  );

  useEffect(() => {
    const acknowledgedNodes = loadAcknowledgedAbortedNodes();
    if (acknowledgedNodes.length > 0) {
      useWorkspaceStore
        .getState()
        .setAcknowledgedAbortedNodes(acknowledgedNodes);
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
      (nodeId): nodeId is string =>
        typeof nodeId === "string" && nodeId.length > 0,
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

  useEffect(() => {
    if (
      !sessionReady ||
      workspaceType !== "work_item_plan" ||
      !manuallySelectedWorkItemPlanArtifactVersion ||
      manuallySelectedWorkItemPlanArtifactVersion.artifact
    ) {
      return;
    }
    let cancelled = false;
    fetchWorkspaceArtifactVersion(
      sessionId,
      manuallySelectedWorkItemPlanArtifactVersion.version,
    )
      .then((response) => {
        if (cancelled) {
          return;
        }
        const artifact = normalizeWorkItemPlanArtifactResponse(response.artifact);
        if (!artifact) {
          return;
        }
        const state = useWorkspaceStore.getState();
        if (state.sessionId !== sessionId) {
          return;
        }
        state.setWorkItemPlanArtifact(artifact, response.version);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [
    manuallySelectedWorkItemPlanArtifactVersion,
    sessionId,
    sessionReady,
    workspaceType,
  ]);

  useUnloadGuard({
    enabled: UNLOAD_GUARDED_STAGES.has(stage),
    message: UNLOAD_GUARD_MESSAGE,
  });

  function handleStartGeneration() {
    const { providers, reviewerEnabled, reviewRounds } =
      useWorkspaceStore.getState();
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

  function handleChoiceResponse(
    entry: ChatEntry,
    response: ChoiceResponsePayload,
  ) {
    const requestId = requestIdFromEntry(entry);
    if (!requestId) {
      return;
    }
    sendChoiceResponse(
      requestId,
      response.selected_option_ids,
      response.free_text,
    );
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

  const handleLoadContent = useCallback(
    async (currentSessionId: string, ref: WorkspaceContentRef) => {
      if (ref.kind === "execution_output") {
        const response = await fetchWorkspaceEventOutput(
          currentSessionId,
          ref.nodeId,
          ref.eventId,
        );
        return response.output;
      }
      if (ref.kind === "provider_prompt") {
        const response = await fetchWorkspacePrompt(
          currentSessionId,
          ref.nodeId,
        );
        return response.prompt;
      }
      throw new Error("不支持加载该内容类型");
    },
    [],
  );

  const handleCacheContent = useCallback(
    (key: string, value: string) => {
      const state = useWorkspaceStore.getState();
      if (state.sessionId !== sessionId) {
        return;
      }
      state.setContentCacheEntry(key, value);
    },
    [sessionId],
  );

  const handleLoadArtifactVersion = useCallback(
    async (version: number) => {
      const response = await fetchWorkspaceArtifactVersion(sessionId, version);
      return response.markdown;
    },
    [sessionId],
  );

  const handleCacheArtifactContent = useCallback(
    (version: number, value: string) => {
      const state = useWorkspaceStore.getState();
      if (state.sessionId !== sessionId) {
        return;
      }
      state.setArtifactContentCacheEntry(version, value);
    },
    [sessionId],
  );

  const providerPanel = (
    <ProviderConfigDialogButton
      providers={providers}
      editable={stageConfig.providerEditable}
      onSelectProvider={(role, provider) => selectProvider(role, provider)}
      reviewerEnabled={reviewerEnabled}
      onToggleReviewer={(enabled) =>
        useWorkspaceStore.setState({ reviewerEnabled: enabled })
      }
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
                ts:
                  abortedByDisconnectNode.completed_at ??
                  abortedByDisconnectNode.started_at,
              }
            : null
        }
        onAcknowledge={(nodeIds) =>
          useWorkspaceStore.getState().setAcknowledgedAbortedNodes(nodeIds)
        }
        onViewTimeline={
          abortedByDisconnectNode
            ? () =>
                useWorkspaceStore
                  .getState()
                  .setSelectedNode(abortedByDisconnectNode.node_id)
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
            <span className="font-mono text-xs font-semibold">
              {protocolError.code}
            </span>
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
              displayedWorkItemPlanArtifact ? (
                <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)]">
                  {showingHistoricalWorkItemPlanArtifact ? null : (
                    <WorkItemPlanStagedPanel
                      activeNodeType={activeNode?.node_type ?? null}
                      artifact={displayedWorkItemPlanArtifact}
                      onAcceptOutline={() => sendAuthorDecision("accept")}
                      onSelectMode={sendSelectWorkItemGenerationMode}
                      onRequestOutlineRevision={() =>
                        sendRequestOutlineRevision()
                      }
                      onDraftDecision={sendWorkItemDraftDecision}
                      onBatchDecision={sendWorkItemBatchDecision}
                      onCompileRecoveryAction={
                        sendWorkItemPlanCompileRecoveryAction
                      }
                    />
                  )}
                  <WorkItemPlanArtifactPanel
                    artifact={displayedWorkItemPlanArtifact}
                    versions={workItemPlanArtifactVersions}
                    selectedVersion={
                      displayedWorkItemPlanArtifactVersion?.version ?? null
                    }
                    onSelectVersion={
                      setSelectedWorkItemPlanArtifactVersionNumber
                    }
                    activeNodeType={activeNode?.node_type ?? null}
                    readonly={showingHistoricalWorkItemPlanArtifact}
                    className="min-h-0"
                  />
                </div>
              ) : (
                <>
                  {workItemPlanCandidate ? (
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
                  )}
                </>
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
                <ReviewDecisionActionBar
                  options={reviewDecisionOptions}
                  onSelectDecision={sendReviewDecision}
                  onSelectRevisionPath={handleSelectRevisionPath}
                />
              ) : null}
              <ChatInputBar
                stage={stage}
                activeNodeType={activeNode?.node_type ?? null}
                workItemPlanArtifact={workItemPlanArtifact}
                disabled={inputDisabled}
                onSendContextNote={sendContextNote}
                onStartGeneration={handleStartGeneration}
                onSendHumanDecision={(content) =>
                  sendHumanConfirm("request-change", content)
                }
                onAuthorDecision={handleAuthorDecision}
                onSelectWorkItemGenerationMode={
                  sendSelectWorkItemGenerationMode
                }
                onRequestOutlineRevision={() => sendRequestOutlineRevision()}
                onWorkItemDraftDecision={sendWorkItemDraftDecision}
                onWorkItemBatchDecision={sendWorkItemBatchDecision}
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
