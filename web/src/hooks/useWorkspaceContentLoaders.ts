import { useCallback } from "react";
import { fetchWorkspaceEventOutput, fetchWorkspacePrompt } from "../api/workspace-content";
import type { WorkspaceContentRef } from "../state/chat-entries";
import { useWorkspaceStore } from "../state/workspace-ws-store";

export function useWorkspaceContentLoaders(sessionId: string) {
  const loadContent = useCallback(
    async (currentSessionId: string, ref: WorkspaceContentRef) => {
      if (ref.kind === "execution_output") {
        const response = await fetchWorkspaceEventOutput(
          currentSessionId,
          ref.nodeId,
          ref.eventId,
        );
        return response.output;
      }
      if (ref.kind === "provider_prompt") {
        const response = await fetchWorkspacePrompt(currentSessionId, ref.nodeId);
        return response.prompt;
      }
      throw new Error("不支持加载该内容类型");
    },
    [],
  );

  const cacheContent = useCallback(
    (key: string, value: string) => {
      const state = useWorkspaceStore.getState();
      if (state.sessionId !== sessionId) {
        return;
      }
      state.setContentCacheEntry(key, value);
    },
    [sessionId],
  );

  return { loadContent, cacheContent };
}
