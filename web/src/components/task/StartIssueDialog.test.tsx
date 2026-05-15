import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { StartIssueDialog } from "./StartIssueDialog";

describe("StartIssueDialog", () => {
  it("renders compact workspace start controls", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    render(
      <StartIssueDialog
        issue={{
          issue_id: "issue_0001",
          title: "启动 Issue",
          description: null,
          status: "draft",
          workspace_id: null,
          task_id: null,
          session_id: null,
          change_id: "start-check",
          created_at: "2026-05-15T00:00:00Z",
          updated_at: "2026-05-15T00:00:00Z",
        }}
        workspaces={[
          {
            workspace_id: "workspace_0001",
            name: "Main workspace",
            path: "/tmp/workspace",
            default_policy_preset: "manual-write",
            default_provider_mode: "fake",
            created_at: "2026-05-15T00:00:00Z",
            updated_at: "2026-05-15T00:00:00Z",
          },
        ]}
        busy={false}
        onCancel={vi.fn()}
        onConfirm={onConfirm}
      />,
    );

    expect(screen.getByRole("dialog", { name: "选择 workspace" })).toBeInTheDocument();
    expect(screen.getByLabelText("启动 workspace")).toHaveDisplayValue(
      "Main workspace · workspace_0001",
    );
    await user.click(screen.getByRole("button", { name: "确认 Start" }));
    expect(onConfirm).toHaveBeenCalledWith("workspace_0001");
  });
});
