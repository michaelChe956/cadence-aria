import type { ChatEntry } from "../../state/chat-entries";

export interface MessageGroup {
  id: string;
  nodeId?: string;
  role: ChatEntry["role"];
  primaryEntry?: ChatEntry;
  inlineEvents: ChatEntry[];
  interruptEntries: ChatEntry[];
}

export type GroupedItem =
  | { kind: "group"; group: MessageGroup }
  | { kind: "entry"; entry: ChatEntry };

const STANDALONE_ENTRY_TYPES = new Set<string>([
  "permission_response",
  "choice_response",
  "artifact_update",
  "review_verdict",
  "analyst_verdict",
  "stage_change",
  "human_decision",
  "start_generation",
  "context_note",
  "error",
]);

const INTERRUPT_ENTRY_TYPES = new Set<string>([
  "permission_request",
  "gate_prompt",
  "choice_request",
]);

export function groupEntries(entries: ChatEntry[]): GroupedItem[] {
  const result: GroupedItem[] = [];
  let currentGroup: MessageGroup | null = null;
  let currentGroupKey: string | null = null;
  let groupIndex = 0;

  function flushGroup() {
    if (currentGroup) {
      result.push({ kind: "group", group: currentGroup });
      currentGroup = null;
      currentGroupKey = null;
    }
  }

  for (const entry of entries) {
    const type = entry.type as string;
    if (STANDALONE_ENTRY_TYPES.has(type) || !isGroupableEntry(type)) {
      flushGroup();
      result.push({ kind: "entry", entry });
      continue;
    }

    const nodeKey = entry.node_id ?? "global";
    if (!currentGroup || currentGroupKey !== nodeKey) {
      flushGroup();
      groupIndex += 1;
      currentGroupKey = nodeKey;
      currentGroup = {
        id: `group:${groupIndex}:${entry.id}`,
        nodeId: entry.node_id,
        role: roleForEntry(entry),
        inlineEvents: [],
        interruptEntries: [],
      };
    }

    if (type === "provider_stream") {
      currentGroup.primaryEntry = entry;
      currentGroup.role = roleForEntry(entry);
    } else if (type === "execution_event") {
      currentGroup.inlineEvents.push(entry);
    } else if (INTERRUPT_ENTRY_TYPES.has(type)) {
      currentGroup.interruptEntries.push(entry);
    }
  }

  flushGroup();
  return result;
}

function isGroupableEntry(type: string) {
  return type === "provider_stream" || type === "execution_event" || INTERRUPT_ENTRY_TYPES.has(type);
}

function roleForEntry(entry: ChatEntry): ChatEntry["role"] {
  return entry.role;
}
