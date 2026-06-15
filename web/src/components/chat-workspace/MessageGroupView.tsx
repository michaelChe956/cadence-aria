import type { RevisionPath } from "../../api/types";
import type { ChatEntry, ChoiceResponsePayload, WorkspaceContentRef } from "../../state/chat-entries";
import { ChatEntryContainer } from "./ChatEntryContainer";
import { ChatEntryRenderer } from "./ChatEntryRenderer";
import { InlineEventRow } from "./InlineEventRow";
import { ChoiceRequestEntry } from "./entries/ChoiceRequestEntry";
import { PermissionRequestEntry } from "./entries/PermissionRequestEntry";
import {
  MarkdownContent,
  normalizeProviderStreamEntryContent,
} from "./entries/ProviderStreamEntry";
import type { MessageGroup } from "./message-grouping";

interface MessageGroupViewProps {
  group: MessageGroup;
  onPermissionResponse?: (entry: ChatEntry, approved: boolean) => void;
  onChoiceResponse?: (entry: ChatEntry, response: ChoiceResponsePayload) => void;
  onSelectRevisionPath?: (path: RevisionPath, extraContext?: string) => void;
  onHumanConfirm?: (
    decision: "confirm" | "request-change" | "terminate",
    payload?: unknown,
  ) => void;
  sessionId?: string | null;
  contentCache?: Record<string, string>;
  loadContent?: (sessionId: string, ref: WorkspaceContentRef) => Promise<string>;
  onCacheContent?: (key: string, value: string) => void;
}

export function MessageGroupView({
  group,
  onPermissionResponse,
  onChoiceResponse,
  onSelectRevisionPath,
  onHumanConfirm,
  sessionId,
  contentCache,
  loadContent,
  onCacheContent,
}: MessageGroupViewProps) {
  return (
    <ChatEntryContainer
      role={group.role}
      title={groupTitle(group)}
      testId="message-group"
    >
      <div className="space-y-3">
        {group.primaryEntry ? (
          <MarkdownContent content={normalizeProviderStreamEntryContent(group.primaryEntry)} />
        ) : null}
        {group.inlineEvents.length > 0 ? (
          <div className="space-y-2">
            {group.inlineEvents.map((entry) => (
              <InlineEventRow
                key={entry.id}
                entry={entry}
                sessionId={sessionId}
                contentCache={contentCache}
                loadContent={loadContent}
                onCacheContent={onCacheContent}
              />
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

const PROVIDER_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  fake: "Fake",
};

function groupTitle(group: MessageGroup) {
  const base = ROLE_LABELS[group.role] ?? group.role;
  const provider = providerForGroup(group);
  const runNo = runNoForGroup(group);
  return [base, provider ? providerLabel(provider) : null, runNo ? `Run #${runNo}` : null]
    .filter(Boolean)
    .join(" · ");
}

const ROLE_LABELS: Record<string, string> = {
  user: "用户",
  author: "作者",
  reviewer: "审核者",
  coder: "Coder",
  tester: "Tester",
  analyst: "Analyst",
  code_reviewer: "Code Reviewer",
  internal_reviewer: "Internal Reviewer",
  system: "系统",
};

function providerForGroup(group: MessageGroup) {
  const entries = [
    group.primaryEntry,
    ...group.inlineEvents,
    ...group.interruptEntries,
  ].filter((entry): entry is ChatEntry => Boolean(entry));
  for (const entry of entries) {
    const provider = metadataProvider(entry.metadata);
    if (provider) {
      return provider;
    }
  }
  return null;
}

function runNoForGroup(group: MessageGroup) {
  const entries = [
    group.primaryEntry,
    ...group.inlineEvents,
    ...group.interruptEntries,
  ].filter((entry): entry is ChatEntry => Boolean(entry));
  for (const entry of entries) {
    const runNo = entry.metadata?.run_no;
    if (typeof runNo === "number" && Number.isFinite(runNo)) {
      return runNo;
    }
  }
  return null;
}

function metadataProvider(metadata: ChatEntry["metadata"]) {
  const provider = metadata?.provider ?? metadata?.agent;
  return typeof provider === "string" && provider.length > 0 ? provider : null;
}

function providerLabel(provider: string) {
  return PROVIDER_LABELS[provider] ?? provider;
}
