import type { RevisionPath } from "../../api/types";
import type { ChatEntry, ChoiceResponsePayload } from "../../state/chat-entries";
import { ChatEntryContainer } from "./ChatEntryContainer";
import { ChatEntryRenderer } from "./ChatEntryRenderer";
import { InlineEventRow } from "./InlineEventRow";
import { ChoiceRequestEntry } from "./entries/ChoiceRequestEntry";
import { PermissionRequestEntry } from "./entries/PermissionRequestEntry";
import { MarkdownContent } from "./entries/ProviderStreamEntry";
import type { MessageGroup } from "./message-grouping";

interface MessageGroupViewProps {
  group: MessageGroup;
  onPermissionResponse?: (entry: ChatEntry, approved: boolean) => void;
  onChoiceResponse?: (entry: ChatEntry, response: ChoiceResponsePayload) => void;
  onSelectRevisionPath?: (path: RevisionPath) => void;
  onHumanConfirm?: (decision: "confirm" | "request-change" | "terminate") => void;
}

export function MessageGroupView({
  group,
  onPermissionResponse,
  onChoiceResponse,
  onSelectRevisionPath,
  onHumanConfirm,
}: MessageGroupViewProps) {
  return (
    <ChatEntryContainer
      role={group.role}
      title={group.role === "reviewer" ? "审核者" : "作者"}
      testId="message-group"
    >
      <div className="space-y-3">
        {group.primaryEntry ? <MarkdownContent content={group.primaryEntry.content} /> : null}
        {group.inlineEvents.length > 0 ? (
          <div className="space-y-2">
            {group.inlineEvents.map((entry) => (
              <InlineEventRow key={entry.id} entry={entry} />
            ))}
          </div>
        ) : null}
        {group.interruptEntries.length > 0 ? (
          <div className="space-y-2">
            {group.interruptEntries.map((entry) =>
              entry.type === "permission_request" ? (
                <PermissionRequestEntry
                  key={entry.id}
                  entry={entry}
                  onRespond={onPermissionResponse}
                  embedded
                />
              ) : entry.type === "choice_request" ? (
                <ChoiceRequestEntry
                  key={entry.id}
                  entry={entry}
                  onRespond={onChoiceResponse}
                  embedded
                />
              ) : (
                <ChatEntryRenderer
                  key={entry.id}
                  entry={entry}
                  onPermissionResponse={onPermissionResponse}
                  onChoiceResponse={onChoiceResponse}
                  onSelectRevisionPath={onSelectRevisionPath}
                  onHumanConfirm={onHumanConfirm}
                />
              ),
            )}
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}
