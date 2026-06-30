import { create } from "zustand";
import type { ChatEntry, ChoiceResponsePayload } from "./chat-entries";
import type { WorkspaceProviderName } from "../api/types";
import {
  emptyWorkspaceContentCache,
  getWorkspaceContentCacheValue,
  setWorkspaceContentCacheEntry,
} from "./workspace-content-cache";
import {
  buildChatEntries,
  chatEntryId,
  choiceResponseSummary,
  providerEntryMetadata,
} from "./workspace-chat-rebuild";
import {
  detailsForTimelineNodes,
  emptyNodeDetail,
  ensureNodeDetail,
  mergeVisitedStages,
  normalizeTimelineNodeDetails,
  normalizeWorkspaceArtifact,
  STREAMING_STAGES,
  upsertEvent,
  visitedStagesFor,
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
  ArtifactVersion,
  ArtifactVersionSummary,
  ExecutionEvent,
  ExecutionEventKind,
  ExecutionEventStatus,
  NodeDetailSummary,
  PermissionRequest,
  ProtocolErrorState,
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
import type {
  ArtifactVersion,
  TimelineNodeDetail,
  WorkspaceWsActions,
  WorkspaceWsState,
  WsMessage,
} from "./workspace-ws-store-types";

const initialState: WorkspaceWsState = {
  sessionId: null,
  workspaceType: null,
  stage: "prepare_context",
  superpowersEnabled: false,
  openSpecEnabled: false,
  visitedStages: ["prepare_context"],
  messages: [],
  checkpoints: [],
  chatEntries: [],
  artifact: null,
  workItemPlanCandidate: null,
  workItemPlanArtifact: null,
  workItemPlanArtifactVersions: [],
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
  protocolError: null,
  providerLocked: false,
  providerSnapshot: null,
  providerLockedAt: null,
  acknowledgedAbortedNodes: [],
  reviewerEnabled: true,
  reviewRounds: 1,
  pendingReviewDecision: null,
  pendingReviewerSummary: null,
};

const PROVIDER_INTERACTION_GUIDANCE: Record<WorkspaceProviderName, string> = {
  claude_code:
    "当前 author provider 是 Claude Code；需要向用户确认时，必须使用结构化 AskUserQuestion，让同一个 Claude Code 进程等待用户回答后继续。禁止输出文本 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。",
  codex:
    "当前 author provider 是 Codex；需要向用户确认时，必须使用结构化 requestUserInput，让同一个 Codex turn 等待用户回答后继续。禁止输出文本 1/2/3 或 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。",
  fake:
    "当前 author provider 未声明原生结构化交互能力；需要向用户确认时，必须输出 daemon 可识别的暂停信号并交给 text_fallback。禁止伪造 AskUserQuestion 或 requestUserInput 工具调用，也不要把文本选择题作为正常交互路径。",
};

function refreshPreparedContextAuthorGuidance(messages: WsMessage[], provider: WorkspaceProviderName) {
  let changed = false;
  const nextMessages = messages.map((message) => {
    const content = refreshPreparedContextAuthorGuidanceContent(message.content, provider);
    if (content === message.content) {
      return message;
    }
    changed = true;
    return { ...message, content };
  });
  return changed ? nextMessages : messages;
}

function refreshPreparedContextAuthorGuidanceContent(
  content: string,
  provider: WorkspaceProviderName,
) {
  if (!content.startsWith("Workspace 生成任务已准备")) {
    return content;
  }
  const marker = "\n[workflow_discipline]\n";
  const sectionStart = content.indexOf(marker);
  if (sectionStart === -1) {
    return content;
  }
  const disciplineStart = sectionStart + marker.length;
  const sectionEnd = content.indexOf("\n\n[", disciplineStart);
  const safeSectionEnd = sectionEnd === -1 ? content.length : sectionEnd;
  const section = content.slice(disciplineStart, safeSectionEnd);
  const guidanceStart = section.lastIndexOf("\n当前 author provider");
  if (guidanceStart === -1) {
    return content;
  }
  const nextSection =
    section.slice(0, guidanceStart) + "\n" + PROVIDER_INTERACTION_GUIDANCE[provider];
  return content.slice(0, disciplineStart) + nextSection + content.slice(safeSectionEnd);
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
      const artifactVersions = state.artifact_version_summaries ?? state.artifact_versions ?? [];
      const workItemPlanArtifactVersions = workItemPlanVersionsFromSession(
        artifactVersions,
        workItemPlanArtifact,
        state.active_node_id ?? null,
        state.providers.author,
        state.providers.reviewer ?? null,
      );
      const messages = refreshPreparedContextAuthorGuidance(
        state.messages,
        state.providers.author,
      );

      const nextState: WorkspaceWsState = {
        ...prev,
        sessionId: state.session_id,
        workspaceType: state.workspace_type,
        stage: state.stage,
        superpowersEnabled: state.superpowers_enabled ?? false,
        openSpecEnabled: state.openspec_enabled ?? false,
        visitedStages: visitedStagesFor(state.stage),
        messages,
        checkpoints: state.checkpoints,
        chatEntries: [],
        artifact: artifactMarkdown,
        workItemPlanCandidate,
        workItemPlanArtifact,
        workItemPlanArtifactVersions,
        providers: state.providers,
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
          ...normalizeTimelineNodeDetails(state.timeline_node_details ?? {}),
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
        activeRunId: state.active_run_id ?? null,
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
      return { nodeDetails: details };
    }),

  appendBufferedStreamChunk: (content, nodeId, role) =>
    set((prev) => {
      const existing = prev.streamBuffers[nodeId] ?? { chunks: [], visibleText: "", role };
      return {
        streamBuffers: {
          ...prev.streamBuffers,
          [nodeId]: {
            ...existing,
            role,
            chunks: [...existing.chunks, content],
          },
        },
      };
    }),

  flushBufferedStream: (nodeId) =>
    set((prev) => {
      const buffer = prev.streamBuffers[nodeId];
      if (!buffer || buffer.chunks.length === 0) {
        return {};
      }
      const appended = buffer.chunks.join("");
      const visibleText = buffer.visibleText + appended;
      const entryId = chatEntryId(nodeId, "stream-active");
      const index = prev.chatEntries.findIndex((entry) => entry.id === entryId);
      const timelineNode = prev.timelineNodes.find((candidate) => candidate.node_id === nodeId);
      const provider = timelineNode?.agent ?? prev.nodeDetails[nodeId]?.provider?.name ?? null;
      const entry: ChatEntry = {
        id: entryId,
        type: "provider_stream",
        role: buffer.role,
        content: visibleText,
        timestamp: new Date().toISOString(),
        node_id: nodeId,
        content_ref: { kind: "node_stream", nodeId },
        metadata: providerEntryMetadata(timelineNode, provider),
      };
      const chatEntries = index === -1 ? [...prev.chatEntries, entry] : [...prev.chatEntries];
      if (index !== -1) {
        chatEntries[index] = entry;
      }
      return {
        chatEntries,
        streamBuffers: {
          ...prev.streamBuffers,
          [nodeId]: { ...buffer, chunks: [], visibleText },
        },
        activeStreamEntryId: entryId,
      };
    }),

  completeBufferedStream: (nodeId, messageId, checkpointId) => {
    get().flushBufferedStream(nodeId);
    get().completeMessage(messageId, checkpointId, nodeId);
    get().clearBufferedStream(nodeId);
  },

  clearBufferedStream: (nodeId) =>
    set((prev) => {
      if (!prev.streamBuffers[nodeId]) {
        return {};
      }
      const { [nodeId]: _removed, ...streamBuffers } = prev.streamBuffers;
      return { streamBuffers };
    }),

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

  resolveGateEntry: (resolution) =>
    set((prev) => {
      const entries = [...prev.chatEntries];
      for (let index = entries.length - 1; index >= 0; index -= 1) {
        const entry = entries[index];
        if (entry.type === "gate_prompt" && entry.resolved !== true) {
          entries[index] = { ...entry, resolved: true, resolution };
          return { chatEntries: entries };
        }
      }
      return { chatEntries: prev.chatEntries };
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
      const existingArtifactVersion = version === undefined
        ? undefined
        : prev.artifactVersions.find((artifactVersion) => artifactVersion.version === version);
      const existingWorkItemPlanVersion = version === undefined
        ? undefined
        : prev.workItemPlanArtifactVersions.find(
            (artifactVersion) => artifactVersion.version === version,
          );
      const replacesCurrentArtifact =
        artifact &&
        (version === undefined || existingWorkItemPlanVersion?.is_current !== false);
      return {
        workItemPlanArtifact: replacesCurrentArtifact ? artifact : prev.workItemPlanArtifact,
        workItemPlanCandidate: replacesCurrentArtifact ? null : prev.workItemPlanCandidate,
        artifact: replacesCurrentArtifact ? null : prev.artifact,
        artifactVersions:
          artifact && version !== undefined
            ? [
                ...prev.artifactVersions.filter((artifactVersion) => artifactVersion.version !== version),
                {
                  version,
                  generated_by: existingArtifactVersion?.generated_by ?? prev.providers?.author ?? "fake",
                  reviewed_by: existingArtifactVersion?.reviewed_by ?? null,
                  review_verdict: existingArtifactVersion?.review_verdict ?? null,
                  confirmed_by: existingArtifactVersion?.confirmed_by ?? null,
                  is_current: existingArtifactVersion?.is_current ?? false,
                  created_at: existingArtifactVersion?.created_at ?? new Date().toISOString(),
                  source_node_id: existingArtifactVersion?.source_node_id ?? prev.activeNodeId ?? "",
                },
              ].sort((left, right) => left.version - right.version)
            : prev.artifactVersions,
        workItemPlanArtifactVersions:
          artifact && version !== undefined
            ? [
                ...prev.workItemPlanArtifactVersions.filter(
                  (artifactVersion) => artifactVersion.version !== version,
                ),
                {
                  version,
                  generated_by: existingWorkItemPlanVersion?.generated_by ?? prev.providers?.author ?? "fake",
                  reviewed_by: existingWorkItemPlanVersion?.reviewed_by ?? prev.providers?.reviewer ?? null,
                  review_verdict: existingWorkItemPlanVersion?.review_verdict ?? null,
                  confirmed_by: existingWorkItemPlanVersion?.confirmed_by ?? null,
                  is_current: existingWorkItemPlanVersion?.is_current ?? false,
                  created_at: existingWorkItemPlanVersion?.created_at ?? new Date().toISOString(),
                  source_node_id: existingWorkItemPlanVersion?.source_node_id ?? prev.activeNodeId ?? "",
                  artifact,
                },
              ].sort((left, right) => left.version - right.version)
            : prev.workItemPlanArtifactVersions,
      };
    }),

  addTimelineNode: (node) =>
    set((prev) => {
      const retrySourceNodeId = node.retry?.retry_of_node_id ?? null;
      const streamBuffers = { ...prev.streamBuffers };
      if (retrySourceNodeId) {
        delete streamBuffers[retrySourceNodeId];
      }
      const sourceActiveEntryId = retrySourceNodeId
        ? chatEntryId(retrySourceNodeId, "stream-active")
        : null;
      return {
        timelineNodes: [...prev.timelineNodes, node],
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

  resolveChoiceRequest: (id, selectedOptionIds, freeText) =>
    set((prev) => {
      let responseContent = "已选择";
      const response: ChoiceResponsePayload = {
        selected_option_ids: selectedOptionIds,
        free_text: freeText,
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
    set((prev) => {
      const current = prev.providers ?? { author: "claude_code", reviewer: "codex" };
      const providers =
        role === "author"
          ? { ...current, author: provider }
          : { ...current, reviewer: provider };
      const messages =
        role === "author"
          ? refreshPreparedContextAuthorGuidance(prev.messages, provider)
          : prev.messages;
      const shouldRebuildChatEntries = messages !== prev.messages;
      const nextState = { ...prev, providers, messages };
      return {
        providers,
        messages,
        chatEntries: shouldRebuildChatEntries
          ? buildChatEntries(nextState)
          : prev.chatEntries,
      };
    }),

  setAcknowledgedAbortedNodes: (nodeIds) =>
    set({ acknowledgedAbortedNodes: Array.from(new Set(nodeIds)) }),

  reset: () => set(initialState),
}));
