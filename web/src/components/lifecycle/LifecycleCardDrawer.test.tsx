import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { LifecycleCardDrawer } from "./LifecycleCardDrawer";

describe("LifecycleCardDrawer", () => {
  it("renders entity info, version history, artifact preview, and next action", () => {
    const onOpenWorkspace = vi.fn();
    const onGenerateNext = vi.fn();

    render(
      <LifecycleCardDrawer
        entity={{
          id: "story-id",
          kind: "story_spec",
          title: "用户认证模块",
          status: "confirmed",
          version: 2,
          artifactVersions: [
            {
              version: 2,
              markdown: "# v2\n\n## 功能需求\n\n[REQ-001] 登录用户看到认证提示。",
              generated_by: "claude_code",
              reviewed_by: "codex",
              review_verdict: "pass",
              confirmed_by: "human",
              created_at: "2026-05-20T14:30:00Z",
              source_node_id: "node-1",
            },
          ],
        }}
        onClose={vi.fn()}
        onOpenWorkspace={onOpenWorkspace}
        onGenerateNext={onGenerateNext}
      />,
    );

    expect(screen.getByTestId("lifecycle-card-drawer")).toBeInTheDocument();
    expect(screen.getByText("用户认证模块")).toBeInTheDocument();
    expect(screen.getAllByText("v2").length).toBeGreaterThan(0);
    expect(screen.getByText("版本历史")).toBeInTheDocument();
    expect(screen.getByText(/REQ-001/)).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("drawer-open-workspace"));
    fireEvent.click(screen.getByTestId("drawer-generate-next"));

    expect(onOpenWorkspace).toHaveBeenCalled();
    expect(onGenerateNext).toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "生成 Design Spec" })).toBeInTheDocument();
  });

  it("calls onClose when close button clicked", () => {
    const onClose = vi.fn();
    render(
      <LifecycleCardDrawer
        entity={{
          id: "story-id",
          kind: "story_spec",
          title: "测试",
          status: "confirmed",
          version: 1,
          artifactVersions: [],
        }}
        onClose={onClose}
        onOpenWorkspace={vi.fn()}
        onGenerateNext={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByLabelText("关闭"));

    expect(onClose).toHaveBeenCalled();
  });
});
