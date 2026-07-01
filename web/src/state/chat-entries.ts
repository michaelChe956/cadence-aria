export type ChatEntryType =
  | "context_note"
  | "start_generation"
  | "provider_stream"
  | "execution_event"
  | "permission_request"
  | "permission_response"
  | "choice_request"
  | "choice_response"
  | "artifact_update"
  | "review_verdict"
  | "analyst_verdict"
  | "gate_prompt"
  | "human_decision"
  | "stage_change"
  | "error";

export type ChatEntryRole =
  | "user"
  | "author"
  | "reviewer"
  | "coder"
  | "tester"
  | "analyst"
  | "code_reviewer"
  | "internal_reviewer"
  | "system";
export type ChatEntryResolution = "confirm" | "request-change" | "terminate";

export interface ChoiceResponsePayload {
  selected_option_ids: string[];
  free_text: string | null;
  answers?: ChoiceAnswerPayload[];
}

export interface ChoiceAnswerPayload {
  question_id: string;
  selected_option_ids: string[];
  free_text: string | null;
}

export type WorkspaceContentRef =
  | { kind: "node_stream"; nodeId: string }
  | { kind: "provider_prompt"; nodeId: string }
  | { kind: "execution_output"; nodeId: string; eventId: string }
  | { kind: "artifact_version"; version: number; sourceNodeId?: string };

export interface ChatEntry {
  id: string;
  type: ChatEntryType;
  role: ChatEntryRole;
  content: string;
  timestamp: string;
  node_id?: string;
  metadata?: Record<string, unknown>;
  content_ref?: WorkspaceContentRef;
  content_size?: number;
  has_full_content?: boolean;
  resolved?: boolean;
  resolution?: ChatEntryResolution;
}
