import type { WorkspaceProviderName } from "../api/types";
import { buildChatEntries } from "./workspace-chat-rebuild";
import { refreshPreparedContextAuthorGuidance } from "./workspace-ws-store-guidance";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

export function setProviderSelection(
  prev: WorkspaceWsState,
  role: "author" | "reviewer",
  provider: WorkspaceProviderName,
) {
  const current = prev.providers ?? { author: "claude_code", reviewer: "codex" };
  const providers =
    role === "author"
      ? { ...current, author: provider }
      : { ...current, reviewer: provider };
  const messages =
    role === "author"
      ? refreshPreparedContextAuthorGuidance(prev.messages, provider)
      : prev.messages;
  const nextState = { ...prev, providers, messages };
  return {
    providers,
    messages,
    chatEntries:
      messages !== prev.messages ? buildChatEntries(nextState) : prev.chatEntries,
  };
}
