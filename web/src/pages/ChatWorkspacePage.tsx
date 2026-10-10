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
  const storeError = useWorkspaceStore((state) => state.error);

  if (storeSessionId !== sessionId) {
    return (
      <section data-testid="workspace-connection-shell" aria-live="polite">
        正在连接工作区…
        {/* S6（2026-10-10 E2E）可观测性：服务端 create 失败（如仓库路由
            TargetAmbiguous）只回一帧 Error 即关连接，store 不绑定 session；
            连接壳必须透出错误，否则用户只见「正在连接」看不到失败原因。 */}
        {storeError ? (
          <p
            data-testid="workspace-connection-error"
            role="alert"
            className="mt-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-sm text-[var(--aria-danger)]"
          >
            {storeError}
          </p>
        ) : null}
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
