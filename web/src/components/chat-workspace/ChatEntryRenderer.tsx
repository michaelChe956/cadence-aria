import type { ChatEntry } from "../../state/chat-entries";
import type { RevisionPath } from "../../api/types";
import { AnalystVerdictEntry } from "./entries/AnalystVerdictEntry";
import { ArtifactUpdateEntry } from "./entries/ArtifactUpdateEntry";
import { ChoiceRequestEntry } from "./entries/ChoiceRequestEntry";
import { ChoiceResponseEntry } from "./entries/ChoiceResponseEntry";
import { ErrorEntry } from "./entries/ErrorEntry";
import { ExecutionEventEntry } from "./entries/ExecutionEventEntry";
import { GatePromptEntry } from "./entries/GatePromptEntry";
import { HumanDecisionEntry } from "./entries/HumanDecisionEntry";
import { PermissionRequestEntry } from "./entries/PermissionRequestEntry";
import { PermissionResponseEntry } from "./entries/PermissionResponseEntry";
import { ReviewVerdictEntry } from "./entries/ReviewVerdictEntry";
import { StageChangeEntry } from "./entries/StageChangeEntry";
import { StartGenerationEntry } from "./entries/StartGenerationEntry";
import { ProviderStreamEntry } from "./entries/ProviderStreamEntry";
import { UserContextEntry } from "./entries/UserContextEntry";
import { ChatEntryContainer } from "./ChatEntryContainer";

interface ChatEntryRendererProps {
  entry: ChatEntry;
  onPermissionResponse?: (entry: ChatEntry, approved: boolean) => void;
  onChoiceResponse?: (
    entry: ChatEntry,
    response: { selected_option_ids: string[]; free_text: string | null },
  ) => void;
  onSelectRevisionPath?: (path: RevisionPath, extraContext?: string) => void;
  onHumanConfirm?: (
    decision: "confirm" | "request-change" | "terminate",
    payload?: unknown,
  ) => void;
}

export function ChatEntryRenderer({
  entry,
  onPermissionResponse,
  onChoiceResponse,
  onSelectRevisionPath,
  onHumanConfirm,
}: ChatEntryRendererProps) {
  switch (entry.type) {
    case "context_note":
      return <UserContextEntry entry={entry} />;
    case "start_generation":
      return <StartGenerationEntry entry={entry} />;
    case "provider_stream":
      return <ProviderStreamEntry entry={entry} />;
    case "execution_event":
      return <ExecutionEventEntry entry={entry} />;
    case "permission_request":
      return <PermissionRequestEntry entry={entry} onRespond={onPermissionResponse} />;
    case "permission_response":
      return <PermissionResponseEntry entry={entry} />;
    case "choice_request":
      return <ChoiceRequestEntry entry={entry} onRespond={onChoiceResponse} />;
    case "choice_response":
      return <ChoiceResponseEntry entry={entry} />;
    case "artifact_update":
      return <ArtifactUpdateEntry entry={entry} />;
    case "review_verdict":
      return <ReviewVerdictEntry entry={entry} onSelectPath={onSelectRevisionPath} />;
    case "analyst_verdict":
      return <AnalystVerdictEntry entry={entry} />;
    case "gate_prompt":
      return <GatePromptEntry entry={entry} onDecision={onHumanConfirm} />;
    case "human_decision":
      return <HumanDecisionEntry entry={entry} />;
    case "stage_change":
      return <StageChangeEntry entry={entry} />;
    case "error":
      return <ErrorEntry entry={entry} />;
    default:
      return (
        <ChatEntryContainer role={entry.role} title={entry.type}>
          <div className="whitespace-pre-wrap text-sm text-[var(--aria-ink)]">{entry.content}</div>
        </ChatEntryContainer>
      );
  }
}
