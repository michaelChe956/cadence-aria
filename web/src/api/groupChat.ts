import type { ApiError } from "./types";
import type { WorkspaceProviderName } from "./types";
import { ApiRequestError, normalizeApiError } from "./client";

export type GroupChatProviderName = WorkspaceProviderName;
export type GroupChatRoleKey =
  | "author"
  | "frontend_design"
  | "backend_design"
  | "reviewer"
  | "researcher";
export type GroupChatPermissionMode = "auto" | "supervised";
export type DraftSlotKey = string;
export type ArtifactLineKind =
  | "issue_refinement"
  | "story_spec"
  | "design_spec";
export type GroupChatSessionStatus = "active" | "finalized" | "archived";

export type RoleInstance = {
  id: string;
  role_key: GroupChatRoleKey;
  provider: GroupChatProviderName;
  display_name: string;
  permission_mode: GroupChatPermissionMode;
  seen_cursor: number;
  injection_watermark: number;
};

export type ArtifactDraft = {
  version: number;
  markdown: string;
  author_role_id: string;
  based_on_events: number;
};

export type DraftClaim = {
  holder_role_id: string;
  claimed_at: string;
};

export type DraftSlot = {
  slot_key: DraftSlotKey;
  current: ArtifactDraft | null;
  claim: DraftClaim | null;
};

export type ArtifactLine = {
  kind: ArtifactLineKind;
  drafts: DraftSlot[];
  finalized_versions: string[];
  entity_id: string | null;
  bridge_session_id: string | null;
};

export type ArtifactRef = {
  line: ArtifactLineKind;
  slot: DraftSlotKey;
  version: number;
};

export type RoomEvent =
  | { type: "user_message"; text: string; mentions: string[] }
  | {
      type: "agent_message";
      role_instance_id: string;
      text: string;
      artifact_ref: ArtifactRef | null;
      cursor_after: number;
    }
  | {
      type: "claim_event";
      role_instance_id: string;
      line: ArtifactLineKind;
      slot_key: DraftSlotKey;
      claimed: boolean;
    }
  | {
      type: "held_event";
      role_instance_id: string;
      reason: string;
      cursor_after: number;
    }
  | {
      type: "finalize_event";
      artifact_line: ArtifactLineKind;
      version: string;
      included_slots: DraftSlotKey[];
    }
  | { type: "system_notice"; text: string };

export type GroupChatSession = {
  id: string;
  project_id: string;
  issue_id: string;
  status: GroupChatSessionStatus;
  roles: RoleInstance[];
  artifact_lines: ArtifactLine[];
  triage_provider?: GroupChatProviderName;
  created_at: string;
  updated_at: string;
};

export type TimelineEvent = { seq: number; event: RoomEvent };

export type GroupChatSessionResponse = GroupChatSession & {
  timeline: TimelineEvent[];
};

export type CoordinatorRunSummary = {
  appended_seqs: number[];
  held_events: number;
  circuit_break: boolean;
  no_one_notice: boolean;
};

export type SendMessageResponse = {
  summary: CoordinatorRunSummary;
  session: GroupChatSession;
};

export type FinalizeResponse = {
  event: RoomEvent;
  session: GroupChatSession;
};

export type TriageProviderResponse = {
  provider: GroupChatProviderName | null;
};

export type SpecGenerationMode = "pipeline" | "group_chat";

export type CreateGroupChatSessionRequest = {
  project_id: string;
  issue_id: string;
};

export type SendGroupChatMessageRequest = {
  text: string;
  mentions?: string[];
  draft_slot?: DraftSlotKey | null;
};

export type AddGroupChatRoleRequest = {
  role_key: GroupChatRoleKey;
  provider: GroupChatProviderName;
  display_name?: string | null;
  permission_mode?: GroupChatPermissionMode | null;
};

export type FinalizeGroupChatRequest = {
  line_kind: ArtifactLineKind;
  included_slots_override?: DraftSlotKey[] | null;
  confirmed_by?: string | null;
};

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    throw new ApiRequestError(await normalizeApiError(response));
  }
  const text = await response.text();
  if (!text.trim()) {
    return undefined as T;
  }
  return JSON.parse(text) as T;
}

function sessionPath(sessionId: string, suffix = ""): string {
  return `/api/group-chat/sessions/${encodeURIComponent(sessionId)}${suffix}`;
}

export function createGroupChatSession(
  payload: CreateGroupChatSessionRequest,
): Promise<GroupChatSessionResponse> {
  return requestJson<GroupChatSessionResponse>("/api/group-chat/sessions", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getGroupChatSession(
  sessionId: string,
  options: { afterSeq?: number; limit?: number } = {},
): Promise<GroupChatSessionResponse> {
  const query = new URLSearchParams();
  if (options.afterSeq !== undefined) query.set("after_seq", String(options.afterSeq));
  if (options.limit !== undefined) query.set("limit", String(options.limit));
  const suffix = query.toString() ? `?${query.toString()}` : "";
  return requestJson<GroupChatSessionResponse>(sessionPath(sessionId, suffix));
}

export function sendGroupChatMessage(
  sessionId: string,
  payload: SendGroupChatMessageRequest,
): Promise<SendMessageResponse> {
  return requestJson<SendMessageResponse>(sessionPath(sessionId, "/messages"), {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function addGroupChatRole(
  sessionId: string,
  payload: AddGroupChatRoleRequest,
): Promise<GroupChatSession> {
  return requestJson<GroupChatSession>(sessionPath(sessionId, "/roles"), {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function finalizeGroupChat(
  sessionId: string,
  payload: FinalizeGroupChatRequest,
): Promise<FinalizeResponse> {
  return requestJson<FinalizeResponse>(sessionPath(sessionId, "/finalize"), {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getGroupChatTriageProvider(
  sessionId: string,
): Promise<TriageProviderResponse> {
  return requestJson<TriageProviderResponse>(
    sessionPath(sessionId, "/settings/triage-provider"),
  );
}

export function updateGroupChatTriageProvider(
  sessionId: string,
  provider: GroupChatProviderName | null,
): Promise<TriageProviderResponse> {
  return requestJson<TriageProviderResponse>(
    sessionPath(sessionId, "/settings/triage-provider"),
    { method: "PUT", body: JSON.stringify({ provider }) },
  );
}

export function getSpecGenerationMode(): Promise<SpecGenerationMode> {
  return requestJson<SpecGenerationMode>("/api/settings/spec-generation-mode");
}

export function updateSpecGenerationMode(
  mode: SpecGenerationMode,
): Promise<SpecGenerationMode> {
  return requestJson<SpecGenerationMode>("/api/settings/spec-generation-mode", {
    method: "PUT",
    body: JSON.stringify(mode),
  });
}

export type GroupChatWsInMessage =
  | {
      type: "send_message";
      text: string;
      mentions?: string[];
      draft_slot?: DraftSlotKey | null;
    }
  | {
      type: "add_role";
      role_key: GroupChatRoleKey;
      provider: GroupChatProviderName;
      display_name?: string | null;
      permission_mode?: GroupChatPermissionMode | null;
    }
  | {
      type: "finalize";
      line_kind: ArtifactLineKind;
      included_slots?: DraftSlotKey[] | null;
      confirmed_by?: string | null;
    }
  | { type: "ping" };

export type GroupChatWsOutMessage =
  | { type: "room_event"; seq: number; event: RoomEvent }
  | { type: "turn_started"; role_instance_id: string }
  | { type: "turn_delta"; role_instance_id: string; delta: string }
  | { type: "turn_held"; role_instance_id: string; reason: string }
  | { type: "error"; code: string; message: string }
  | { type: "pong" };

export type GroupChatApiError = ApiError;
