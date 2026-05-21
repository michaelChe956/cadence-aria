import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ArtifactVersion } from "../../state/workspace-ws-store";
import { ArtifactPane } from "./ArtifactPane";

describe("ArtifactPane", () => {
  it("renders the latest artifact version and switches versions", () => {
    render(<ArtifactPane artifactVersions={artifactVersions()} artifact="# fallback" />);

    expect(screen.getByRole("heading", { name: "Artifact v2" })).toBeInTheDocument();
    expect(screen.getByTestId("artifact-pane")).toHaveTextContent("新增内容");

    fireEvent.change(screen.getByLabelText("Artifact 版本"), { target: { value: "1" } });

    expect(screen.getByRole("heading", { name: "Artifact v1" })).toBeInTheDocument();
    expect(screen.getByText("旧内容")).toBeInTheDocument();
  });

  it("shows a line diff and can collapse", () => {
    render(<ArtifactPane artifactVersions={artifactVersions()} artifact={null} />);

    fireEvent.click(screen.getByRole("button", { name: "显示 Diff" }));
    expect(screen.getByTestId("artifact-diff")).toHaveTextContent("+ 新增内容");

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
