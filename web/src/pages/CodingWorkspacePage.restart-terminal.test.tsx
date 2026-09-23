import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
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

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: () => <div data-testid="monaco-viewer" />,
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: () => <div data-testid="monaco-diff-viewer" />,
}));

describe("CodingWorkspacePage terminal restart entry (F-44)", () => {
  installCodingWorkspacePageTestHooks();

  function renderTerminalPage(status: "aborted" | "failed" | "completed" | "running") {
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status,
      stage: "coding",
      branchName: "aria/work-items/work_item_0001/attempt-1",
      baseBranch: "main",
      worktreePath: "/tmp/worktree",
      timelineNodes: [],
      chatEntries: [],
    });
    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);
  }

  it("offers 重新开始 on an aborted attempt and only sends the restart after the confirm step", async () => {
    const api = mockCodingWs();
    renderTerminalPage("aborted");

    const restart = screen.getByRole("button", { name: "重新开始" });
    expect(screen.queryByRole("button", { name: "中止" })).toBeNull();

    await userEvent.click(restart);
    expect(api.restartCoding).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "确认重新开始" })).toBeInTheDocument();

    await userEvent.click(restart);
    expect(api.restartCoding).toHaveBeenCalledOnce();
  });

  it("offers 重新开始 on a failed attempt", () => {
    mockCodingWs();
    renderTerminalPage("failed");

    expect(screen.getByRole("button", { name: "重新开始" })).toBeInTheDocument();
  });

  it.each(["running", "completed"] as const)(
    "does not offer 重新开始 while the attempt is %s",
    (status) => {
      mockCodingWs();
      renderTerminalPage(status);

      expect(screen.queryByRole("button", { name: "重新开始" })).toBeNull();
      expect(screen.queryByRole("button", { name: "确认重新开始" })).toBeNull();
    },
  );
});
