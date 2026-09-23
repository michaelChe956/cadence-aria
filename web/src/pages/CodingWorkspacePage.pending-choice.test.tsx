import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../state/chat-entries";
import { getCodingAttemptDiff } from "../api/client";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useWorkspaceSessionObservers } from "../hooks/useWorkspaceSessionObservers";
import { CockpitShell } from "../components/cockpit/CockpitShell";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
  readyCodingState,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", () => ({
  confirmWorkItemExecutionPlan: vi.fn(),
  deleteCodingAttempt: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  requestWorkItemExecutionPlanChange: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../hooks/useWorkspaceSessionObservers", () => ({
  useWorkspaceSessionObservers: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => <div data-testid="monaco-viewer">{value}</div>,
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({ original, modified }: { original: string; modified: string }) => (
    <div data-testid="monaco-diff-viewer">
      <span>{original}</span>
      <span>{modified}</span>
    </div>
  ),
}));

const DANGEROUS_PROMPT =
  '⚠️ Dangerous command:\n\n  rm -rf "$BASE"; mkdir -p "$BASE"\n\nAllow?';

function pendingChoiceEntry(overrides: Partial<ChatEntry> = {}): ChatEntry {
  return {
    id: "choice_request:choice-1",
    type: "choice_request",
    role: "coder",
    content: DANGEROUS_PROMPT,
    timestamp: "2026-09-23T04:00:45Z",
    metadata: {
      request_id: "choice-1",
      prompt: DANGEROUS_PROMPT,
      source: "provider_choice",
      options: [
        { id: "Yes", label: "Yes" },
        { id: "No", label: "No" },
      ],
      allow_multiple: false,
      allow_free_text: true,
    },
    ...overrides,
  } as ChatEntry;
}

function stubObservers() {
  vi.mocked(useWorkspaceSessionObservers).mockReturnValue({
    records: [],
    inbox: [],
    countedInbox: [],
    watchedSessionIds: [],
    watchSession: vi.fn(),
  });
}

describe("CodingWorkspacePage choice arrival and settings slot", () => {
  installCodingWorkspacePageTestHooks();

  function renderReadyPage() {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "waiting_for_human",
      stage: "coding",
    });
    return render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);
  }

  // F-43 ②：coding attempt 的 choice 不走 workspace observer（controller 实测
  // attach 无选择帧），卡到达时页面没有任何可见提示。契约：未处理 choice 常驻
  // 横幅提示 + 一键定位到卡；应答后横幅消失。
  it("announces a pending choice that arrives in the run conversation", async () => {
    renderReadyPage();

    expect(screen.queryByTestId("pending-choice-notice")).toBeNull();

    act(() => {
      useCodingWorkspaceStore.getState().appendChatEntry(pendingChoiceEntry());
    });

    const notice = screen.getByTestId("pending-choice-notice");
    expect(notice).toHaveAttribute("role", "status");
    expect(notice).toHaveTextContent("有 1 个选择请求待处理");
    expect(notice).toHaveTextContent("⚠️ Dangerous command:");

    await userEvent.click(screen.getByRole("button", { name: "定位选择卡" }));

    expect(screen.getByTestId("choice-request-entry")).toBeInTheDocument();

    // 页级提示：停在「运行结果」页签时选择卡不可见，但仍必须看到待处理提示。
    vi.mocked(getCodingAttemptDiff).mockResolvedValue({
      attempt_id: CODING_ATTEMPT_ADDRESS.attemptId,
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      diff: "",
    });
    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    expect(screen.queryByTestId("choice-request-entry")).toBeNull();
    expect(screen.getByTestId("pending-choice-notice")).toHaveTextContent("有 1 个选择请求待处理");
  });

  // k3 复审 P2：横幅在「运行结果」页签下点「定位选择卡」时对话列表未挂载
  // （ref=null）→ 原实现静默无操作。契约：先切回对话页签，列表重挂后再滚动。
  it("switches back to the conversation panel before scrolling when the results tab is open", async () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
    vi.mocked(getCodingAttemptDiff).mockResolvedValue({
      attempt_id: CODING_ATTEMPT_ADDRESS.attemptId,
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      diff: "",
    });
    renderReadyPage();
    act(() => {
      useCodingWorkspaceStore.getState().appendChatEntry(pendingChoiceEntry());
    });

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    expect(screen.queryByTestId("coding-chat-entry-list")).toBeNull();

    scrollIntoView.mockClear();
    await userEvent.click(screen.getByRole("button", { name: "定位选择卡" }));

    expect(screen.getByTestId("coding-chat-entry-list")).toBeInTheDocument();
    expect(screen.getByTestId("choice-request-entry")).toBeInTheDocument();
    expect(scrollIntoView).toHaveBeenCalled();
  });

  it("drops the announcement once the choice is answered", () => {
    renderReadyPage();

    act(() => {
      useCodingWorkspaceStore.getState().appendChatEntry(pendingChoiceEntry());
    });
    expect(screen.getByTestId("pending-choice-notice")).toBeInTheDocument();

    act(() => {
      useCodingWorkspaceStore.getState().appendChatEntry(
        pendingChoiceEntry({
          resolved: true,
          metadata: {
            request_id: "choice-1",
            prompt: DANGEROUS_PROMPT,
            response: { selected_option_ids: ["Yes"], free_text: null },
          },
        }),
      );
    });

    expect(screen.queryByTestId("pending-choice-notice")).toBeNull();
  });

  // F-43 ③：coding attempt 页此前不注册设置入口宿主，CockpitShell 只能落
  // fixed 兜底浮层（压内容/乱飘）。契约：页头顶栏登记 slot，兜底浮层不再出现。
  it("registers the cockpit settings slot in its page header", () => {
    stubObservers();
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "waiting_for_human",
      stage: "coding",
    });

    render(
      <CockpitShell>
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />
      </CockpitShell>,
    );

    const slot = screen.getByTestId("cockpit-settings-slot");
    expect(slot).toContainElement(screen.getByTestId("cockpit-settings-trigger"));
    expect(slot.closest('[data-testid="coding-workspace-top-bar"]')).not.toBeNull();
    expect(screen.queryByTestId("cockpit-settings-fallback")).toBeNull();
  });
});
