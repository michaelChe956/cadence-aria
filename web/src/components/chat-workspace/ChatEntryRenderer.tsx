import type { ChatEntry } from "../../state/chat-entries";
import type { RevisionPath } from "../../api/types";
import { ArtifactUpdateEntry } from "./entries/ArtifactUpdateEntry";
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
  onSelectRevisionPath?: (path: RevisionPath) => void;
  onHumanConfirm?: (decision: "confirm" | "request-change" | "terminate") => void;
}

export function ChatEntryRenderer({
  entry,
  onPermissionResponse,
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
    case "artifact_update":
      return <ArtifactUpdateEntry entry={entry} />;
    case "review_verdict":
      return <ReviewVerdictEntry entry={entry} onSelectPath={onSelectRevisionPath} />;
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
