import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function UserContextEntry({ entry }: { entry: ChatEntry }) {
  return (
    <ChatEntryContainer role="user" title="你" className="border-blue-200">
      <div className="whitespace-pre-wrap text-sm text-[var(--aria-ink)]">{entry.content}</div>
    </ChatEntryContainer>
  );
}
