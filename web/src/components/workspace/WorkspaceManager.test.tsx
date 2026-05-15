import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { WorkspaceManager } from "./WorkspaceManager";

describe("WorkspaceManager", () => {
  it("deletes a workspace registry entry from the list", async () => {
    const user = userEvent.setup();
    const onDeleteWorkspace = vi.fn();

    render(
      <WorkspaceManager
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
        onCreateWorkspace={vi.fn()}
        onDeleteWorkspace={onDeleteWorkspace}
      />,
    );

    await user.click(screen.getByRole("button", { name: "删除 workspace Main workspace" }));

    expect(onDeleteWorkspace).toHaveBeenCalledWith("workspace_0001");
  });
});
