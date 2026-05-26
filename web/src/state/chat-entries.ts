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
  | "gate_prompt"
  | "human_decision"
  | "stage_change"
  | "error";

export type ChatEntryRole = "user" | "author" | "reviewer" | "system";
export type ChatEntryResolution = "confirm" | "request-change" | "terminate";

export interface ChoiceResponsePayload {
  selected_option_ids: string[];
  free_text: string | null;
}

export interface ChatEntry {
  id: string;
  type: ChatEntryType;
  role: ChatEntryRole;
  content: string;
  timestamp: string;
  node_id?: string;
  metadata?: Record<string, unknown>;
  resolved?: boolean;
  resolution?: ChatEntryResolution;
}
