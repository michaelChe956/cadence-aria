import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { ChatCockpitPage } from "./ChatCockpitPage";
import { LegacyChatWorkspacePage } from "./ChatWorkspacePageLegacy";
import { readChatCockpitMode } from "../state/chat-cockpit-mode";
import { useWorkspaceStore } from "../state/workspace-ws-store";

export function ChatWorkspacePage({
  sessionId,
  onBack,
  onOpenSession,
  onOpenInfoCoding,
}: {
  sessionId: string;
  onBack: () => void;
  onOpenSession: (sessionId: string) => void;
  /** REQ-WIGA-07/R5 Task 9：coding FinalConfirm info 的 attempt 地址下钻。 */
  onOpenInfoCoding?: (address: {
    projectId: string;
    issueId: string;
    attemptId: string;
  }) => void;
}) {
  const workspaceWs = useWorkspaceWs(sessionId);
  const storeSessionId = useWorkspaceStore((state) => state.sessionId);
  const workspaceType = useWorkspaceStore((state) => state.workspaceType);

  if (storeSessionId !== sessionId) {
    return (
      <section data-testid="workspace-connection-shell" aria-live="polite">
        正在连接工作区…
      </section>
    );
  }

  if (readChatCockpitMode(workspaceType) === "cockpit") {
    return (
      <ChatCockpitPage
        sessionId={sessionId}
        onBack={onBack}
        onOpenSession={onOpenSession}
        onOpenInfoCoding={onOpenInfoCoding}
        workspaceWs={workspaceWs}
      />
    );
  }
  return <LegacyChatWorkspacePage sessionId={sessionId} onBack={onBack} workspaceWs={workspaceWs} />;
}
