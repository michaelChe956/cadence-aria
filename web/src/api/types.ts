export type ApiError = {
  code: string;
  message: string;
  details: Record<string, unknown>;
};

export type CreateTaskRequest = {
  request_text: string;
  change_id: string;
  policy_preset: string;
  provider_mode: string;
  timeout_secs: number;
};

export type CreateTaskResponse = {
  task_id: string;
  session_id: string;
  change_id: string;
  phase: string;
};

export type PendingProviderStep = {
  node_id: string;
  provider_type: string;
  runtime_role: string;
  adapter_role: string;
  prompt: string;
  input_summary: unknown;
  canonical_input_refs: string[];
  context_files: string[];
  output_schema: string;
  allowed_write_scope: string[];
  forbidden_actions: string[];
  verification_commands: string[];
  checkpoint_id: string;
};

export type WebWorkspaceProjection = {
  workspace_root: string;
  active_task_id: string | null;
  active_session_id: string | null;
  overview: Record<string, unknown>;
  sessions: unknown[];
  timeline: Array<Record<string, unknown>>;
  artifact_index: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
  available_actions: string[];
  pending_provider_step: PendingProviderStep | null;
  selected_node_context: {
    node_id: string | null;
    overview: Record<string, unknown>;
    inputs: unknown[];
    run: unknown[];
    outputs: unknown[];
    diffs: unknown[];
  };
  git_summary: {
    workspace_path: string;
    branch: string | null;
    head: string | null;
    dirty: boolean;
    dirty_files: string[];
  };
  event_cursor: number;
};

export type WebEvent = {
  cursor: number;
  event_type: string;
  task_id: string | null;
  payload: unknown;
};

export type TaskListResponse = {
  tasks: Array<{
    task_id: string;
    change_id: string | null;
    phase: string | null;
    updated_at?: string | null;
  }>;
};

export type ArtifactContentResponse = {
  artifact_ref: string;
  artifact_kind: string;
  producer_node: string | null;
  path: string;
  content_type: "markdown" | "json" | "source" | "test" | "log" | "unknown";
  content: string;
};

export type FileContentResponse = {
  path: string;
  content_type: string;
  content: string;
};

export type FileDiffResponse = {
  base_checkpoint: string;
  path: string;
  diff: string;
};

export type ProviderOutputChunk = {
  node_id: string;
  provider_run_id: string;
  stream: "stdout" | "stderr";
  text: string;
  structured_output?: unknown;
  manual_gate?: string;
  retry_attempt?: number;
};

export type StopTaskResponse = {
  status: string;
  task_id: string;
};

export type RollbackPreviewResponse = {
  checkpoint_id: string;
  git_head: string | null;
  dirty: boolean;
  turns_to_drop: number;
  node_runs_to_drop: number;
  provider_runs_to_drop: number;
  artifacts_to_drop: number;
  files_may_change: string[];
};
