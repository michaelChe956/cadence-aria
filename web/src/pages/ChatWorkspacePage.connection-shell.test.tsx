import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { ChatWorkspacePage } from "./ChatWorkspacePage";
import {
  installChatWorkspacePageTestHooks,
  mockWorkspaceWs,
} from "./ChatWorkspacePage.test-utils";

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: () => <div data-testid="monaco-viewer" />,
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: () => <div data-testid="monaco-diff-viewer" />,
}));

// F1（work item plan 删除连带删 workspace session）：服务端在 manager create
// 失败臂发 Error{message}（socket.rs），manager 的包装文本形如
// `workspace session not found: product_store_not_found: workspace_session ...`。
const SESSION_GONE_ERROR =
  "workspace session not found: product_store_not_found: workspace_session workspace_session_deleted_0001";

const OTHER_CONNECTION_ERROR =
  "workspace repository unavailable: repository_routing_ambiguous: issue_work_item_plan_0001 has multiple logical repository targets";

describe("ChatWorkspacePage connection shell", () => {
  installChatWorkspacePageTestHooks();

  function renderShell(sessionId: string, error: string | null) {
    mockWorkspaceWs();
    const onBack = vi.fn();
    const view = render(
      <ChatWorkspacePage
        sessionId={sessionId}
        onBack={onBack}
        onOpenSession={vi.fn()}
      />,
    );
    if (error !== null) {
      act(() => {
        useWorkspaceStore.getState().setError(error);
      });
    }
    return { view, onBack };
  }

  it("offers a back-to-overview escape with friendly copy when the session was deleted", async () => {
    const user = userEvent.setup();
    const { onBack } = renderShell("workspace_session_deleted_0001", SESSION_GONE_ERROR);

    expect(screen.getByTestId("workspace-connection-shell")).toBeInTheDocument();
    expect(screen.getByTestId("workspace-session-gone")).toHaveTextContent(
      "该会话已被删除（可能随 story/design/plan 一起被清理）。请返回总览重新选择。",
    );
    // 不吞错误：原始错误仍保留在次要位置（详情区）。
    expect(
      within(screen.getByTestId("workspace-connection-error")).getByText(
        /product_store_not_found/,
      ),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "回总览" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("keeps the raw error for other connection failures and still offers the escape", async () => {
    const user = userEvent.setup();
    const { onBack } = renderShell("session_other_failure", OTHER_CONNECTION_ERROR);

    expect(screen.queryByTestId("workspace-session-gone")).toBeNull();
    expect(screen.queryByText(/该会话已被删除/)).toBeNull();
    expect(screen.getByTestId("workspace-connection-error")).toHaveTextContent(
      OTHER_CONNECTION_ERROR,
    );

    await user.click(screen.getByRole("button", { name: "回总览" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("leaves a healthy bound session untouched", () => {
    mockWorkspaceWs();
    act(() => {
      useWorkspaceStore.getState().setSessionState({
        session_id: "workspace_session_live_0001",
        workspace_type: "story",
        stage: "prepare_context",
        session_status: "open",
        flow_kind: "legacy",
        run_policy: "interactive",
        run_history: {
          seen_fingerprints: [],
          repairs_used: 0,
          manual_repairs_used: 0,
          transitions_used: 0,
          initial_review_count: 0,
          verification_review_count: 0,
        },
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
      });
    });

    render(
      <ChatWorkspacePage
        sessionId="workspace_session_live_0001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
      />,
    );

    expect(screen.queryByTestId("workspace-connection-shell")).toBeNull();
    expect(screen.queryByRole("button", { name: "回总览" })).toBeNull();
    expect(screen.getByTestId("cockpit-page")).toBeInTheDocument();
  });
});
