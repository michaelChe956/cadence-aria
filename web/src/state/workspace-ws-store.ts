import { create } from "zustand";
import type { ChatEntry, ChoiceResponsePayload } from "./chat-entries";
import { snapshotGateFingerprint } from "./cockpit-action-routing";
import { gateArchiveClosedCopy, gateArchiveFeedbackCopy } from "./gate-prompt-copy";
import {
  emptyWorkspaceContentCache,
  getWorkspaceContentCacheValue,
  setWorkspaceContentCacheEntry,
} from "./workspace-content-cache";
import { refreshPreparedContextAuthorGuidance } from "./workspace-ws-store-guidance";
import { setProviderSelection } from "./workspace-ws-store-providers";
import {
  appendBufferedStreamChunk,
  clearBufferedStream,
  flushBufferedStream,
} from "./workspace-ws-store-stream-buffers";
import {
  beginHumanPresentationSave,
  completeHumanPresentationSave,
  failHumanPresentationSave,
  failPendingHumanPresentationSaves,
  humanPresentationRevisionsFromSession,
} from "./workspace-ws-store-presentations";
import {
  buildChatEntries,
  chatEntryId,
  choiceResponseSummary,
  providerEntryMetadata,
} from "./workspace-chat-rebuild";
import {
  detailsForTimelineNodes,
  emptyWorkItemPlanProjectionArtifacts,
  emptyNodeDetail,
  ensureNodeDetail,
  mergeSnapshotNodeDetail,
  mergeVisitedStages,
  normalizeTimelineNodeDetails,
  normalizeWorkspaceArtifact,
  STREAMING_STAGES,
  upsertArtifactVersionSummary,
  upsertEvent,
  upsertWorkItemPlanArtifactVersion,
  visitedStagesFor,
  workItemPlanProjectionArtifactsFromVersions,
  workItemPlanVersionsFromSession,
} from "./workspace-ws-store-helpers";
export { chatRoleForTimelineNode } from "./workspace-ws-store-helpers";
export {
  selectChatPanelState,
  selectPrepareContextNotes,
  selectWorkspaceHeaderState,
  workspaceContentCacheKey,
} from "./workspace-ws-selectors";
export type {
  AdvanceCommandState,
  ArtifactVersion,
  ArtifactVersionSummary,
  ExecutionEvent,
  ExecutionEventKind,
  ExecutionEventStatus,
  GateClosureDecision,
  HumanGateClosure,
  HumanGateTurnState,
  HumanGateTurnStatus,
  NodeDetailSummary,
  PermissionRequest,
  ProtocolDiagnostic,
  ProtocolErrorState,
  RecoverableInterruptedRun,
  ProviderConfigSnapshot,
  ProviderStatus,
  ReviewDecisionRequired,
  ReviewFinding,
  ReviewFindingSeverity,
  ReviewGate,
  ReviewVerdict,
  ReviewVerdictType,
  TimelineNode,
  TimelineNodeDetail,
  TimelineNodeRetry,
  TimelineNodeRetryError,
  TimelineNodeStatus,
  TimelineNodeType,
  WorkspaceArtifact,
  WorkspaceWsActions,
  WorkspaceWsState,
  WsCheckpoint,
  WsConnectionStatus,
  WsMessage,
  WsProviderConfig,
} from "./workspace-ws-store-types";
import type { ArtifactVersion, TimelineNodeDetail, WorkspaceWsActions, WorkspaceWsState, WsMessage } from "./workspace-ws-store-types";
const initialState: WorkspaceWsState = {
  sessionId: null,
  workspaceType: null,
  stage: "prepare_context",
  superpowersEnabled: false,
  openSpecEnabled: false,
  sessionStatus: null,
  flowKind: null,
  runPolicy: null,
  runHistory: null,
  reviewInvocationScope: null,
  humanGateSnapshot: null,
  previousGateFindings: null,
  repairReservation: null,
  snapshotGateIdentity: null,
  snapshotGateOpenedAt: null,
  policyDiagnostics: [],
  providerStartLedger: [],
  singleCandidatePhase: null,
  workItemPlanSourceRevisionRef: null,
  planCandidateIrRef: null,
  mechanicalReportRef: null,
  publicationProvenanceRef: null,
  visitedStages: ["prepare_context"],
  messages: [],
  checkpoints: [],
  chatEntries: [],
  artifact: null,
  workItemPlanCandidate: null,
  workItemPlanArtifact: null,
  workItemPlanArtifactVersions: [],
  workItemPlanProjectionArtifacts: emptyWorkItemPlanProjectionArtifacts(),
  humanPresentationRevisions: {},
  humanPresentationSaveStates: {},
  providers: null,
  connectionStatus: "disconnected",
  streamingContent: "",
  streamBuffers: {},
  activeStreamEntryId: null,
  pendingPermissions: [],
  providerStatus: "starting",
  executionEvents: [],
  timelineNodes: [],
  activeNodeId: null,
  selectedNodeId: null,
  nodeDetails: {},
  nodeSummaries: {},
  contentCache: emptyWorkspaceContentCache(),
  artifactContentCache: emptyWorkspaceContentCache(),
  artifactVersions: [],
  pendingDecision: null,
  error: null,
  activeRunId: null,
  recoverableInterruptedRun: null,
  protocolError: null,
  providerLocked: false,
  providerSnapshot: null,
  providerLockedAt: null,
  acknowledgedAbortedNodes: [],
  reviewerEnabled: true,
  reviewRounds: 1,
  permissionModes: { author: "auto", reviewer: "auto" },
  pendingReviewDecision: null,
  pendingReviewerSummary: null,
  humanGateTurn: null,
  planRepair: null,
  humanGateClosure: null,
  advanceCommands: {},
  protocolDiagnostics: [],
  connectionCloseDiagnostics: [],
};

function snapshotIdentityFor(
  fingerprint: string,
  openedAt: string,
): string {
  return `snapshot:${openedAt}:${fingerprint}`;
}

function isArchivedGateEntry(entry: ChatEntry): boolean {
  return entry.type === "gate_prompt" && typeof entry.metadata?.gate_archive_round === "number";
}

/**
 * F-49 A1/B5：把被新一轮门取代的旧门卡留档——只读（resolved=true）+ 轮次 + 该轮
 * 提交的反馈摘要（未记录到反馈时不谎报「已提交反馈」）。resolution 用
 * `superseded`，与 confirm/terminate（关门决定）区分。
 */
function archiveGateEntry(entry: ChatEntry, round: number): ChatEntry {
  const submitted = entry.metadata?.submitted_feedback;
  const note =
    typeof submitted === "string" && submitted.trim()
      ? gateArchiveFeedbackCopy(round, submitted)
      : gateArchiveClosedCopy(round);
  return {
    ...entry,
    resolved: true,
    resolution: "superseded",
    metadata: {
      ...entry.metadata,
      gate_archive_round: round,
      gate_archive_note: note,
    },
  };
}

/**
 * F-49 A1：门内轮次切换即收口旧门卡。命中条件是「未决 + 门身份与来卡不同」——
 * 同身份重放（引擎同 turn_id 重放）只更新同一条目，不触发留档。
 */
function supersedeStaleGateEntries(entries: ChatEntry[], incoming: ChatEntry): ChatEntry[] {
  const incomingIdentity = incoming.metadata?.gate_identity;
  if (typeof incomingIdentity !== "string") {
    // 无门身份的卡（legacy 形态）不构成「新一轮」凭据：fail-closed 不动既有卡。
    return [...entries];
  }
  let round = entries.filter(isArchivedGateEntry).length;
  return entries.map((entry) => {
    if (entry.type !== "gate_prompt" || entry.resolved === true) {
      return entry;
    }
    if (entry.metadata?.gate_identity === incomingIdentity) {
      return entry;
    }
    round += 1;
    return archiveGateEntry(entry, round);
  });
}

export const useWorkspaceStore = create<WorkspaceWsState & WorkspaceWsActions>((set, get) => ({
  ...initialState,

  setSessionState: (state) =>
    set((prev) => {
      const timelineNodes = state.timeline_nodes ?? [];
      const selectedNodeStillExists =
        prev.sessionId === state.session_id &&
        prev.selectedNodeId !== null &&
        timelineNodes.some((node) => node.node_id === prev.selectedNodeId);
      const defaultSelectedNodeId =
        state.active_node_id ?? timelineNodes[timelineNodes.length - 1]?.node_id ?? null;

      const { artifactMarkdown, workItemPlanCandidate, workItemPlanArtifact } =
        normalizeWorkspaceArtifact(state.artifact);
      const fullArtifactVersions = state.artifact_versions ?? [];
      const artifactVersions = state.artifact_version_summaries ?? fullArtifactVersions;
      const workItemPlanArtifactVersions = workItemPlanVersionsFromSession(
        artifactVersions,
        fullArtifactVersions,
        workItemPlanArtifact,
        state.active_node_id ?? null,
        state.providers.author,
        state.providers.reviewer ?? null,
      );
      const messages = refreshPreparedContextAuthorGuidance(
        state.messages,
        state.providers.author,
      );
      const humanPresentationRevisions = humanPresentationRevisionsFromSession(
        state.human_presentation_revisions,
      );

      const sameSession = prev.sessionId === state.session_id;
      const snapshotGateFingerprintValue = state.human_gate_snapshot
        ? snapshotGateFingerprint(state.human_gate_snapshot)
        : null;
      const snapshotGateOpenedAt = snapshotGateFingerprintValue
        ? sameSession && prev.snapshotGateIdentity?.endsWith(`:${snapshotGateFingerprintValue}`)
          ? prev.snapshotGateOpenedAt
          : new Date().toISOString()
        : null;
      const snapshotGateIdentity = snapshotGateFingerprintValue && snapshotGateOpenedAt
        ? snapshotIdentityFor(snapshotGateFingerprintValue, snapshotGateOpenedAt)
        : null;
      const snapshotGateChanged = sameSession && snapshotGateIdentity !== prev.snapshotGateIdentity;
      const humanGateSnapshot = state.human_gate_snapshot
        ? { ...state.human_gate_snapshot, opened_at: snapshotGateOpenedAt ?? undefined }
        : null;
      // C2（REQ-HGC-02 场景 3）：跨轮 delta 的前轮事实——同会话且快照被替换
      // （复评重建；与 T1 后端 carry-forward 同一「快照在场=同一 logical gate
      // episode」判据）时保留旧快照 findings；首次开门/关门/跨会话/刷新一律
      // null（历史不全 → 门卡显式 unknown，不猜）。
      const previousGateFindings =
        sameSession && humanGateSnapshot !== null && snapshotGateChanged
          ? prev.humanGateSnapshot?.findings ?? null
          : humanGateSnapshot !== null && sameSession
            ? prev.previousGateFindings
            : null;
      const durableGateStillOpen = humanGateSnapshot !== null;
      // 重建口径：durable snapshot 在场即门仍开；否则要求仍处 legacy human_confirm 阶段。
      const gateProjectionStillOpen = durableGateStillOpen || state.stage === "human_confirm";
      // F-47 REQ-NDR-01：快照应用是 merge 而非重建——原实现整表重建 nodeDetails，
      // 任何一帧 session_state 都会把 REST 已水合的 detail 换成空壳，而水合去重 ref
      // 阻止二次拉取 → token 行静默消失且不自愈（诊断 §2.2 B2，探针实测）。
      // 保留规则：同会话 + 节点仍在 timeline 中 + 条目已物化（REST 水合 / 内联 /
      // 本地内容落库，参见 `hydration_pending`）。纯占位壳按当前节点重建，避免沿用
      // 陈旧 status/ended_at；快照内联 detail 最后覆盖（REQ-NDR-01 场景二：内联优先）。
      // 跨会话不保留：节点 id 同构（timeline_node_00N），串场即错误数据。
      const timelineNodeIdSet = new Set(timelineNodes.map((node) => node.node_id));
      const preservedNodeDetails = sameSession
        ? Object.fromEntries(
            Object.entries(prev.nodeDetails).filter(
              ([nodeId, detail]) =>
                timelineNodeIdSet.has(nodeId) && detail.hydration_pending !== true,
            ),
          )
        : {};
      // 快照内联 detail 覆盖对应节点（场景二：内联优先），但对已物化条目走 merge——
      // 内联投影裁剪掉的执行事件 output 不覆盖已水合的载荷（见 mergeSnapshotNodeDetail）。
      const inlineNodeDetails = Object.fromEntries(
        Object.entries(normalizeTimelineNodeDetails(state.timeline_node_details ?? {})).map(
          ([nodeId, inline]) => [
            nodeId,
            preservedNodeDetails[nodeId]
              ? mergeSnapshotNodeDetail(preservedNodeDetails[nodeId], inline)
              : inline,
          ],
        ),
      );
      const nextState: WorkspaceWsState = {
        ...prev,
        sessionId: state.session_id,
        workspaceType: state.workspace_type,
        planRepair: state.plan_repair ?? null,
        stage: state.stage,
        superpowersEnabled: state.superpowers_enabled ?? false,
        openSpecEnabled: state.openspec_enabled ?? false,
        sessionStatus: state.session_status,
        flowKind: state.flow_kind,
        runPolicy: state.run_policy,
        runHistory: state.run_history,
        previousGateFindings,
        reviewInvocationScope: state.review_invocation_scope ?? null,
        humanGateSnapshot,
        snapshotGateIdentity,
        snapshotGateOpenedAt,
        repairReservation: state.repair_reservation ?? null,
        policyDiagnostics: state.policy_diagnostics ?? [],
        providerStartLedger: state.provider_start_ledger ?? [],
        singleCandidatePhase: state.single_candidate_phase ?? null,
        workItemPlanSourceRevisionRef: state.work_item_plan_source_revision_ref ?? null,
        planCandidateIrRef: state.plan_candidate_ir_ref ?? null,
        mechanicalReportRef: state.mechanical_report_ref ?? null,
        publicationProvenanceRef: state.publication_provenance_ref ?? null,
        visitedStages: visitedStagesFor(state.stage),
        messages,
        checkpoints: state.checkpoints,
        chatEntries: [],
        artifact: artifactMarkdown,
        workItemPlanCandidate,
        workItemPlanArtifact,
        workItemPlanArtifactVersions,
        workItemPlanProjectionArtifacts:
          workItemPlanProjectionArtifactsFromVersions(
            workItemPlanArtifactVersions,
          ),
        humanPresentationRevisions,
        humanPresentationSaveStates: {},
        providers: state.providers,
        permissionModes: state.providers.permission_modes ?? {
          author: "auto",
          reviewer: "auto",
        },
        streamingContent: "",
        streamBuffers: {},
        activeStreamEntryId: null,
        pendingPermissions: [],
        providerStatus: "starting",
        executionEvents: [],
        timelineNodes,
        activeNodeId: state.active_node_id ?? null,
        selectedNodeId: selectedNodeStillExists ? prev.selectedNodeId : defaultSelectedNodeId,
        nodeDetails: {
          ...detailsForTimelineNodes(timelineNodes, state.session_id),
          ...preservedNodeDetails,
          ...inlineNodeDetails,
        },
        nodeSummaries: state.timeline_node_summaries ?? {},
        contentCache:
          prev.sessionId === state.session_id ? prev.contentCache : emptyWorkspaceContentCache(),
        artifactContentCache:
          prev.sessionId === state.session_id
            ? prev.artifactContentCache
            : emptyWorkspaceContentCache(),
        artifactVersions,
        pendingDecision: null,
        pendingReviewDecision: null,
        pendingReviewerSummary: null,
        error: null,
        protocolError: null,
        activeRunId: state.active_run_id ?? null,
        recoverableInterruptedRun: state.recoverable_interrupted_run ?? null,
        // D8：同会话重连保留 turn/command 去重集与诊断（不丢弃）；
        // 跨会话一律清空，避免上一会话的门与推进记忆串到新会话。
        humanGateTurn:
          sameSession && gateProjectionStillOpen && prev.humanGateClosure === null
            ? prev.humanGateTurn
            : null,
        humanGateClosure:
          sameSession && gateProjectionStillOpen && !snapshotGateChanged
            ? prev.humanGateClosure
            : null,
        advanceCommands: sameSession ? prev.advanceCommands : {},
        protocolDiagnostics: sameSession ? prev.protocolDiagnostics : [],
        connectionCloseDiagnostics: sameSession ? prev.connectionCloseDiagnostics : [],
        reviewerEnabled: state.reviewer_enabled_at_start ?? prev.reviewerEnabled,
      };
      return {
        ...nextState,
        chatEntries: buildChatEntries(nextState),
      };
    }),

  appendStreamChunk: (content, nodeId) =>
    set((prev) => {
      if (!nodeId) {
        return { streamingContent: prev.streamingContent + content };
      }
      const details = { ...prev.nodeDetails };
      const detail = ensureNodeDetail(details, nodeId);
      detail.streaming_content += content;
      const lastEventAt = new Date().toISOString();
      return {
        nodeDetails: details,
        // REQ-UI37-18：持续流式事件刷新节点事件钟，长 running 行不得被判为静默。
        timelineNodes: prev.timelineNodes.some((node) => node.node_id === nodeId)
          ? prev.timelineNodes.map((node) =>
              node.node_id === nodeId ? { ...node, last_event_at: lastEventAt } : node,
            )
          : prev.timelineNodes,
      };
    }),

  appendBufferedStreamChunk: (content, nodeId, role) =>
    set((prev) => appendBufferedStreamChunk(prev, content, nodeId, role)),

  flushBufferedStream: (nodeId) => set((prev) => flushBufferedStream(prev, nodeId)),

  completeBufferedStream: (nodeId, messageId, checkpointId) => {
    get().flushBufferedStream(nodeId);
    get().completeMessage(messageId, checkpointId, nodeId);
    get().clearBufferedStream(nodeId);
  },

  clearBufferedStream: (nodeId) => set((prev) => clearBufferedStream(prev, nodeId)),

  clearAllStreamBuffers: () => set({ streamBuffers: {} }),

  completeMessage: (messageId, checkpointId, nodeId) =>
    set((prev) => {
      if (nodeId) {
        const details = { ...prev.nodeDetails };
        const detail = ensureNodeDetail(details, nodeId);
        const newMessage: WsMessage = {
          id: messageId,
          role: "assistant",
          content: detail.streaming_content,
          checkpoint_id: checkpointId,
          created_at: new Date().toISOString(),
        };
        detail.messages = [...detail.messages, newMessage];
        detail.streaming_content = "";
        return {
          nodeDetails: details,
          checkpoints: [
            ...prev.checkpoints,
            {
              id: checkpointId,
              message_index: prev.messages.length + detail.messages.length,
              stage: prev.stage,
              created_at: new Date().toISOString(),
            },
          ],
        };
      }
      const newMessage: WsMessage = {
        id: messageId,
        role: "assistant",
        content: prev.streamingContent,
        checkpoint_id: checkpointId,
        created_at: new Date().toISOString(),
      };
      return {
        messages: [...prev.messages, newMessage],
        checkpoints: [
          ...prev.checkpoints,
          {
            id: checkpointId,
            message_index: prev.messages.length + 1,
            stage: prev.stage,
            created_at: new Date().toISOString(),
          },
        ],
        streamingContent: "",
      };
    }),

  appendChatEntry: (entry) =>
    set((prev) => {
      const index = prev.chatEntries.findIndex((existing) => existing.id === entry.id);
      const next = index === -1 ? [...prev.chatEntries, entry] : [...prev.chatEntries];
      if (index !== -1) {
        next[index] = entry;
      }
      return {
        chatEntries: next,
        activeStreamEntryId: entry.type === "provider_stream" ? entry.id : prev.activeStreamEntryId,
      };
    }),

  // F-49 A1：门卡专用 upsert——门内轮次切换（门载体从 durable snapshot 切到 typed
  // turn）即收口旧门卡，保证任一时刻只有一张可动作门卡。判据是门身份
  // （gate_identity）而非条目 id：同一次门内存活期内 id 会变
  // （snapshot:<opened_at>:<fp> → <turn_id>），只有身份能识别「同一张门」。
  upsertGatePromptEntry: (entry) =>
    set((prev) => {
      const next = supersedeStaleGateEntries(prev.chatEntries, entry);
      const index = next.findIndex((existing) => existing.id === entry.id);
      if (index === -1) {
        next.push(entry);
      } else {
        next[index] = entry;
      }
      return { chatEntries: next };
    }),

  // F-49 A4/B5：记下本卡提交的反馈文本——提交成功后清空输入（A4）与旧轮留档摘要
  // （B5）都以此为准，不在渲染层重读输入框。
  recordGateFeedbackSubmission: (entryId, feedback) =>
    set((prev) => {
      const index = prev.chatEntries.findIndex((entry) => entry.id === entryId);
      if (index === -1) {
        return {};
      }
      const next = [...prev.chatEntries];
      next[index] = {
        ...next[index],
        metadata: { ...next[index].metadata, submitted_feedback: feedback },
      };
      return { chatEntries: next };
    }),

  applyHumanGateTurnOpen: (turnId, commandId, remainingBudget) =>
    set((prev) => {
      if (prev.humanGateTurn?.turn_id === turnId) {
        // 引擎同 turn_id 重放（decisions.rs Replayed）→ 幂等消费：
        // 不新建条目、不重算预算、不刷新 opened_at。
        return {};
      }
      return {
        humanGateTurn: {
          turn_id: turnId,
          command_id: commandId,
          remaining_budget: remainingBudget,
          status: "open",
          artifact_ref: null,
          failure_class: null,
          failure_message: null,
          opened_at: new Date().toISOString(),
          inlineError: null,
          // F-49 A8：记下本 turn 开出时的门快照身份——门以新快照重建（修订成功后
          // Evaluate 重置预算）后，预算以新快照为准，见 selectGateProjection。
          opened_snapshot_identity: prev.snapshotGateIdentity,
        },
        humanGateClosure: null,
      };
    }),

  applyHumanGateTurnCompleted: (turnId, artifactRef) =>
    set((prev) => {
      if (prev.humanGateTurn?.turn_id !== turnId) {
        return {};
      }
      return {
        humanGateTurn: {
          ...prev.humanGateTurn,
          status: "awaiting_confirm",
          artifact_ref: artifactRef,
        },
      };
    }),

  applyHumanGateTurnFailed: (turnId, failureClass, message) =>
    set((prev) => {
      if (prev.humanGateTurn?.turn_id !== turnId) {
        return {};
      }
      return {
        humanGateTurn: {
          ...prev.humanGateTurn,
          status: "failed",
          failure_class: failureClass,
          failure_message: message,
        },
      };
    }),

  applyHumanGateBusy: (turnId) =>
    set((prev) => {
      if (prev.humanGateTurn?.turn_id !== turnId) {
        return {};
      }
      return { humanGateTurn: { ...prev.humanGateTurn, status: "busy" } };
    }),

  applyHumanGateClosed: (decision, stage) =>
    set({ humanGateClosure: { decision, stage } }),

  applyAdvanceCompleted: (commandId, attemptId, workspaceEntry) =>
    set((prev) => {
      const existing = prev.advanceCommands[commandId];
      if (existing?.status === "completed" || existing?.status === "rejected") {
        return {};
      }
      return {
        advanceCommands: {
          ...prev.advanceCommands,
          [commandId]: {
            command_id: commandId,
            status: "completed",
            code: null,
            reason: null,
            attempt_id: attemptId,
            workspace_entry: workspaceEntry,
            inlineError: null,
          },
        },
      };
    }),

  applyAdvanceRejected: (commandId, code, reason) =>
    set((prev) => {
      const existing = prev.advanceCommands[commandId];
      if (existing?.status === "completed" || existing?.status === "rejected") {
        return {};
      }
      return {
        advanceCommands: {
          ...prev.advanceCommands,
          [commandId]: {
            command_id: commandId,
            status: "rejected",
            code,
            reason,
            attempt_id: null,
            workspace_entry: null,
            inlineError: null,
          },
        },
      };
    }),

  recordProtocolDiagnostic: (diagnostic) =>
    set((prev) => ({
      protocolDiagnostics: [...prev.protocolDiagnostics, diagnostic].slice(-50),
    })),
  recordConnectionCloseDiagnostic: (diagnostic) =>
    set((prev) => ({
      connectionCloseDiagnostics: [...prev.connectionCloseDiagnostics, diagnostic].slice(-50),
    })),


  applyGateProtocolError: (turnId, error) =>
    set((prev) =>
      prev.humanGateTurn?.turn_id === turnId
        ? { humanGateTurn: { ...prev.humanGateTurn, inlineError: error } }
        : {},
    ),

  applyAdvanceProtocolError: (commandId, error) =>
    set((prev) => {
      const command = prev.advanceCommands[commandId];
      return command
        ? {
            advanceCommands: {
              ...prev.advanceCommands,
              [commandId]: { ...command, inlineError: error },
            },
          }
        : {};
    }),

  // F-49 A3：关门决定必须收口「全部」未决门卡。此前只 resolve 倒序命中的第一张：
  // 门内轮次切换后残留的旧卡永不被收口（human_gate_closed 只在 approve/abandon
  // 时发出），关门后对话流仍留可点旧卡。
  resolveGateEntry: (resolution) =>
    set((prev) => {
      let changed = false;
      const entries = prev.chatEntries.map((entry) => {
        if (entry.type !== "gate_prompt" || entry.resolved === true) {
          return entry;
        }
        changed = true;
        return { ...entry, resolved: true, resolution };
      });
      return changed ? { chatEntries: entries } : {};
    }),

  updateStreamingEntry: (entryId, content) =>
    set((prev) => {
      const index = prev.chatEntries.findIndex((entry) => entry.id === entryId);
      if (index === -1) {
        return {
          chatEntries: [
            ...prev.chatEntries,
            {
              id: entryId,
              type: "provider_stream",
              role: "author",
              content,
              timestamp: new Date().toISOString(),
            },
          ],
          activeStreamEntryId: entryId,
        };
      }

      const next = [...prev.chatEntries];
      const current = next[index];
      next[index] = {
        ...current,
        content: `${current.content}${content}`,
      };
      return {
        chatEntries: next,
        activeStreamEntryId: entryId,
      };
    }),

  setEntryUsage: (entryId, usage) =>
    set((prev) => {
      const index = prev.chatEntries.findIndex((entry) => entry.id === entryId);
      if (index === -1) {
        return {};
      }
      const next = [...prev.chatEntries];
      const current = next[index];
      next[index] = { ...current, metadata: { ...current.metadata, usage } };
      return { chatEntries: next };
    }),

  finalizeStreamingEntry: (entryId) =>
    set((prev) =>
      prev.activeStreamEntryId === entryId
        ? { activeStreamEntryId: null }
        : { activeStreamEntryId: prev.activeStreamEntryId },
    ),

  rebuildChatEntries: () =>
    set((prev) => ({
      chatEntries: buildChatEntries(prev),
      activeStreamEntryId: null,
    })),

  setStage: (stage) =>
    set((prev) => ({
      stage,
      visitedStages: mergeVisitedStages(prev.visitedStages, stage),
      streamingContent: STREAMING_STAGES.has(stage) ? prev.streamingContent : "",
      // 进入 human_confirm = 新一代 legacy 门开启：清上一轮闭环，避免新门被误判为已收。
      // 其余阶段迁移不清：闭环绑定在当前 turn/snapshot 上，清掉会让已收敛的门在收件箱复活
      // （terminate 时序为 HumanGateClosed → StageChange("completed")）；
      // 新 typed 门的旧闭环由 applyHumanGateTurnOpen 清除。
      humanGateClosure:
        stage === "human_confirm" && stage !== prev.stage ? null : prev.humanGateClosure,
    })),

  setArtifact: (markdown, version) =>
    set((prev) => {
      if (version === undefined) {
        return { artifact: markdown };
      }

      const existing = prev.artifactVersions.find((artifact) => artifact.version === version);
      const nextVersion: ArtifactVersion = {
        version,
        markdown,
        generated_by: existing?.generated_by ?? prev.providers?.author ?? "fake",
        reviewed_by: existing?.reviewed_by ?? null,
        review_verdict: existing?.review_verdict ?? null,
        confirmed_by: existing?.confirmed_by ?? null,
        created_at: existing?.created_at ?? new Date().toISOString(),
        source_node_id: existing?.source_node_id ?? prev.activeNodeId ?? "",
      };

      return {
        artifact: markdown,
        artifactVersions: [
          ...prev.artifactVersions.filter((artifact) => artifact.version !== version),
          nextVersion,
        ].sort((left, right) => left.version - right.version),
      };
    }),

  setWorkItemPlanCandidate: (candidate) =>
    set((prev) => ({
      workItemPlanCandidate: candidate,
      workItemPlanArtifact: candidate ? null : prev.workItemPlanArtifact,
    })),

  setWorkItemPlanArtifact: (artifact, version) =>
    set((prev) => {
      const existingWorkItemPlanVersion = version === undefined
        ? undefined
        : prev.workItemPlanArtifactVersions.find(
            (artifactVersion) => artifactVersion.version === version,
          );
      const replacesCurrentArtifact =
        artifact &&
        (version === undefined || existingWorkItemPlanVersion?.is_current !== false);
      const createdAt = new Date().toISOString();
      const workItemPlanArtifactVersions =
        artifact && version !== undefined
          ? upsertWorkItemPlanArtifactVersion(
              prev.workItemPlanArtifactVersions,
              artifact,
              version,
              Boolean(replacesCurrentArtifact),
              {
                author: prev.providers?.author ?? "fake",
                reviewer: prev.providers?.reviewer ?? null,
                activeNodeId: prev.activeNodeId ?? "",
                createdAt,
              },
            )
          : prev.workItemPlanArtifactVersions;
      return {
        workItemPlanArtifact: replacesCurrentArtifact ? artifact : prev.workItemPlanArtifact,
        workItemPlanCandidate: replacesCurrentArtifact ? null : prev.workItemPlanCandidate,
        artifact: replacesCurrentArtifact ? null : prev.artifact,
        artifactVersions:
          artifact && version !== undefined
            ? upsertArtifactVersionSummary(
                prev.artifactVersions,
                version,
                Boolean(replacesCurrentArtifact),
                {
                  author: prev.providers?.author ?? "fake",
                  activeNodeId: prev.activeNodeId ?? "",
                  createdAt,
                },
              )
            : prev.artifactVersions,
        workItemPlanArtifactVersions,
        workItemPlanProjectionArtifacts:
          workItemPlanProjectionArtifactsFromVersions(
            workItemPlanArtifactVersions,
          ),
      };
    }),

  beginHumanPresentationSave: (sourceProjectionBundleId) =>
    set((prev) => beginHumanPresentationSave(prev, sourceProjectionBundleId)),
  completeHumanPresentationSave: (revision) =>
    set((prev) => completeHumanPresentationSave(prev, revision)),
  failHumanPresentationSave: (sourceProjectionBundleId, message) =>
    set((prev) => failHumanPresentationSave(prev, sourceProjectionBundleId, message)),
  failPendingHumanPresentationSaves: (message) =>
    set((prev) => failPendingHumanPresentationSaves(prev, message)),
  addTimelineNode: (node) =>
    set((prev) => {
      // v38 复验 #1（重复「Review Round 1」卡）：活跃 run 期间重连/初帧 attach，服务端把
      // snapshot 基线压到补发窗口首事件之前并重发窗口帧（attachment.rs 的
      // activate_attachment_with_initial_frames / journal.rs 的 active_run_window），窗口内
      // 含客户端已消化的 timeline_node_created 重叠帧；前端对 session_state 无条件拉低
      // event_seq 基线（useWorkspaceWs.ts），重叠 created 帧必然通过 seq 去重到达这里。
      // 因此按 node_id 幂等：节点已在列表即忽略追加（首写优先——已有节点可能是快照终态
      // 或已吸收 update 帧，重放的初态 created 不得回退它），active/selected 亦不动。
      if (prev.timelineNodes.some((existing) => existing.node_id === node.node_id)) {
        return prev;
      }
      const retrySourceNodeId = node.retry?.retry_of_node_id ?? null;
      const streamBuffers = { ...prev.streamBuffers };
      if (retrySourceNodeId) {
        delete streamBuffers[retrySourceNodeId];
      }
      const sourceActiveEntryId = retrySourceNodeId
        ? chatEntryId(retrySourceNodeId, "stream-active")
        : null;
      return {
        timelineNodes: [
          ...prev.timelineNodes,
          { ...node, last_event_at: node.last_event_at ?? new Date().toISOString() },
        ],
        activeNodeId: node.node_id,
        selectedNodeId: node.node_id,
        nodeDetails: {
          ...prev.nodeDetails,
          [node.node_id]:
            prev.nodeDetails[node.node_id] ??
            emptyNodeDetail(node.node_id, { sessionId: prev.sessionId, node }),
        },
        chatEntries: retrySourceNodeId
          ? prev.chatEntries.filter((entry) => entry.node_id !== retrySourceNodeId)
          : prev.chatEntries,
        streamBuffers,
        activeStreamEntryId:
          sourceActiveEntryId && prev.activeStreamEntryId === sourceActiveEntryId
            ? null
            : prev.activeStreamEntryId,
      };
    }),

  updateTimelineNode: (nodeId, status, summary, completedAt) =>
    set((prev) => ({
      timelineNodes: prev.timelineNodes.map((node) =>
        node.node_id === nodeId
          ? {
              ...node,
              status,
              summary: summary ?? node.summary,
              completed_at: completedAt ?? node.completed_at,
              // REQ-UI37-18：timeline_node_updated 本身就是一次引擎事件。
              last_event_at: new Date().toISOString(),
            }
          : node,
      ),
    })),

  setSelectedNode: (nodeId) => set({ selectedNodeId: nodeId }),

  setNodeDetail: (detail) =>
    set((prev) => {
      const nodeDetails = { ...prev.nodeDetails, [detail.node_id]: detail };
      const nextState = { ...prev, nodeDetails };
      return {
        nodeDetails,
        chatEntries: buildChatEntries(nextState),
      };
    }),

  setNodeVerdict: (nodeId, verdict) =>
    set((prev) => {
      const details = { ...prev.nodeDetails };
      const detail = ensureNodeDetail(details, nodeId);
      detail.verdict = verdict;
      return {
        nodeDetails: details,
        pendingReviewDecision: {
          verdict: verdict.verdict,
          summary: verdict.summary,
        },
        pendingReviewerSummary: {
          verdict: verdict.verdict,
          points: [verdict.summary, verdict.comments].filter((point) => point.trim().length > 0),
        },
      };
    }),

  setContentCacheEntry: (key, value, now) =>
    set((prev) => ({
      contentCache: setWorkspaceContentCacheEntry(prev.contentCache, key, value, now),
    })),

  touchContentCacheEntry: (key, now) =>
    set((prev) => {
      const touched = getWorkspaceContentCacheValue(prev.contentCache, key, now);
      return touched ? { contentCache: touched.cache } : {};
    }),

  setArtifactContentCacheEntry: (version, value, now) =>
    set((prev) => ({
      artifactContentCache: setWorkspaceContentCacheEntry(
        prev.artifactContentCache,
        String(version),
        value,
        now,
      ),
    })),

  touchArtifactContentCacheEntry: (version, now) =>
    set((prev) => {
      const touched = getWorkspaceContentCacheValue(
        prev.artifactContentCache,
        String(version),
        now,
      );
      return touched ? { artifactContentCache: touched.cache } : {};
    }),

  setPendingDecision: (decision) =>
    set((prev) => {
      if (!decision) {
        return { pendingDecision: null, pendingReviewDecision: null };
      }
      const verdict = prev.nodeDetails[decision.node_id]?.verdict;
      return {
        pendingDecision: decision,
        pendingReviewDecision: {
          verdict: verdict?.verdict ?? "revise",
          summary: verdict?.summary ?? "",
        },
      };
    }),

  setConnectionStatus: (status) => set({ connectionStatus: status }),

  addPermissionRequest: (request) =>
    set((prev) => ({
      pendingPermissions: [
        ...prev.pendingPermissions.filter((pending) => pending.id !== request.id),
        request,
      ],
    })),

  resolvePermissionRequest: (id, approved) =>
    set((prev) => {
      let matched = false;
      const nextEntries = prev.chatEntries.map((entry) => {
        if (entry.type !== "permission_request" || entry.metadata?.request_id !== id) {
          return entry;
        }
        matched = true;
        return {
          ...entry,
          resolved: approved !== undefined ? true : entry.resolved,
          metadata: {
            ...entry.metadata,
            approved,
            response:
              approved !== undefined
                ? {
                    approved,
                  }
                : entry.metadata?.response,
          },
        };
      });

      if (approved !== undefined && matched) {
        const responseEntry: ChatEntry = {
          id: `permission_response:${id}`,
          type: "permission_response",
          role: "user",
          content: approved ? "已允许" : "已拒绝",
          timestamp: new Date().toISOString(),
          metadata: { request_id: id, approved },
        };
        return {
          pendingPermissions: prev.pendingPermissions.filter((request) => request.id !== id),
          chatEntries: [
            ...nextEntries.filter((entry) => entry.id !== responseEntry.id),
            responseEntry,
          ],
        };
      }

      return {
        pendingPermissions: prev.pendingPermissions.filter((request) => request.id !== id),
        chatEntries: nextEntries,
      };
    }),

  resolveChoiceRequest: (id, selectedOptionIds, freeText, answers) =>
    set((prev) => {
      let responseContent = "已选择";
      const response: ChoiceResponsePayload = {
        selected_option_ids: selectedOptionIds,
        free_text: freeText,
        ...(answers && answers.length > 0 ? { answers } : {}),
      };
      const nextEntries = prev.chatEntries.map((entry) => {
        if (entry.type !== "choice_request" || entry.metadata?.request_id !== id) {
          return entry;
        }
        responseContent = `已选择${choiceResponseSummary(entry, response)}`;
        return {
          ...entry,
          resolved: true,
          metadata: {
            ...entry.metadata,
            response,
          },
        };
      });
      const responseEntry: ChatEntry = {
        id: `choice_response:${id}`,
        type: "choice_response",
        role: "user",
        content: responseContent,
        timestamp: new Date().toISOString(),
        metadata: {
          request_id: id,
          ...response,
        },
      };
      return {
        chatEntries: [...nextEntries.filter((entry) => entry.id !== responseEntry.id), responseEntry],
      };
    }),

  rejectChoiceRequest: (id, reason) =>
    set((prev) => ({
      chatEntries: prev.chatEntries
        .filter((entry) => entry.id !== `choice_response:${id}`)
        .map((entry) => {
          if (entry.type !== "choice_request" || entry.metadata?.request_id !== id) {
            return entry;
          }
          const metadata = { ...(entry.metadata ?? {}) };
          delete metadata.response;
          return {
            ...entry,
            resolved: true,
            metadata: {
              ...metadata,
              rejected: true,
              rejection_reason: reason,
            },
          };
        }),
    })),

  setProviderStatus: (status) => set({ providerStatus: status }),

  setTimelineNodesForTest: (nodes) => set({ timelineNodes: nodes }),
  setActiveNodeId: (nodeId) => set({ activeNodeId: nodeId }),
  setSessionStatus: (status) => set({ sessionStatus: status }),
  setHumanGateSnapshot: (snapshot) =>
    set((prev) => {
      const openedAt = snapshot ? snapshot.opened_at ?? new Date().toISOString() : null;
      const snapshotGateIdentity = snapshot && openedAt
        ? snapshotIdentityFor(snapshotGateFingerprint(snapshot), openedAt)
        : null;
      return {
        humanGateSnapshot: snapshot ? { ...snapshot, opened_at: openedAt ?? undefined } : null,
        snapshotGateOpenedAt: openedAt,
        snapshotGateIdentity,
      };
    }),
  setSessionIdForTest: (sessionId) => set({ sessionId }),

  upsertExecutionEvent: (event) =>
    set((prev) => {
      if (event.node_id) {
        const details = { ...prev.nodeDetails };
        const detail = ensureNodeDetail(details, event.node_id);
        detail.execution_events = upsertEvent(detail.execution_events, event);
        return { nodeDetails: details };
      }
      const index = prev.executionEvents.findIndex(
        (existing) => existing.event_id === event.event_id,
      );
      if (index === -1) {
        return { executionEvents: [...prev.executionEvents, event] };
      }
      const next = [...prev.executionEvents];
      next[index] = { ...next[index], ...event };
      return { executionEvents: next };
    }),

  clearExecutionEvents: () => set({ executionEvents: [] }),

  setError: (error) => set({ error }),
  clearStreaming: () => set({ streamingContent: "", streamBuffers: {}, activeStreamEntryId: null }),

  selectNodeDetail: (nodeId) => {
    if (!nodeId) {
      return null;
    }
    return get().nodeDetails[nodeId] ?? null;
  },

  setProtocolError: (error) => set({ protocolError: error }),

  setProviderLocked: (payload) =>
    set({
      providerLocked: payload !== null,
      providerSnapshot: payload?.snapshot ?? null,
      providerLockedAt: payload?.locked_at ?? null,
    }),

  setProviderSelection: (role, provider) =>
    set((prev) => setProviderSelection(prev, role, provider)),
  setPermissionMode: (role, mode) =>
    set((prev) => ({
      permissionModes: { ...prev.permissionModes, [role]: mode },
    })),
  setAcknowledgedAbortedNodes: (nodeIds) =>
    set({ acknowledgedAbortedNodes: Array.from(new Set(nodeIds)) }),
  reset: () => set(initialState),
}));
