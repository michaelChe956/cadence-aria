import type { ChatEntry, ChatEntryRole } from "./chat-entries";
import { chatEntryId, providerEntryMetadata } from "./workspace-chat-rebuild";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

export function appendBufferedStreamChunk(
  prev: WorkspaceWsState,
  content: string,
  nodeId: string,
  role: ChatEntryRole,
) {
  const existing = prev.streamBuffers[nodeId] ?? { chunks: [], visibleText: "", role };
  return {
    streamBuffers: {
      ...prev.streamBuffers,
      [nodeId]: {
        ...existing,
        role,
        chunks: [...existing.chunks, content],
      },
    },
  };
}

export function flushBufferedStream(prev: WorkspaceWsState, nodeId: string) {
  const buffer = prev.streamBuffers[nodeId];
  if (!buffer || buffer.chunks.length === 0) {
    return {};
  }
  const appended = buffer.chunks.join("");
  const visibleText = buffer.visibleText + appended;
  const entryId = chatEntryId(nodeId, "stream-active");
  const index = prev.chatEntries.findIndex((entry) => entry.id === entryId);
  const timelineNode = prev.timelineNodes.find((candidate) => candidate.node_id === nodeId);
  const provider = timelineNode?.agent ?? prev.nodeDetails[nodeId]?.provider?.name ?? null;
  const entry: ChatEntry = {
    id: entryId,
    type: "provider_stream",
    role: buffer.role,
    content: visibleText,
    timestamp: new Date().toISOString(),
    node_id: nodeId,
    content_ref: { kind: "node_stream", nodeId },
    metadata: providerEntryMetadata(timelineNode, provider),
  };
  const chatEntries = index === -1 ? [...prev.chatEntries, entry] : [...prev.chatEntries];
  if (index !== -1) {
    chatEntries[index] = entry;
  }
  return {
    chatEntries,
    streamBuffers: {
      ...prev.streamBuffers,
      [nodeId]: { ...buffer, chunks: [], visibleText },
    },
    activeStreamEntryId: entryId,
  };
}

export function clearBufferedStream(prev: WorkspaceWsState, nodeId: string) {
  if (!prev.streamBuffers[nodeId]) {
    return {};
  }
  const { [nodeId]: _removed, ...streamBuffers } = prev.streamBuffers;
  return { streamBuffers };
}
