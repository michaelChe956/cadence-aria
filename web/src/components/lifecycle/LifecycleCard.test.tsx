import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { LifecycleCard } from "./LifecycleCard";

describe("LifecycleCard", () => {
  it("uses distinct color tokens and visible labels for issue lifecycle entity types", () => {
    const cards = [
      lifecycleCard("issue", "登录流程异常"),
      lifecycleCard("story_spec", "登录故事规格"),
      lifecycleCard("design_spec", "登录设计方案"),
      lifecycleCard("work_item", "登录实现任务"),
    ];

    render(
      <div>
        {cards.map((card) => (
          <LifecycleCard
            key={card.kind}
            card={card}
            selected={false}
            onSelect={vi.fn()}
          />
        ))}
      </div>,
    );

    expect(screen.getByTestId("lifecycle-card-issue")).toHaveAttribute(
      "data-color-token",
      "sky",
    );
    expect(screen.getByTestId("lifecycle-card-story_spec")).toHaveAttribute(
      "data-color-token",
      "emerald",
    );
    expect(screen.getByTestId("lifecycle-card-design_spec")).toHaveAttribute(
      "data-color-token",
      "violet",
    );
    expect(screen.getByTestId("lifecycle-card-work_item")).toHaveAttribute(
      "data-color-token",
      "amber",
    );

    expect(screen.getByText("Issue")).toBeInTheDocument();
    expect(screen.getByText("Story")).toBeInTheDocument();
    expect(screen.getByText("Design")).toBeInTheDocument();
    expect(screen.getByText("Work Item")).toBeInTheDocument();
  });

  it("allows long card titles to use two lines before truncating", () => {
    render(
      <LifecycleCard
        card={lifecycleCard(
          "story_spec",
          "一个很长的 Story Spec 标题用于验证卡片不会只展示右侧很短的一行内容",
        )}
        selected={false}
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByTestId("lifecycle-card-title")).toHaveClass(
      "line-clamp-2",
    );
    expect(screen.getByTestId("lifecycle-card-title")).not.toHaveClass(
      "truncate",
    );
  });

  it("uses a restrained deleting state without leaving controls active", () => {
    render(
      <LifecycleCard
        card={lifecycleCard("story_spec", "会话过期提示")}
        selected={false}
        deleting={true}
        onSelect={vi.fn()}
        onDelete={vi.fn()}
      />,
    );

    expect(screen.getByTestId("lifecycle-card-story_spec")).toHaveAttribute(
      "data-delete-state",
      "deleting",
    );
    expect(screen.getByTestId("lifecycle-card-story_spec")).toHaveClass(
      "aria-lifecycle-card--deleting",
    );
    expect(screen.getByRole("button", { name: "会话过期提示" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "删除 Story Spec 会话过期提示" }),
    ).toBeDisabled();
  });
});

function lifecycleCard(
  kind: LifecycleCardData["kind"],
  title: string,
): LifecycleCardData {
  const base = {
    id: `${kind}_0001`,
    issueId: "issue_0001",
    title,
    status: "draft",
    version: null,
    preview: "摘要",
    sourceIds: [],
  };

  if (kind === "issue") {
    return {
      ...base,
      kind,
      raw: {
        issue_id: "issue_0001",
        project_id: "project_0001",
        repo_id: "repository_0001",
        workspace_id: null,
        task_id: null,
        session_id: null,
        title,
        description: "摘要",
        change_id: "issue",
        phase: "clarification",
        status: "draft",
        active_binding_id: null,
        artifacts: [],
        created_at: "2026-05-25T00:00:00Z",
        updated_at: "2026-05-25T00:00:00Z",
      },
    };
  }

  if (kind === "story_spec") {
    return {
      ...base,
      kind,
      artifactVersions: [],
      raw: {
        story_spec_id: "story_spec_0001",
        issue_id: "issue_0001",
        repository_id: "repository_0001",
        title,
        current_version: null,
        current_markdown_preview: "摘要",
        confirmation_status: "draft",
        artifact_versions: [],
      },
    };
  }

  if (kind === "design_spec") {
    return {
      ...base,
      kind,
      artifactVersions: [],
      raw: {
        design_spec_id: "design_spec_0001",
        issue_id: "issue_0001",
        story_spec_ids: ["story_spec_0001"],
        design_kind: "frontend",
        title,
        current_version: null,
        current_markdown_preview: "摘要",
        confirmation_status: "draft",
        artifact_versions: [],
      },
    };
  }

  return {
    ...base,
    kind,
    raw: {
      work_item_id: "work_item_0001",
      issue_id: "issue_0001",
      repository_id: "repository_0001",
      story_spec_ids: ["story_spec_0001"],
      design_spec_ids: ["design_spec_0001"],
      title,
      plan_status: "draft",
      execution_status: "pending",
      latest_attempt: null,
    },
  };
}
