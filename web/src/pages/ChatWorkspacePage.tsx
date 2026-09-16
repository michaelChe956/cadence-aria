import { ChatCockpitPage } from "./ChatCockpitPage";
import { LegacyChatWorkspacePage } from "./ChatWorkspacePageLegacy";
import { readChatCockpitMode } from "../state/chat-cockpit-mode";
import { useWorkspaceStore } from "../state/workspace-ws-store";

// 显式开关优先；类型到达前保持 legacy，避免未知会话抢先进入 cockpit。
// work_item_plan 在生成节点出现后默认进入 cockpit，其他会话沿用 legacy。
export function ChatWorkspacePage({
  sessionId,
  onBack,
  onOpenSession,
}: {
  sessionId: string;
  onBack: () => void;
  onOpenSession: (sessionId: string) => void;
}) {
  const workspaceType = useWorkspaceStore((state) =>
    state.sessionId === sessionId ? state.workspaceType : null,
  );
  const hasTimelineNodes = useWorkspaceStore(
    (state) => state.sessionId === sessionId && state.timelineNodes.length > 0,
  );


  if (readChatCockpitMode(workspaceType, hasTimelineNodes) === "cockpit") {
    return (
      <ChatCockpitPage
        sessionId={sessionId}
        onBack={onBack}
        onOpenSession={onOpenSession}
      />
    );
  }
  return <LegacyChatWorkspacePage sessionId={sessionId} onBack={onBack} />;
}
