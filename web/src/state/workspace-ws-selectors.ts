import type { WorkspaceContentRef } from "./chat-entries";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

export const selectWorkspaceHeaderState = (state: WorkspaceWsState) => ({
  sessionId: state.sessionId,
  workspaceType: state.workspaceType,
  providers: state.providers,
  reviewRounds: state.reviewRounds,
  stage: state.stage,
  providerLocked: state.providerLocked,
  providerLockedAt: state.providerLockedAt,
  superpowersEnabled: state.superpowersEnabled,
  openSpecEnabled: state.openSpecEnabled,
});

export function workspaceContentCacheKey(ref: WorkspaceContentRef) {
  if (ref.kind === "provider_prompt") {
    return `provider_prompt:${ref.nodeId}`;
  }
  if (ref.kind === "execution_output") {
    return `execution_output:${ref.nodeId}:${ref.eventId}`;
  }
  if (ref.kind === "node_stream") {
    return `node_stream:${ref.nodeId}`;
  }
  return null;
}

export const selectChatPanelState = (state: WorkspaceWsState) => ({
  chatEntries: state.chatEntries,
  stage: state.stage,
  selectedNodeId: state.selectedNodeId,
});

export function selectPrepareContextNotes(state: WorkspaceWsState) {
  return state.timelineNodes
    .filter((node) => node.node_type === "context_note")
    .map((node) => {
      const detailContent = state.nodeDetails[node.node_id]?.streaming_content;
      return detailContent && detailContent.trim().length > 0
        ? detailContent
        : node.summary ?? "";
    })
    .filter((content) => content.trim().length > 0);
}

