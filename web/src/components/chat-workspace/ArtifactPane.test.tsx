import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ArtifactVersion } from "../../state/workspace-ws-store";
import { ArtifactPane } from "./ArtifactPane";

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
      <span data-testid="artifact-diff-original">{original}</span>
      <span data-testid="artifact-diff-modified">{modified}</span>
    </div>
  ),
}));

describe("ArtifactPane", () => {
  it("renders the latest artifact version and switches versions", () => {
    render(<ArtifactPane artifactVersions={artifactVersions()} artifact="# fallback" />);

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v2");
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("新增内容");

    fireEvent.change(screen.getByLabelText("Artifact 版本"), { target: { value: "1" } });

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v1");
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("旧内容");
  });

  it("shows a Monaco diff and can collapse", () => {
    render(<ArtifactPane artifactVersions={artifactVersions()} artifact={null} />);

    fireEvent.click(screen.getByRole("button", { name: "显示 Diff" }));
    expect(screen.getByTestId("artifact-diff")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-diff-viewer")).toBeInTheDocument();
    expect(screen.getByTestId("artifact-diff-original")).toHaveTextContent("# Artifact v1");
    expect(screen.getByTestId("artifact-diff-modified")).toHaveTextContent("新增内容");

    fireEvent.click(screen.getByRole("button", { name: "折叠 Artifact" }));
    expect(screen.queryByLabelText("Artifact 版本")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "展开 Artifact" }));
    expect(screen.getByLabelText("Artifact 版本")).toBeInTheDocument();
  });
});

function artifactVersions(): ArtifactVersion[] {
  return [
    {
      version: 1,
      markdown: "# Artifact v1\n\n旧内容",
      generated_by: "claude_code",
      created_at: "2026-05-21T10:00:00Z",
      source_node_id: "node-1",
    },
    {
      version: 2,
      markdown: "# Artifact v2\n\n旧内容\n新增内容",
      generated_by: "claude_code",
      created_at: "2026-05-21T10:01:00Z",
      source_node_id: "node-2",
    },
  ];
}
