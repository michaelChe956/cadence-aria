import type { CodingChatEntry } from "../api/types";
import type { ChatEntry, ChatEntryRole, ChatEntryType } from "./chat-entries";

export function codingChatEntryToChatEntry(entry: CodingChatEntry): ChatEntry {
  return {
    id: entry.id,
    type: chatEntryTypeFromCodingEntry(entry),
    role: chatEntryRoleFromCodingEntry(entry),
    content: chatEntryContentFromCodingEntry(entry),
    timestamp: entry.created_at,
    node_id: entry.node_id ?? undefined,
    metadata: chatEntryMetadataFromCodingEntry(entry),
  };
}

function chatEntryTypeFromCodingEntry(entry: CodingChatEntry): ChatEntryType {
  switch (entry.entry_type.type) {
    case "user_message":
      return "context_note";
    case "assistant_message":
      return "provider_stream";
    case "stage_gate":
      return "gate_prompt";
    case "analyst_verdict":
      return "analyst_verdict";
    case "stage_summary":
      return "stage_change";
    case "tool_call":
    case "tool_result":
    case "system_event":
      return "execution_event";
  }
}

function chatEntryRoleFromCodingEntry(entry: CodingChatEntry): ChatEntryRole {
  const source = chatEntrySource(entry);
  if (entry.entry_type.type === "user_message") return "user";
  if (entry.entry_type.type === "analyst_verdict") return "analyst";
  if (source === "internal_pr_review") return "internal_reviewer";
  if (source === "code_review") return "code_reviewer";
  switch (entry.role) {
    case "author":
      return "coder";
    case "reviewer":
      return "code_reviewer";
    case "tester":
      return "tester";
    case "git":
    case "system":
      return "system";
  }
}

function chatEntryContentFromCodingEntry(entry: CodingChatEntry): string {
  if (entry.content) return entry.content;
  switch (entry.entry_type.type) {
    case "tool_call":
      return entry.entry_type.tool_name;
    case "tool_result":
      return entry.entry_type.output;
    case "system_event":
      return entry.entry_type.message;
    case "analyst_verdict":
      return entry.entry_type.verdict;
    case "stage_gate":
      return entry.entry_type.stage;
    case "stage_summary":
      return entry.entry_type.summary;
    default:
      return "";
  }
}

function chatEntryMetadataFromCodingEntry(
  entry: CodingChatEntry,
): Record<string, unknown> | undefined {
  const metadata: Record<string, unknown> = { ...(entry.metadata ?? {}) };
  switch (entry.entry_type.type) {
    case "tool_call":
      metadata.tool_name = entry.entry_type.tool_name;
      metadata.input = entry.entry_type.input;
      break;
    case "tool_result":
      metadata.tool_use_id = entry.entry_type.tool_use_id;
      metadata.output = entry.entry_type.output;
      metadata.is_error = entry.entry_type.is_error;
      break;
    case "analyst_verdict":
      metadata.verdict = entry.entry_type.verdict;
      break;
    case "system_event":
      metadata.event_type = entry.entry_type.event_type;
      metadata.message = entry.entry_type.message;
      break;
    case "stage_gate":
      metadata.stage = entry.entry_type.stage;
      metadata.countdown_seconds = entry.entry_type.countdown_seconds;
      break;
    case "stage_summary":
      metadata.stage = entry.entry_type.stage;
      metadata.summary = entry.entry_type.summary;
      break;
    case "user_message":
    case "assistant_message":
      break;
  }
  return Object.keys(metadata).length > 0 ? metadata : undefined;
}

function chatEntrySource(entry: CodingChatEntry): string | null {
  const source = entry.metadata?.source;
  return typeof source === "string" ? source : null;
}
