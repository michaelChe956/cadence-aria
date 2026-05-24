import { create } from "zustand";
import type {
  CodeReviewReport,
  CodingAttemptStatus,
  CodingExecutionStage,
  CodingGateRequired,
  CodingTimelineNode,
  CodingTimelineNodeStatus,
  CodingWsOutMessage,
  ExecutionEvent,
  InternalPrReview,
  ProviderConfigSnapshot,
  ReviewRequest,
  TestingReport,
} from "../api/types";
import type { ChatEntry } from "./chat-entries";

export type CodingArtifactTab = "diff" | "tests" | "review" | "git" | "logs";
export type CodingConnectionStatus =
  | "connecting"
  | "connected"
  | "disconnected"
  | "reconnecting";

export interface CodingProtocolError {
  code: string;
  message: string;
}

export interface CodingLogEntry {
  id: string;
  message: string;
  timestamp: string;
  nodeId?: string | null;
}

export interface CodingWorkspaceState {
  attemptId: string | null;
  workItemId: string | null;
  issueId: string | null;
  projectId: string | null;
  status: CodingAttemptStatus | null;
  stage: CodingExecutionStage | null;
  branchName: string | null;
  baseBranch: string | null;
  worktreePath: string | null;
  reworkCount: number;
  maxAutoRework: number;
  headCommit: string | null;
  pushedRemote: string | null;
  providerConfigSnapshot: ProviderConfigSnapshot | null;
  timelineNodes: CodingTimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  chatEntries: ChatEntry[];
  streamingContent: string | null;
  activeStreamNodeId: string | null;
  activeTab: CodingArtifactTab;
  diffSummary: null;
  testingReport: TestingReport | null;
  codeReviewReports: CodeReviewReport[];
  internalPrReview: InternalPrReview | null;
  reviewRequest: ReviewRequest | null;
  logs: CodingLogEntry[];
  connectionStatus: CodingConnectionStatus;
  pendingGates: CodingGateRequired[];
  protocolError: CodingProtocolError | null;
  tabLockedByUser: boolean;
}

export interface CodingWorkspaceActions {
  setSessionState: (
    snapshot: Extract<CodingWsOutMessage, { type: "coding_session_state" }>,
  ) => void;
  updateStage: (stage: CodingExecutionStage) => void;
  addTimelineNode: (node: CodingTimelineNode) => void;
  updateTimelineNode: (
    nodeId: string,
    status: CodingTimelineNodeStatus,
    summary?: string | null,
    completedAt?: string | null,
  ) => void;
  setTestingReport: (report: TestingReport | null) => void;
  addCodeReviewReport: (report: CodeReviewReport) => void;
  setReviewRequest: (request: ReviewRequest | null) => void;
  setInternalPrReview: (review: InternalPrReview | null) => void;
  addExecutionEvent: (event: ExecutionEvent) => void;
  addPendingGate: (gate: CodingGateRequired) => void;
  resolvePendingGate: (gateId: string) => void;
  appendStreamChunk: (content: string, nodeId?: string | null) => void;
  completeStream: (nodeId?: string | null) => void;
  setConnectionStatus: (status: CodingConnectionStatus) => void;
  setProtocolError: (error: CodingProtocolError | null) => void;
  setSelectedNode: (nodeId: string | null) => void;
  setActiveTab: (tab: CodingArtifactTab, lockedByUser?: boolean) => void;
  reset: () => void;
}

const initialState: CodingWorkspaceState = {
  attemptId: null,
  workItemId: null,
  issueId: null,
  projectId: null,
  status: null,
  stage: null,
  branchName: null,
  baseBranch: null,
  worktreePath: null,
  reworkCount: 0,
  maxAutoRework: 0,
  headCommit: null,
  pushedRemote: null,
  providerConfigSnapshot: null,
  timelineNodes: [],
  activeNodeId: null,
  selectedNodeId: null,
  chatEntries: [],
  streamingContent: null,
  activeStreamNodeId: null,
  activeTab: "diff",
  diffSummary: null,
  testingReport: null,
  codeReviewReports: [],
  internalPrReview: null,
  reviewRequest: null,
  logs: [],
  connectionStatus: "disconnected",
  pendingGates: [],
  protocolError: null,
  tabLockedByUser: false,
};

export const useCodingWorkspaceStore = create<
  CodingWorkspaceState & CodingWorkspaceActions
>((set, get) => ({
  ...initialState,

  setSessionState: (snapshot) =>
    set((prev) => {
      const selectedNodeId =
        snapshot.active_node_id ??
        preserveSelectedNode(prev.selectedNodeId, snapshot.timeline_nodes) ??
        snapshot.timeline_nodes.at(-1)?.id ??
        null;
      const selectedNode = snapshot.timeline_nodes.find((node) => node.id === selectedNodeId);
      const nextTab = selectedNode ? stageToArtifactTab(selectedNode.stage) : null;
      return {
        attemptId: snapshot.attempt_id,
        status: snapshot.status,
        stage: snapshot.stage,
        branchName: snapshot.branch_name,
        baseBranch: snapshot.base_branch,
        worktreePath: snapshot.worktree_path,
        reworkCount: snapshot.rework_count,
        maxAutoRework: snapshot.max_auto_rework,
        headCommit: snapshot.head_commit,
        pushedRemote: snapshot.pushed_remote,
        providerConfigSnapshot: snapshot.provider_config_snapshot,
        timelineNodes: snapshot.timeline_nodes,
        activeNodeId: snapshot.active_node_id,
        selectedNodeId,
        testingReport: snapshot.testing_report,
        codeReviewReports: snapshot.code_review_reports,
        reviewRequest: snapshot.review_request,
        internalPrReview: snapshot.internal_pr_review,
        pendingGates: snapshot.pending_gates,
        protocolError: null,
        streamingContent: null,
        activeStreamNodeId: null,
        ...(!prev.tabLockedByUser && nextTab ? { activeTab: nextTab } : {}),
      };
    }),

  updateStage: (stage) => set({ stage }),

  addTimelineNode: (node) =>
    set((state) => {
      const timelineNodes = upsertById(state.timelineNodes, node);
      return {
        timelineNodes,
        activeNodeId: isActiveNodeStatus(node.status) ? node.id : state.activeNodeId,
        selectedNodeId: state.selectedNodeId ?? node.id,
      };
    }),

  updateTimelineNode: (nodeId, status, summary, completedAt) =>
    set((state) => ({
      timelineNodes: state.timelineNodes.map((node) =>
        node.id === nodeId
          ? {
              ...node,
              status,
              summary: summary ?? node.summary,
              completed_at: completedAt ?? node.completed_at,
            }
          : node,
      ),
      activeNodeId:
        state.activeNodeId === nodeId && !isActiveNodeStatus(status)
          ? null
          : state.activeNodeId,
    })),

  setTestingReport: (testingReport) => set({ testingReport }),

  addCodeReviewReport: (report) =>
    set((state) => ({
      codeReviewReports: upsertById(state.codeReviewReports, report),
    })),

  setReviewRequest: (reviewRequest) => set({ reviewRequest }),

  setInternalPrReview: (internalPrReview) => set({ internalPrReview }),

  addExecutionEvent: (event) =>
    set((state) => {
      const timestamp = new Date().toISOString();
      return {
        logs: upsertById(state.logs, {
          id: event.event_id,
          message: executionEventMessage(event),
          timestamp,
          nodeId: event.node_id ?? null,
        }),
        chatEntries: upsertChatEntry(state.chatEntries, {
          id: event.event_id,
          type: "execution_event",
          role: "system",
          content: event.title,
          timestamp,
          node_id: event.node_id ?? undefined,
          metadata: event as unknown as Record<string, unknown>,
        }),
      };
    }),

  addPendingGate: (gate) =>
    set((state) => ({
      pendingGates: upsertByKey(state.pendingGates, gate, "gate_id"),
    })),

  resolvePendingGate: (gateId) =>
    set((state) => ({
      pendingGates: state.pendingGates.filter((gate) => gate.gate_id !== gateId),
    })),

  appendStreamChunk: (content, nodeId = null) =>
    set((state) => {
      const streamingContent = `${state.streamingContent ?? ""}${content}`;
      const entryId = streamEntryId(nodeId);
      const entry: ChatEntry = {
        id: entryId,
        type: "provider_stream",
        role: "author",
        content: streamingContent,
        timestamp: new Date().toISOString(),
        node_id: nodeId ?? undefined,
      };
      return {
        streamingContent,
        activeStreamNodeId: nodeId ?? null,
        chatEntries: upsertChatEntry(state.chatEntries, entry),
      };
    }),

  completeStream: () =>
    set({
      streamingContent: null,
      activeStreamNodeId: null,
    }),

  setConnectionStatus: (connectionStatus) => set({ connectionStatus }),

  setProtocolError: (protocolError) => set({ protocolError }),

  setSelectedNode: (selectedNodeId) =>
    set((state) => {
      const selectedNode = state.timelineNodes.find((node) => node.id === selectedNodeId);
      const nextTab = selectedNode ? stageToArtifactTab(selectedNode.stage) : null;

      return {
        selectedNodeId,
        ...(!state.tabLockedByUser && nextTab ? { activeTab: nextTab } : {}),
      };
    }),

  setActiveTab: (activeTab, lockedByUser = true) =>
    set({ activeTab, tabLockedByUser: lockedByUser }),

  reset: () => set({ ...initialState }),
}));

function isActiveNodeStatus(status: CodingTimelineNodeStatus) {
  return status === "pending" || status === "running" || status === "blocked";
}

function preserveSelectedNode(
  selectedNodeId: string | null,
  nodes: CodingTimelineNode[],
) {
  if (!selectedNodeId) return null;
  return nodes.some((node) => node.id === selectedNodeId) ? selectedNodeId : null;
}

function stageToArtifactTab(stage: CodingExecutionStage): CodingArtifactTab | null {
  switch (stage) {
    case "worktree_prepare":
    case "review_request":
      return "git";
    case "coding":
    case "rework":
      return "diff";
    case "testing":
      return "tests";
    case "code_review":
    case "internal_pr_review":
      return "review";
    case "prepare_context":
    case "final_confirm":
      return null;
  }
}

function upsertById<T extends { id: string }>(items: T[], item: T): T[] {
  const index = items.findIndex((existing) => existing.id === item.id);
  if (index === -1) return [...items, item];
  return items.map((existing, currentIndex) => (currentIndex === index ? item : existing));
}

function upsertByKey<T extends Record<K, string>, K extends keyof T>(
  items: T[],
  item: T,
  key: K,
): T[] {
  const index = items.findIndex((existing) => existing[key] === item[key]);
  if (index === -1) return [...items, item];
  return items.map((existing, currentIndex) => (currentIndex === index ? item : existing));
}

function streamEntryId(nodeId?: string | null) {
  return `coding_stream_${nodeId ?? "global"}`;
}

function executionEventMessage(event: ExecutionEvent) {
  return event.output ?? event.detail ?? event.command ?? event.title;
}

function upsertChatEntry(entries: ChatEntry[], entry: ChatEntry): ChatEntry[] {
  const index = entries.findIndex((existing) => existing.id === entry.id);
  if (index === -1) return [...entries, entry];
  return entries.map((existing, currentIndex) => (currentIndex === index ? entry : existing));
}
