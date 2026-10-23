import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { ChatCockpitPage } from "./ChatCockpitPage";
import { LegacyChatWorkspacePage } from "./ChatWorkspacePageLegacy";
import { readChatCockpitMode } from "../state/chat-cockpit-mode";
import { useWorkspaceStore } from "../state/workspace-ws-store";

/**
 * F1：判断连接失败是否因为会话已不存在（被删除）。
 *
 * 服务端 create 失败臂（`src/web/workspace_ws_handler/socket.rs`）把 manager
 * 的 `workspace session not found: product_store_not_found: workspace_session …`
 * 原样作为 Error 帧 message 下发；产品 store 侧的稳定 code 形态为
 * `workspace_session_not_found`。两种形态都按「会话不存在」处理。
 */
export function isWorkspaceSessionGoneError(error: string | null): boolean {
  if (!error) {
    return false;
  }
  const normalized = error.toLowerCase();
  const mentionsWorkspaceSession =
    normalized.includes("workspace_session") || normalized.includes("workspace session");
  return (
    mentionsWorkspaceSession &&
    (normalized.includes("not_found") || normalized.includes("not found"))
  );
}

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
    const sessionGone = isWorkspaceSessionGoneError(storeError);
    return (
      <section data-testid="workspace-connection-shell" aria-live="polite">
        正在连接工作区…
        {/* S6（2026-10-10 E2E）可观测性：服务端 create 失败（如仓库路由
            TargetAmbiguous）只回一帧 Error 即关连接，store 不绑定 session；
            连接壳必须透出错误，否则用户只见「正在连接」看不到失败原因。 */}
        {storeError ? (
          sessionGone ? (
            // F1（2026-10-23）：会话已删（work item plan/story/design 删除连带
            // 清理 workspace session）时后端 Error 原文对用户无意义——给友好
            // 文案，原始错误降级到详情区，仍可查看、不吞掉。
            <div
              data-testid="workspace-session-gone"
              role="alert"
              className="mt-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-sm text-[var(--aria-ink)]"
            >
              <p>该会话已被删除（可能随 story/design/plan 一起被清理）。请返回总览重新选择。</p>
              <details className="mt-2 text-xs text-[var(--aria-ink-muted)]">
                <summary>错误详情</summary>
                <p
                  data-testid="workspace-connection-error"
                  className="aria-mono mt-1 break-all"
                >
                  {storeError}
                </p>
              </details>
            </div>
          ) : (
            <p
              data-testid="workspace-connection-error"
              role="alert"
              className="mt-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-sm text-[var(--aria-danger)]"
            >
              {storeError}
            </p>
          )
        ) : null}
        {/* F1：连接失败壳必须有出路——坏 URL（session 已不存在）下浏览器刷新
            无效，只有离开当前路由才能恢复。 */}
        {storeError ? (
          <button type="button" onClick={onBack} className="btn-secondary mt-2">
            回总览
          </button>
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
