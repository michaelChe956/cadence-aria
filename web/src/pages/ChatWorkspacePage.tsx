import { ChatCockpitPage } from "./ChatCockpitPage";
import { LegacyChatWorkspacePage } from "./ChatWorkspacePageLegacy";
import { readChatCockpitMode } from "../state/chat-cockpit-mode";

// M5：页内分支早返回——命中 cockpit 才渲染三区，否则渲染既有四块形态。
// 不新增路由、不新增独立开关组件；旧代码路径原样保留在 ChatWorkspacePageLegacy.tsx 便于回滚。
export function ChatWorkspacePage({
  sessionId,
  onBack,
}: {
  sessionId: string;
  onBack: () => void;
}) {
  if (readChatCockpitMode() === "cockpit") {
    return <ChatCockpitPage sessionId={sessionId} onBack={onBack} />;
  }
  return <LegacyChatWorkspacePage sessionId={sessionId} onBack={onBack} />;
}
