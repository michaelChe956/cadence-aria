import { create } from "zustand";
import type { NodeDetail, WorkspaceProviderName } from "../api/types";

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
}

export interface ArtifactVersion {
  version: number;
  markdown: string;
  generated_by: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  created_at: string;
  source_node_id: string;
}

export type TimelineNodeDetail = NodeDetail;

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
  visitedStages: string[];
  messages: WsMessage[];
  checkpoints: WsCheckpoint[];
  artifact: string | null;
  providers: WsProviderConfig | null;
  connectionStatus: WsConnectionStatus;
  streamingContent: string;
  pendingPermissions: PermissionRequest[];
  providerStatus: ProviderStatus;
  executionEvents: ExecutionEvent[];
  timelineNodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  nodeDetails: Record<string, TimelineNodeDetail>;
  artifactVersions: ArtifactVersion[];
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
    messages: WsMessage[];
    checkpoints: WsCheckpoint[];
    artifact: string | null;
    providers: WsProviderConfig;
    timeline_nodes?: TimelineNode[];
    active_node_id?: string | null;
    artifact_versions?: ArtifactVersion[];
    timeline_node_details?: Record<string, TimelineNodeDetail>;
    active_run_id?: string | null;
  }) => void;
  appendStreamChunk: (content: string, nodeId?: string | null) => void;
  completeMessage: (messageId: string, checkpointId: string, nodeId?: string | null) => void;
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
  setNodeVerdict: (nodeId: string, verdict: ReviewVerdict) => void;
  setPendingDecision: (decision: ReviewDecisionRequired | null) => void;
  setConnectionStatus: (status: WsConnectionStatus) => void;
  addPermissionRequest: (request: PermissionRequest) => void;
  resolvePermissionRequest: (id: string) => void;
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
  setAcknowledgedAbortedNodes: (nodeIds: string[]) => void;
  reset: () => void;
}

const initialState: WorkspaceWsState = {
  sessionId: null,
  workspaceType: null,
  stage: "prepare_context",
  visitedStages: ["prepare_context"],
  messages: [],
  checkpoints: [],
  artifact: null,
  providers: null,
  connectionStatus: "disconnected",
  streamingContent: "",
  pendingPermissions: [],
  providerStatus: "starting",
  executionEvents: [],
  timelineNodes: [],
  activeNodeId: null,
  selectedNodeId: null,
  nodeDetails: {},
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

      return {
        sessionId: state.session_id,
        workspaceType: state.workspace_type,
        stage: state.stage,
        visitedStages: visitedStagesFor(state.stage),
        messages: state.messages,
        checkpoints: state.checkpoints,
        artifact: state.artifact,
        providers: state.providers,
        streamingContent: "",
        pendingPermissions: [],
        providerStatus: "starting",
        executionEvents: [],
        timelineNodes,
        activeNodeId: state.active_node_id ?? null,
        selectedNodeId: selectedNodeStillExists ? prev.selectedNodeId : defaultSelectedNodeId,
        nodeDetails: {
          ...detailsForTimelineNodes(timelineNodes, state.session_id),
          ...(state.timeline_node_details ?? {}),
        },
        artifactVersions: state.artifact_versions ?? [],
        pendingDecision: null,
        pendingReviewDecision: null,
        pendingReviewerSummary: null,
        error: null,
        activeRunId: state.active_run_id ?? null,
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

  resolvePermissionRequest: (id) =>
    set((prev) => ({
      pendingPermissions: prev.pendingPermissions.filter((request) => request.id !== id),
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

  clearStreaming: () => set({ streamingContent: "" }),

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

  setAcknowledgedAbortedNodes: (nodeIds) =>
    set({ acknowledgedAbortedNodes: Array.from(new Set(nodeIds)) }),

  reset: () => set(initialState),
}));

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
  if (node?.node_type === "author_run") {
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
