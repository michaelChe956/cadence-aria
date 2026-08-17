import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ArtifactVersionSummary } from "../../state/workspace-ws-store";
import { ArtifactReviewPanel } from "./ArtifactReviewPanel";

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
  }: {
    original: string;
    modified: string;
  }) => (
    <div data-testid="artifact-diff-viewer">
      <span>{original}</span>
      <span>{modified}</span>
    </div>
  ),
}));

function baseProps() {
  return {
    artifactVersions: [
      {
        version: 1,
        generated_by: "claude_code",
        created_at: "2026-08-17T10:00:00Z",
        source_node_id: "node-1",
      },
      {
        version: 2,
        generated_by: "claude_code",
        created_at: "2026-08-17T11:00:00Z",
        source_node_id: "node-2",
      },
    ] satisfies ArtifactVersionSummary[],
    artifact: "# Artifact 全文",
    sessionId: "session-1",
    artifactContentCache: {},
    loadArtifactVersion: vi.fn(() => Promise.resolve("# Artifact 全文")),
    onCacheArtifactContent: vi.fn(),
    onClose: vi.fn(),
  };
}

describe("ArtifactReviewPanel", () => {
  it("渲染产物全文与吸顶操作条，改动摘要条默认展开", () => {
    const props = baseProps();
    render(
      <ArtifactReviewPanel
        {...props}
        changelogSummary="新增 REQ-006"
        actions={<button type="button">定稿</button>}
      />,
    );

    expect(screen.getByText("新增 REQ-006")).toBeVisible();
    expect(screen.getByText("本轮改动")).toBeInTheDocument();
    // details 默认展开
    expect(
      (screen.getByText("本轮改动").closest("details") as HTMLDetailsElement).open,
    ).toBe(true);
    expect(screen.getByTestId("artifact-review-actions").className).toContain("sticky");
    expect(screen.getByRole("button", { name: "定稿" })).toBeInTheDocument();
    expect(screen.getByTestId("artifact-pane")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "收起面板" })).toBeInTheDocument();
  });

  it("changelogSummary 为空时整条不渲染", () => {
    const props = baseProps();
    render(<ArtifactReviewPanel {...props} changelogSummary={undefined} actions={null} />);

    expect(screen.queryByText("本轮改动")).not.toBeInTheDocument();
    expect(screen.queryByTestId("artifact-review-actions")).not.toBeInTheDocument();
  });

  it("点击收起按钮触发 onClose", async () => {
    const props = baseProps();
    render(<ArtifactReviewPanel {...props} actions={null} />);
    await userEvent.click(screen.getByRole("button", { name: "收起面板" }));
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });
});
