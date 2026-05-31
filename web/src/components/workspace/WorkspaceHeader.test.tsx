import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { WorkspaceHeader } from "./WorkspaceHeader";

describe("WorkspaceHeader", () => {
  it("renders provider snapshot and stage badge", () => {
    render(
      <WorkspaceHeader
        entityType="Story Spec"
        entityId="SP-12"
        version={2}
        author="claude_code"
        reviewer="codex"
        rounds={1}
        stage="running"
        providerLocked={true}
        lockedAt="2026-05-20T14:35:00Z"
      />,
    );

    expect(screen.getByText(/Story Spec #SP-12/)).toBeInTheDocument();
    expect(screen.getByText(/v2/)).toBeInTheDocument();
    expect(screen.getByText(/Author: Claude Code/)).toBeInTheDocument();
    expect(screen.getByText(/Reviewer: Codex/)).toBeInTheDocument();
    expect(screen.getByText(/1 round/)).toBeInTheDocument();
    expect(screen.getByText("运行中 · 保持本页打开")).toBeInTheDocument();
    expect(screen.getByLabelText("Provider 已锁定")).toBeInTheDocument();
  });

  it("hides the lock icon when provider is editable", () => {
    render(
      <WorkspaceHeader
        entityType="Story Spec"
        entityId="SP-12"
        version={2}
        author="claude_code"
        reviewer={null}
        rounds={0}
        stage="prepare_context"
        providerLocked={false}
      />,
    );

    expect(screen.queryByLabelText("Provider 已锁定")).not.toBeInTheDocument();
  });
});
