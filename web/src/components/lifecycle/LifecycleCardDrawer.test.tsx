import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { LifecycleCardDrawer } from "./LifecycleCardDrawer";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({
    original,
    modified,
    height,
  }: {
    original: string;
    modified: string;
    height?: string;
  }) => (
    <div data-testid="monaco-diff-viewer" data-height={height}>
      <span data-testid="version-diff-original">{original}</span>
      <span data-testid="version-diff-modified">{modified}</span>
    </div>
  ),
}));

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

  it("renders issue description, artifacts, and metadata", () => {
    render(
      <LifecycleCardDrawer
        entity={{
          id: "issue-id",
          kind: "issue",
          title: "登录会话过期",
          status: "draft",
          version: null,
          description: "## 背景\n\n会话过期后需要提示用户。",
          artifacts: [
            {
              artifact_ref: "artifact-story-1",
              artifact_kind: "story_spec",
              producer_node: "node-1",
              path: "story.md",
              summary: "会话过期提示 Story",
              stage: "story_spec",
            },
          ],
          phase: "clarification",
          createdAt: "2026-05-16T00:00:00Z",
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );

    expect(screen.getByText("Issue 描述")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("会话过期后需要提示用户");
    expect(screen.getByText("关联产物")).toBeInTheDocument();
    expect(screen.getAllByText("story_spec")).toHaveLength(2);
    expect(screen.getByText("会话过期提示 Story")).toBeInTheDocument();
    expect(screen.getByText("阶段: clarification")).toBeInTheDocument();
    expect(screen.getByText("创建时间: 2026-05-16")).toBeInTheDocument();
  });

  it("switches spec artifact versions and compares an older version to latest", () => {
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
              markdown: "# v2\n\n新增验收标准",
              generated_by: "claude_code",
              reviewed_by: "codex",
              review_verdict: "pass",
              confirmed_by: "human",
              created_at: "2026-05-20T14:30:00Z",
              source_node_id: "node-2",
            },
            {
              version: 1,
              markdown: "# v1\n\n基础需求",
              generated_by: "claude_code",
              reviewed_by: null,
              review_verdict: null,
              confirmed_by: null,
              created_at: "2026-05-19T14:30:00Z",
              source_node_id: "node-1",
            },
          ],
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
        onGenerateNext={vi.fn()}
      />,
    );

    expect(screen.getByText("版本 v2 预览")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("新增验收标准");

    fireEvent.click(screen.getByRole("button", { name: /v1/ }));

    expect(screen.getByText("版本 v1 预览")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("基础需求");
    fireEvent.click(screen.getByRole("button", { name: "与最新版本对比" }));

    expect(screen.getByTestId("version-diff-original")).toHaveTextContent("# v1");
    expect(screen.getByTestId("version-diff-modified")).toHaveTextContent("# v2");
  });
});
