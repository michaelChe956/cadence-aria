import type { CockpitActionFacade } from "../../state/cockpit-action-routing";
import type { ChatEntry } from "../../state/chat-entries";
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
  actions?: CockpitActionFacade;
  /** F-38：门卡「查看产物」入口——切到产物视图（cockpit 产物审核页签）。 */
  onOpenArtifact?: () => void;
}

export function ChatEntryRenderer({
  entry,
  onPermissionResponse,
  onChoiceResponse,
  actions,
  onOpenArtifact,
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
      // 退役留档（T5/REQ-RET-02）：review_decision 路径动作面已删，verdict 仅只读呈现。
      return <ReviewVerdictEntry entry={entry} />;
    case "gate_prompt":
      return (
        <GatePromptEntry
          entry={entry}
          actions={actions}
          onOpenArtifact={onOpenArtifact}
        />
      );
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
