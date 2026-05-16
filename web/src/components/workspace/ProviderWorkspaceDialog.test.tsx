import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProviderWorkspaceDialog } from "./ProviderWorkspaceDialog";

describe("ProviderWorkspaceDialog", () => {
  it("shows flow rail conversation artifact pane and config overrides", async () => {
    const user = userEvent.setup();
    const onMessage = vi.fn();

    render(
      <ProviderWorkspaceDialog
        open
        title="Story Workspace"
        session={{
          workspace_session_id: "workspace_session_0001",
          issue_id: "issue_0001",
          entity_id: "story_spec_0001",
          workspace_type: "story",
          status: "waiting_for_human",
          author_provider: "codex",
          reviewer_provider: "claude_code",
          review_rounds: 2,
          superpowers_enabled: true,
          openspec_enabled: true,
          messages: [
            {
              role: "assistant",
              content: "请确认 Story Spec 边界",
              created_at: "2026-05-16T00:00:00Z",
            },
          ],
        }}
        onClose={vi.fn()}
        onMessage={onMessage}
        onRunNext={vi.fn()}
        onConfirm={vi.fn()}
        onRequestChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("dialog", { name: "Story Workspace" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Workspace 流程" })).toHaveTextContent(
      "author draft",
    );
    expect(screen.getByRole("region", { name: "Provider 对话" })).toHaveTextContent(
      "请确认 Story Spec 边界",
    );
    expect(screen.getByRole("region", { name: "Workspace 产物" })).toHaveTextContent(
      "workspace_session_0001",
    );
    expect(screen.getByText("review 2")).toBeInTheDocument();
    expect(screen.getByText("superpowers")).toBeInTheDocument();
    expect(screen.getByText("openspec")).toBeInTheDocument();

    await user.type(screen.getByLabelText("补充指令"), "请补充边界条件");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(onMessage).toHaveBeenCalledWith("请补充边界条件");
  });

  it("shows work item execution flow from plan onward", () => {
    render(
      <ProviderWorkspaceDialog
        open
        title="Work Item Workspace"
        session={{
          workspace_session_id: "workspace_session_0002",
          issue_id: "issue_0001",
          entity_id: "work_item_0001",
          workspace_type: "work_item",
          status: "running",
          author_provider: "codex",
          reviewer_provider: "claude_code",
          review_rounds: 3,
          superpowers_enabled: true,
          openspec_enabled: false,
          messages: [],
        }}
        onClose={vi.fn()}
        onMessage={vi.fn()}
        onRunNext={vi.fn()}
        onConfirm={vi.fn()}
        onRequestChange={vi.fn()}
      />,
    );

    const flow = screen.getByRole("navigation", { name: "Workspace 流程" });
    expect(flow).toHaveTextContent("author plan");
    expect(flow).toHaveTextContent("confirm plan");
    expect(flow).toHaveTextContent("coding");
    expect(flow).toHaveTextContent("testing");
    expect(flow).toHaveTextContent("review");
    expect(flow).toHaveTextContent("final");
  });

  it("shows action errors without leaking rejected promises", async () => {
    const user = userEvent.setup();

    render(
      <ProviderWorkspaceDialog
        open
        title="Story Workspace"
        session={{
          workspace_session_id: "workspace_session_0001",
          issue_id: "issue_0001",
          entity_id: "story_spec_0001",
          workspace_type: "story",
          status: "waiting_for_human",
          author_provider: "codex",
          reviewer_provider: "claude_code",
          review_rounds: 2,
          superpowers_enabled: true,
          openspec_enabled: true,
          messages: [],
        }}
        onClose={vi.fn()}
        onMessage={vi.fn()}
        onRunNext={vi.fn().mockRejectedValue(new Error("run failed"))}
        onConfirm={vi.fn()}
        onRequestChange={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "下一步" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("run failed");
  });
});
