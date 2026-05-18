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

export interface PermissionRequest {
  id: string;
  tool_name: string;
  description: string;
  risk_level: "low" | "medium" | "high";
}

export interface ExecutionEvent {
  event_id: string;
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

export interface WorkspaceWsState {
  sessionId: string | null;
  workspaceType: string | null;
  stage: string;
  messages: WsMessage[];
  checkpoints: WsCheckpoint[];
  artifact: string | null;
  providers: WsProviderConfig | null;
  connectionStatus: WsConnectionStatus;
  streamingContent: string;
  pendingPermissions: PermissionRequest[];
  providerStatus: ProviderStatus;
  executionEvents: ExecutionEvent[];
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
  }) => void;
  appendStreamChunk: (content: string) => void;
  completeMessage: (messageId: string, checkpointId: string) => void;
  setStage: (stage: string) => void;
  setArtifact: (markdown: string) => void;
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
  messages: [],
  checkpoints: [],
  artifact: null,
  providers: null,
  connectionStatus: "disconnected",
  streamingContent: "",
  pendingPermissions: [],
  providerStatus: "starting",
  executionEvents: [],
  error: null,
};

export const useWorkspaceStore = create<WorkspaceWsState & WorkspaceWsActions>((set) => ({
  ...initialState,

  setSessionState: (state) =>
    set({
      sessionId: state.session_id,
      workspaceType: state.workspace_type,
      stage: state.stage,
      messages: state.messages,
      checkpoints: state.checkpoints,
      artifact: state.artifact,
      providers: state.providers,
      streamingContent: "",
      pendingPermissions: [],
      providerStatus: "starting",
      executionEvents: [],
      error: null,
    }),

  appendStreamChunk: (content) =>
    set((prev) => ({ streamingContent: prev.streamingContent + content })),

  completeMessage: (messageId, checkpointId) =>
    set((prev) => {
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
      streamingContent:
        stage === "running" || stage === "cross_review" ? prev.streamingContent : "",
    })),

  setArtifact: (markdown) => set({ artifact: markdown }),

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
