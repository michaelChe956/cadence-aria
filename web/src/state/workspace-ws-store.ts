import { create } from "zustand";

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
  | "generation"
  | "review"
  | "review_decision"
  | "revision"
  | "human_confirm"
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
  author: string;
  reviewer?: string | null;
}

export interface ProviderConfigSnapshot {
  author: string;
  reviewer?: string | null;
  review_rounds: number;
}

export interface TimelineNode {
  node_id: string;
  node_type: TimelineNodeType;
  agent?: string | null;
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
  generated_by: string;
  reviewed_by?: string | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  created_at: string;
  source_node_id: string;
}

export interface TimelineNodeDetail {
  nodeId: string;
  messages: WsMessage[];
  streamingContent: string;
  executionEvents: ExecutionEvent[];
  verdict?: ReviewVerdict | null;
}

export interface ReviewDecisionRequired {
  node_id: string;
  round: number;
  options: string[];
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
  }) => void;
  appendStreamChunk: (content: string, nodeId?: string | null) => void;
  completeMessage: (messageId: string, checkpointId: string, nodeId?: string | null) => void;
  setStage: (stage: string) => void;
  setArtifact: (markdown: string) => void;
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
};

export const useWorkspaceStore = create<WorkspaceWsState & WorkspaceWsActions>((set) => ({
  ...initialState,

  setSessionState: (state) =>
    set({
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
      timelineNodes: state.timeline_nodes ?? [],
      activeNodeId: state.active_node_id ?? null,
      selectedNodeId:
        state.active_node_id ?? state.timeline_nodes?.[state.timeline_nodes.length - 1]?.node_id ?? null,
      nodeDetails: detailsForTimelineNodes(state.timeline_nodes ?? []),
      artifactVersions: state.artifact_versions ?? [],
      pendingDecision: null,
      error: null,
    }),

  appendStreamChunk: (content, nodeId) =>
    set((prev) => {
      if (!nodeId) {
        return { streamingContent: prev.streamingContent + content };
      }
      const details = { ...prev.nodeDetails };
      const detail = ensureNodeDetail(details, nodeId);
      detail.streamingContent += content;
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
          content: detail.streamingContent,
          checkpoint_id: checkpointId,
          created_at: new Date().toISOString(),
        };
        detail.messages = [...detail.messages, newMessage];
        detail.streamingContent = "";
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

  setArtifact: (markdown) => set({ artifact: markdown }),

  addTimelineNode: (node) =>
    set((prev) => ({
      timelineNodes: [...prev.timelineNodes, node],
      activeNodeId: node.node_id,
      selectedNodeId: node.node_id,
      nodeDetails: {
        ...prev.nodeDetails,
        [node.node_id]: prev.nodeDetails[node.node_id] ?? emptyNodeDetail(node.node_id),
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
      return { nodeDetails: details };
    }),

  setPendingDecision: (decision) => set({ pendingDecision: decision }),

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
        detail.executionEvents = upsertEvent(detail.executionEvents, event);
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

  reset: () => set(initialState),
}));

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

function detailsForTimelineNodes(nodes: TimelineNode[]) {
  return nodes.reduce<Record<string, TimelineNodeDetail>>((details, node) => {
    details[node.node_id] = emptyNodeDetail(node.node_id);
    return details;
  }, {});
}

function emptyNodeDetail(nodeId: string): TimelineNodeDetail {
  return {
    nodeId,
    messages: [],
    streamingContent: "",
    executionEvents: [],
    verdict: null,
  };
}

function ensureNodeDetail(details: Record<string, TimelineNodeDetail>, nodeId: string) {
  const existing = details[nodeId];
  details[nodeId] = existing
    ? {
        ...existing,
        messages: [...existing.messages],
        executionEvents: [...existing.executionEvents],
      }
    : emptyNodeDetail(nodeId);
  return details[nodeId];
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
