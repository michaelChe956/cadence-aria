import { create } from "zustand";
import type { NodeDetail, WorkspaceProviderName } from "../api/types";
import type { WorkspaceContentRef } from "./chat-entries";
import type {
  ChatEntry,
  ChatEntryResolution,
  ChatEntryRole,
  ChoiceResponsePayload,
} from "./chat-entries";
import {
  emptyWorkspaceContentCache,
  getWorkspaceContentCacheValue,
  setWorkspaceContentCacheEntry,
  type WorkspaceContentCache,
} from "./workspace-content-cache";

export type WsConnectionStatus = "disconnected" | "connecting" | "connected" | "error";
export type ProviderStatus =
  | "starting"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type ExecutionEventKind = "provider" | "turn" | "command" | "output" | "artifact";
export type ExecutionEventStatus =
  | "started"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type TimelineNodeType =
  | "prepare_context"
  | "context_note"
  | "start_generation"
  | "author_confirm"
  | "author_run"
  | "reviewer_run"
  | "review_decision"
  | "revision"
  | "human_confirm"
  | "aborted_by_disconnect"
  | "protocol_error"
  | "completed";
export type TimelineNodeStatus = "active" | "paused" | "completed" | "failed" | "skipped";
export type ReviewVerdictType = "pass" | "revise" | "needs_human";

export interface PermissionRequest {
  id: string;
  tool_name: string;
  description: string;
  risk_level: "low" | "medium" | "high";
}

export interface ExecutionEvent {
  event_id: string;
  node_id?: string | null;
  agent?: string | null;
  kind: ExecutionEventKind;
  status: ExecutionEventStatus;
  title: string;
  detail?: string | null;
  command?: string | null;
  cwd?: string | null;
  output?: string | null;
  exit_code?: number | null;
}

export interface WsMessage {
  id: string;
  role: string;
  content: string;
  checkpoint_id?: string | null;
  created_at: string;
}

export interface WsCheckpoint {
  id: string;
  message_index: number;
  stage: string;
  created_at: string;
}

export interface WsProviderConfig {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
}

export interface ProviderConfigSnapshot {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
}

export interface TimelineNode {
  node_id: string;
  node_type: TimelineNodeType;
  agent?: WorkspaceProviderName | null;
  stage: string;
  round?: number | null;
  status: TimelineNodeStatus;
  title: string;
  summary?: string | null;
  started_at: string;
  completed_at?: string | null;
  duration_ms?: number | null;
  artifact_ref?: string | null;
  provider_config_snapshot: ProviderConfigSnapshot;
}

export interface ReviewVerdict {
  verdict: ReviewVerdictType;
  comments: string;
  summary: string;
  findings?: ReviewFinding[];
  review_gate?: ReviewGate;
}

export type ReviewFindingSeverity =
  | "blocking"
  | "must_fix"
  | "strong_recommend_fix"
  | "suggestion"
  | "minor"
  | "optional";

export type ReviewGate =
  | "requires_revision"
  | "user_confirm_allowed"
  | "user_triage_required";

export interface ReviewFinding {
  severity: ReviewFindingSeverity;
  message: string;
  evidence: string;
  impact: string;
  required_action: string;
}

export interface ArtifactVersionSummary {
  version: number;
  markdown?: string;
  generated_by: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  is_current?: boolean;
  created_at: string;
  source_node_id: string;
}

export interface ArtifactVersion extends ArtifactVersionSummary {
  markdown: string;
}

export type TimelineNodeDetail = NodeDetail;

export interface NodeDetailSummary {
  node_id: string;
  node_type: string;
  status: string;
  agent_role?: string | null;
  provider_name?: string | null;
  prompt_size: number;
  prompt_preview?: string | null;
  stream_size: number;
  stream_preview?: string | null;
  execution_event_count: number;
  has_large_outputs: boolean;
  artifact_ref?: string | null;
  started_at: string;
  ended_at?: string | null;
}

export interface ReviewDecisionRequired {
  node_id: string;
  round: number;
  options: string[];
}

export interface ProtocolErrorState {
  code: string;
  message: string;
}

export interface WorkspaceWsState {
  sessionId: string | null;
  workspaceType: string | null;
  stage: string;
  superpowersEnabled: boolean;
  openSpecEnabled: boolean;
  visitedStages: string[];
  messages: WsMessage[];
  checkpoints: WsCheckpoint[];
  chatEntries: ChatEntry[];
  artifact: string | null;
  providers: WsProviderConfig | null;
  connectionStatus: WsConnectionStatus;
  streamingContent: string;
  streamBuffers: Record<string, { chunks: string[]; visibleText: string; role: ChatEntryRole }>;
  activeStreamEntryId: string | null;
  pendingPermissions: PermissionRequest[];
  providerStatus: ProviderStatus;
  executionEvents: ExecutionEvent[];
  timelineNodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  nodeDetails: Record<string, TimelineNodeDetail>;
  nodeSummaries: Record<string, NodeDetailSummary>;
  contentCache: WorkspaceContentCache;
  artifactContentCache: WorkspaceContentCache;
  artifactVersions: ArtifactVersionSummary[];
  pendingDecision: ReviewDecisionRequired | null;
  error: string | null;
  activeRunId: string | null;
  protocolError: ProtocolErrorState | null;
  providerLocked: boolean;
  providerSnapshot: ProviderConfigSnapshot | null;
  providerLockedAt: string | null;
  acknowledgedAbortedNodes: string[];
  reviewerEnabled: boolean;
  reviewRounds: number;
  pendingReviewDecision: { verdict: string; summary: string } | null;
  pendingReviewerSummary: { verdict: string; points: string[] } | null;
}

export interface WorkspaceWsActions {
  setSessionState: (state: {
    session_id: string;
    workspace_type: string;
    stage: string;
    superpowers_enabled?: boolean;
    openspec_enabled?: boolean;
    messages: WsMessage[];
    checkpoints: WsCheckpoint[];
    artifact: string | null;
    providers: WsProviderConfig;
    timeline_nodes?: TimelineNode[];
    active_node_id?: string | null;
    artifact_versions?: ArtifactVersion[];
    artifact_version_summaries?: ArtifactVersionSummary[];
    timeline_node_details?: Record<string, TimelineNodeDetail>;
    timeline_node_summaries?: Record<string, NodeDetailSummary>;
    active_run_id?: string | null;
  }) => void;
  appendStreamChunk: (content: string, nodeId?: string | null) => void;
  appendBufferedStreamChunk: (content: string, nodeId: string, role: ChatEntryRole) => void;
  flushBufferedStream: (nodeId: string) => void;
  completeBufferedStream: (nodeId: string, messageId: string, checkpointId: string) => void;
  clearBufferedStream: (nodeId: string) => void;
  clearAllStreamBuffers: () => void;
  completeMessage: (messageId: string, checkpointId: string, nodeId?: string | null) => void;
  appendChatEntry: (entry: ChatEntry) => void;
  resolveGateEntry: (resolution: ChatEntryResolution) => void;
  updateStreamingEntry: (entryId: string, content: string) => void;
  finalizeStreamingEntry: (entryId: string) => void;
  rebuildChatEntries: () => void;
  setStage: (stage: string) => void;
  setArtifact: (markdown: string, version?: number) => void;
  addTimelineNode: (node: TimelineNode) => void;
  updateTimelineNode: (
    nodeId: string,
    status: TimelineNodeStatus,
    summary?: string | null,
    completedAt?: string | null,
  ) => void;
  setSelectedNode: (nodeId: string | null) => void;
  setNodeDetail: (detail: TimelineNodeDetail) => void;
  setNodeVerdict: (nodeId: string, verdict: ReviewVerdict) => void;
  setContentCacheEntry: (key: string, value: string, now?: number) => void;
  touchContentCacheEntry: (key: string, now?: number) => void;
  setArtifactContentCacheEntry: (version: number, value: string, now?: number) => void;
  touchArtifactContentCacheEntry: (version: number, now?: number) => void;
  setPendingDecision: (decision: ReviewDecisionRequired | null) => void;
  setConnectionStatus: (status: WsConnectionStatus) => void;
  addPermissionRequest: (request: PermissionRequest) => void;
  resolvePermissionRequest: (id: string, approved?: boolean) => void;
  resolveChoiceRequest: (
    id: string,
    selectedOptionIds: string[],
    freeText: string | null,
  ) => void;
  rejectChoiceRequest: (id: string, reason: string) => void;
  setProviderStatus: (status: ProviderStatus) => void;
  upsertExecutionEvent: (event: ExecutionEvent) => void;
  clearExecutionEvents: () => void;
  setError: (error: string | null) => void;
  clearStreaming: () => void;
  selectNodeDetail: (nodeId: string | null | undefined) => TimelineNodeDetail | null;
  setProtocolError: (error: ProtocolErrorState | null) => void;
  setProviderLocked: (
    payload: { snapshot: ProviderConfigSnapshot; locked_at: string } | null,
  ) => void;
  setProviderSelection: (role: "author" | "reviewer", provider: WorkspaceProviderName) => void;
  setAcknowledgedAbortedNodes: (nodeIds: string[]) => void;
  reset: () => void;
}

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

      const nextState: WorkspaceWsState = {
        ...prev,
        sessionId: state.session_id,
        workspaceType: state.workspace_type,
        stage: state.stage,
        superpowersEnabled: state.superpowers_enabled ?? false,
        openSpecEnabled: state.openspec_enabled ?? false,
        visitedStages: visitedStagesFor(state.stage),
        messages: state.messages,
        checkpoints: state.checkpoints,
        chatEntries: [],
        artifact: state.artifact,
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
        artifactVersions: state.artifact_version_summaries ?? state.artifact_versions ?? [],
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
      const provider =
        prev.timelineNodes.find((candidate) => candidate.node_id === nodeId)?.agent ??
        prev.nodeDetails[nodeId]?.provider?.name ??
        null;
      const entry: ChatEntry = {
        id: entryId,
        type: "provider_stream",
        role: buffer.role,
        content: visibleText,
        timestamp: new Date().toISOString(),
        node_id: nodeId,
        content_ref: { kind: "node_stream", nodeId },
        metadata: provider ? { provider } : undefined,
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

  addTimelineNode: (node) =>
    set((prev) => ({
      timelineNodes: [...prev.timelineNodes, node],
      activeNodeId: node.node_id,
      selectedNodeId: node.node_id,
      nodeDetails: {
        ...prev.nodeDetails,
        [node.node_id]:
          prev.nodeDetails[node.node_id] ??
          emptyNodeDetail(node.node_id, { sessionId: prev.sessionId, node }),
      },
    })),

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
      return {
        providers:
          role === "author"
            ? { ...current, author: provider }
            : { ...current, reviewer: provider },
      };
    }),

  setAcknowledgedAbortedNodes: (nodeIds) =>
    set({ acknowledgedAbortedNodes: Array.from(new Set(nodeIds)) }),

  reset: () => set(initialState),
}));

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

const STAGE_ORDER = [
  "prepare_context",
  "running",
  "author_confirm",
  "cross_review",
  "human_confirm",
  "completed",
];
const STREAMING_STAGES = new Set(["running", "cross_review", "revision"]);

function visitedStagesFor(stage: string) {
  const index = STAGE_ORDER.indexOf(flowStageFor(stage));
  if (index === -1) {
    return [stage];
  }
  return STAGE_ORDER.slice(0, index + 1);
}

function mergeVisitedStages(current: string[], stage: string) {
  return Array.from(new Set([...current, ...visitedStagesFor(stage)]));
}

function flowStageFor(stage: string) {
  if (stage === "review_decision" || stage === "revision") {
    return "cross_review";
  }
  return stage;
}

function detailsForTimelineNodes(nodes: TimelineNode[], sessionId: string) {
  return nodes.reduce<Record<string, TimelineNodeDetail>>((details, node) => {
    details[node.node_id] = emptyNodeDetail(node.node_id, { sessionId, node });
    return details;
  }, {});
}

function emptyNodeDetail(
  nodeId: string,
  options: { sessionId?: string | null; node?: TimelineNode } = {},
): TimelineNodeDetail {
  const node = options.node;
  return {
    node_id: nodeId,
    session_id: options.sessionId ?? "",
    node_type: node?.node_type ?? "author_run",
    status: node?.status ?? "active",
    agent_role: agentRoleFor(node),
    provider: node?.agent ? { name: node.agent, model: "" } : null,
    prompt: null,
    messages: [],
    streaming_content: "",
    execution_events: [],
    permission_events: [],
    verdict: null,
    artifact_ref: null,
    is_revision: node?.node_type === "revision",
    base_artifact_ref: null,
    started_at: node?.started_at ?? "",
    ended_at: node?.completed_at ?? null,
  };
}

function ensureNodeDetail(details: Record<string, TimelineNodeDetail>, nodeId: string) {
  const existing = details[nodeId];
  details[nodeId] = existing
    ? {
        ...existing,
        messages: [...existing.messages],
        execution_events: [...existing.execution_events],
        permission_events: [...existing.permission_events],
      }
    : emptyNodeDetail(nodeId);
  return details[nodeId];
}

function agentRoleFor(node?: TimelineNode): "author" | "reviewer" | null {
  if (node?.node_type === "author_run" || node?.node_type === "revision") {
    return "author";
  }
  if (node?.node_type === "reviewer_run") {
    return "reviewer";
  }
  return null;
}

function upsertEvent(events: ExecutionEvent[], event: ExecutionEvent) {
  const index = events.findIndex((existing) => existing.event_id === event.event_id);
  if (index === -1) {
    return [...events, event];
  }
  const next = [...events];
  next[index] = { ...next[index], ...event };
  return next;
}

function normalizeTimelineNodeDetails(details: Record<string, TimelineNodeDetail>) {
  return Object.fromEntries(
    Object.entries(details).map(([nodeId, detail]) => [
      nodeId,
      {
        ...detail,
        execution_events: deduplicateExecutionEvents(detail.execution_events),
      },
    ]),
  );
}

function deduplicateExecutionEvents(events: ExecutionEvent[]) {
  return events.reduce<ExecutionEvent[]>((deduped, event) => {
    const index = deduped.findIndex((existing) => existing.event_id === event.event_id);
    if (index === -1) {
      deduped.push(event);
    } else {
      deduped[index] = { ...deduped[index], ...event };
    }
    return deduped;
  }, []);
}

function buildChatEntries(state: WorkspaceWsState): ChatEntry[] {
  const entries: ChatEntry[] = [];

  for (const message of state.messages) {
    if (!isPreparedWorkspaceContextMessage(message)) {
      continue;
    }

    entries.push({
      id: `prepared-context:${message.id}`,
      type: "context_note",
      role: "user",
      content: message.content,
      timestamp: message.created_at,
      metadata: { prepared: true },
    });
  }

  for (const node of state.timelineNodes) {
    const detail = state.nodeDetails[node.node_id];
    if (!detail) {
      continue;
    }

    if (node.node_type === "context_note") {
      const content = textFromSources([
        detail.streaming_content,
        node.summary,
        detail.messages.map((message) => message.content).join("\n"),
      ]);
      if (content) {
        entries.push({
          id: chatEntryId(node.node_id, "context"),
          type: "context_note",
          role: "user",
          content,
          timestamp: detail.started_at || node.started_at,
          node_id: node.node_id,
        });
      }
      continue;
    }

    const role = chatRoleForNode(node);
    if (!role) {
      const marker = timelineAnchorEntry(node, detail);
      if (marker) {
        entries.push(marker);
      }
      continue;
    }

    const summary = state.nodeSummaries[node.node_id];
    const hasPersistedDetailContent = Boolean(
      detail.prompt?.trim() ||
        detail.streaming_content.trim() ||
        detail.messages.length > 0 ||
        detail.execution_events.length > 0,
    );
    if (!hasPersistedDetailContent && summary) {
      entries.push(...providerSummaryEntries(node, summary, role));
      continue;
    }

    const prompt = detail.prompt?.trim();
    if (prompt && !detail.execution_events.some(isProviderPromptEvent)) {
      const provider = providerNameForNode(node, detail);
      entries.push({
        id: chatEntryId(node.node_id, "provider-prompt"),
        type: "execution_event",
        role,
        content: `${providerPromptContent(node.title)} · ${formatContentSize(prompt.length)}`,
        timestamp: detail.started_at || node.started_at,
        node_id: node.node_id,
        metadata: {
          event_id: `${node.node_id}_prompt`,
          node_id: node.node_id,
          agent: provider,
          kind: "output",
          status: "started",
          title: "Provider Prompt",
          detail: "发送给 Workspace provider 的完整提示词",
          command: null,
          cwd: null,
          exit_code: null,
          ...(provider ? { provider } : {}),
        },
        content_ref: { kind: "provider_prompt", nodeId: node.node_id },
        content_size: prompt.length,
        has_full_content: true,
      });
    }

    const streamContent = textFromSources([
      detail.streaming_content,
      detail.messages.map((message) => message.content).join("\n"),
    ]);
    if (streamContent) {
      const provider = providerNameForNode(node, detail);
      entries.push({
        id: chatEntryId(node.node_id, "stream"),
        type: "provider_stream",
        role,
        content: streamContent,
        timestamp: detail.started_at || node.started_at,
        node_id: node.node_id,
        metadata: provider ? { provider } : undefined,
      });
    }

    const latestProviderPrompt = latestProviderPromptEvent(detail.execution_events);
    for (const event of detail.execution_events) {
      const timestamp = detail.started_at || node.started_at;
      const provider = providerNameForNode(node, detail, event);
      if (isProviderPromptEvent(event)) {
        if (event !== latestProviderPrompt) {
          continue;
        }
        entries.push({
          id: chatEntryId(node.node_id, "provider-prompt"),
          type: "execution_event",
          role,
          content: `${executionEventContent(event, node.title)} · ${formatContentSize(event.output.length)}`,
          timestamp,
          node_id: node.node_id,
          metadata: providerPromptEventMetadata(event, provider),
          content_ref: { kind: "provider_prompt", nodeId: node.node_id },
          content_size: event.output.length,
          has_full_content: true,
        });
        continue;
      }
      entries.push({
        id: chatEntryId(node.node_id, `execution-${event.event_id}`),
        type: "execution_event",
        role,
        content: executionEventContent(event, node.title),
        timestamp,
        node_id: node.node_id,
        metadata: provider ? { ...event, provider } : { ...event },
        content_ref: { kind: "execution_output", nodeId: node.node_id, eventId: event.event_id },
        content_size: event.output?.length,
        has_full_content: typeof event.output === "string",
      });
    }

    for (const permission of detail.permission_events) {
      const request = permission.request;
      const requestToolName = stringField(request, "tool_name") ?? "权限请求";
      const requestDescription = stringField(request, "description") ?? "";
      const requestRiskLevel = stringField(request, "risk_level") ?? null;
      const response = permission.response;

      entries.push({
        id: chatEntryId(node.node_id, `permission-request-${permission.request_id}`),
        type: "permission_request",
        role: "system",
        content: requestDescription
          ? `${requestToolName} · ${requestDescription}`
          : requestToolName,
        timestamp: permission.ts,
        node_id: node.node_id,
        metadata: {
          request_id: permission.request_id,
          request,
          response,
          risk_level: requestRiskLevel,
          ts: permission.ts,
        },
      });

      if (response) {
        entries.push({
          id: chatEntryId(node.node_id, `permission-response-${permission.request_id}`),
          type: "permission_response",
          role: "user",
          content: permissionResponseLabel(requestToolName, response),
          timestamp: permission.ts,
          node_id: node.node_id,
          metadata: {
            request_id: permission.request_id,
            request,
            response,
            ts: permission.ts,
          },
        });
      }
    }

    const artifactVersions = state.artifactVersions
      .filter((artifact) => artifact.source_node_id === node.node_id)
      .sort((left, right) => left.version - right.version);
    for (const artifact of artifactVersions) {
      entries.push({
        id: chatEntryId(node.node_id, `artifact-${artifact.version}`),
        type: "artifact_update",
        role: "system",
        content: `产物已更新 -> v${artifact.version}`,
        timestamp: artifact.created_at,
        node_id: node.node_id,
        metadata: {
          version: artifact.version,
          generated_by: artifact.generated_by,
          reviewed_by: artifact.reviewed_by ?? null,
          review_verdict: artifact.review_verdict ?? null,
          confirmed_by: artifact.confirmed_by ?? null,
          source_node_id: artifact.source_node_id,
        },
        content_ref: {
          kind: "artifact_version",
          version: artifact.version,
          sourceNodeId: artifact.source_node_id,
        },
        content_size: typeof artifact.markdown === "string" ? artifact.markdown.length : undefined,
        has_full_content: typeof artifact.markdown === "string",
      });
    }

    if (detail.verdict) {
      const verdictSummary = getStringField(detail.verdict, "summary") ?? "审核结论";
      const verdictValue = getStringField(detail.verdict, "verdict") ?? "revise";
      const verdictComments = getStringField(detail.verdict, "comments") ?? "";
      const verdictFindings = getArrayField(detail.verdict, "findings");
      const reviewGate = getStringField(detail.verdict, "review_gate") ?? "user_confirm_allowed";
      entries.push({
        id: chatEntryId(node.node_id, "review-verdict"),
        type: "review_verdict",
        role: "reviewer",
        content: verdictSummary,
        timestamp: detail.ended_at ?? detail.started_at,
        node_id: node.node_id,
        metadata: {
          verdict: verdictValue,
          comments: verdictComments,
          summary: verdictSummary,
          findings: verdictFindings,
          review_gate: reviewGate,
        },
      });
    }
  }

  const gatePrompt = buildGatePromptEntry(state, entries);
  if (gatePrompt) {
    entries.push(gatePrompt);
  }

  return entries;
}

function providerSummaryEntries(
  node: TimelineNode,
  summary: NodeDetailSummary,
  role: ChatEntryRole,
): ChatEntry[] {
  const entries: ChatEntry[] = [];
  const provider = summary.provider_name ?? node.agent ?? null;
  const timestamp = summary.started_at || node.started_at;
  const streamPreview = summary.stream_preview?.trim();

  if (streamPreview) {
    entries.push({
      id: chatEntryId(node.node_id, "stream-summary"),
      type: "provider_stream",
      role,
      content: streamPreview,
      timestamp,
      node_id: node.node_id,
      metadata: provider ? { provider } : undefined,
    });
  }

  if (summary.prompt_size > 0) {
    entries.push({
      id: chatEntryId(node.node_id, "provider-prompt-summary"),
      type: "execution_event",
      role,
      content: `${providerPromptContent(node.title)} · ${formatContentSize(summary.prompt_size)}`,
      timestamp,
      node_id: node.node_id,
      metadata: {
        event_id: `${node.node_id}_prompt`,
        node_id: node.node_id,
        agent: provider,
        kind: "output",
        status: summary.status,
        title: "Provider Prompt",
        detail: "发送给 Workspace provider 的完整提示词",
        command: null,
        cwd: null,
        exit_code: null,
        ...(provider ? { provider } : {}),
      },
      content_ref: { kind: "provider_prompt", nodeId: node.node_id },
      content_size: summary.prompt_size,
      has_full_content: true,
    });
  }

  if (summary.has_large_outputs || summary.execution_event_count > 0) {
    entries.push({
      id: chatEntryId(node.node_id, "execution-output-summary"),
      type: "execution_event",
      role,
      content: `Execution Output · ${summary.has_large_outputs ? "按需加载" : "摘要"}`,
      timestamp,
      node_id: node.node_id,
      metadata: {
        event_id: `${node.node_id}_output`,
        node_id: node.node_id,
        agent: provider,
        kind: "output",
        status: summary.status,
        title: "Execution Output",
        detail: "Provider execution output 按需加载",
        command: null,
        cwd: null,
        exit_code: null,
        ...(provider ? { provider } : {}),
      },
      content_ref: {
        kind: "execution_output",
        nodeId: node.node_id,
        eventId: `${node.node_id}_output`,
      },
      has_full_content: true,
    });
  }

  return entries;
}

function chatRoleForNode(node: TimelineNode): ChatEntryRole | null {
  if (node.node_type === "author_run" || node.node_type === "revision") {
    return "author";
  }
  if (node.node_type === "reviewer_run") {
    return "reviewer";
  }
  return null;
}

function timelineAnchorEntry(node: TimelineNode, detail: TimelineNodeDetail): ChatEntry | null {
  if (!shouldRenderTimelineAnchor(node)) {
    return null;
  }

  return {
    id: chatEntryId(node.node_id, "timeline-anchor"),
    type: node.node_type === "start_generation" ? "start_generation" : "stage_change",
    role: "system",
    content: timelineAnchorContent(node),
    timestamp: detail.started_at || node.started_at,
    node_id: node.node_id,
    metadata: {
      node_type: node.node_type,
      status: node.status,
      stage: node.stage,
      summary: node.summary ?? null,
      snapshot: node.provider_config_snapshot,
    },
  };
}

function shouldRenderTimelineAnchor(node: TimelineNode) {
  return [
    "start_generation",
    "author_confirm",
    "review_decision",
    "completed",
    "aborted_by_disconnect",
    "protocol_error",
  ].includes(node.node_type);
}

function timelineAnchorContent(node: TimelineNode) {
  const summary = node.summary?.trim();
  return summary ? `${node.title} · ${summary}` : node.title;
}

function buildGatePromptEntry(
  state: WorkspaceWsState,
  entries = state.chatEntries,
): ChatEntry | null {
  if (state.stage !== "human_confirm") {
    return null;
  }

  const gatePromptNode =
    findLatestNodeOfType(state.timelineNodes, "human_confirm") ?? state.timelineNodes.at(-1);
  const latestReview = entries.filter((entry) => entry.type === "review_verdict").at(-1);
  const summary = latestReview?.metadata?.summary?.toString() ?? "";
  const verdict = latestReview?.metadata?.verdict?.toString() ?? "";
  const comments = latestReview?.metadata?.comments?.toString() ?? "";
  const findings = Array.isArray(latestReview?.metadata?.findings)
    ? latestReview.metadata.findings
    : [];
  const reviewGate = latestReview?.metadata?.review_gate?.toString() ?? "";
  const metadata = {
    ...(summary ? { summary } : {}),
    ...(verdict ? { verdict } : {}),
    ...(comments ? { comments } : {}),
    ...(findings.length > 0 ? { findings } : {}),
    ...(reviewGate ? { review_gate: reviewGate } : {}),
  };
  return {
    id: chatEntryId(gatePromptNode?.node_id ?? "human_confirm", "gate-prompt"),
    type: "gate_prompt",
    role: "system",
    content: verdict === "needs_human" ? "需要人工确认" : "等待人工确认",
    timestamp: gatePromptNode?.completed_at ?? gatePromptNode?.started_at ?? new Date().toISOString(),
    node_id: gatePromptNode?.node_id,
    metadata: Object.keys(metadata).length > 0 ? metadata : undefined,
  };
}

function findLatestNodeOfType(nodes: TimelineNode[], type: TimelineNodeType) {
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    if (nodes[index].node_type === type) {
      return nodes[index];
    }
  }
  return null;
}

function chatEntryId(nodeId: string, suffix: string) {
  return `${nodeId}:${suffix}`;
}

function textFromSources(sources: Array<string | null | undefined>) {
  for (const source of sources) {
    const trimmed = source?.trim();
    if (trimmed) {
      return trimmed;
    }
  }
  return "";
}

function executionEventContent(event: ExecutionEvent, nodeTitle?: string | null) {
  const command = event.kind === "command" ? event.command?.trim() : "";
  if (command) {
    return command;
  }
  if (isProviderPromptEvent(event) && nodeTitle) {
    return providerPromptContent(nodeTitle);
  }
  return event.detail ? `${event.title} · ${event.detail}` : event.title;
}

function providerPromptContent(nodeTitle: string) {
  return `${nodeTitle} · Provider Prompt`;
}

function formatContentSize(chars: number) {
  if (chars < 1024) {
    return `${chars} 字符`;
  }
  return `约 ${Math.ceil(chars / 1024)}KB`;
}

function isProviderPromptEvent(
  event: Pick<ExecutionEvent, "title" | "output">,
): event is Pick<ExecutionEvent, "title" | "output"> & { output: string } {
  return event.title === "Provider Prompt" && typeof event.output === "string";
}

function latestProviderPromptEvent(events: ExecutionEvent[]) {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (isProviderPromptEvent(event)) {
      return event;
    }
  }
  return null;
}

function providerPromptEventMetadata(event: ExecutionEvent, provider?: string | null) {
  return {
    event_id: event.event_id,
    node_id: event.node_id ?? null,
    agent: event.agent ?? null,
    kind: event.kind,
    status: event.status,
    title: event.title,
    detail: event.detail ?? null,
    command: event.command ?? null,
    cwd: event.cwd ?? null,
    exit_code: event.exit_code ?? null,
    ...(provider ? { provider } : {}),
  };
}

function providerNameForNode(node: TimelineNode, detail: TimelineNodeDetail, event?: ExecutionEvent) {
  return (
    stringField(event, "agent") ??
    node.agent ??
    stringField(detail.provider, "name")
  );
}

function isPreparedWorkspaceContextMessage(message: WsMessage) {
  return (
    message.role === "system" &&
    (message.content.startsWith("Workspace 生成任务已准备") ||
      message.content.includes("候选 spec 生成器") ||
      message.content.includes("候选 design 生成器") ||
      message.content.includes("候选 work item 生成器"))
  );
}

function stringField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return null;
  }
  const field = value[key];
  return typeof field === "string" ? field : null;
}

function permissionResponseLabel(toolName: string, response: unknown) {
  if (!isRecord(response)) {
    return `权限响应 ${toolName}`;
  }

  if (response.approved === true) {
    return `已允许 ${toolName}`;
  }
  if (response.approved === false) {
    return `已拒绝 ${toolName}`;
  }
  if (response.status === "timeout") {
    return `权限超时 ${toolName}`;
  }
  return `权限响应 ${toolName}`;
}

function choiceResponseSummary(entry: ChatEntry, response: ChoiceResponsePayload) {
  const metadata = entry.metadata;
  const labels = response.selected_option_ids.map((id) => choiceOptionLabel(metadata?.options, id));
  if (response.free_text) {
    labels.push(response.free_text);
  }
  return labels.length > 0 ? `：${labels.join("、")}` : "";
}

function choiceOptionLabel(options: unknown, id: string) {
  if (!Array.isArray(options)) {
    return id;
  }
  const option = options.find(
    (item) => isRecord(item) && stringField(item, "id") === id,
  );
  return isRecord(option) ? stringField(option, "label") ?? id : id;
}

function getStringField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return null;
  }
  const field = value[key];
  return typeof field === "string" ? field : null;
}

function getArrayField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return [];
  }
  const field = value[key];
  return Array.isArray(field) ? field : [];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
