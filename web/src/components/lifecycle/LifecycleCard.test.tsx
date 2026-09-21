import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  DesignSpec,
  IssueWorkItemPlanDetailDto,
  LifecycleWorkItem,
  StorySpec,
} from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { LifecycleCard } from "./LifecycleCard";

describe("LifecycleCard", () => {
  it("uses distinct color tokens and visible labels for issue lifecycle entity types", () => {
    const cards = [
      lifecycleCard("issue", "登录流程异常"),
      lifecycleCard("story_spec", "登录故事规格"),
      lifecycleCard("design_spec", "登录设计方案"),
      lifecycleCard("work_item", "登录实现任务"),
      lifecycleCard("work_item_group", "登录 Work Item Group"),
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
    expect(
      screen.getByTestId("lifecycle-card-work_item_group"),
    ).toHaveAttribute("data-color-token", "amber");

    expect(screen.getByText("Issue")).toBeInTheDocument();
    expect(screen.getByText("Story")).toBeInTheDocument();
    expect(screen.getByText("Design")).toBeInTheDocument();
    expect(screen.getByText("Work Item")).toBeInTheDocument();
    expect(screen.getByText("Work Item Group")).toBeInTheDocument();
  });
  // 统一视觉语言：status chip 只跟「状态」走（本地化文案 + 语义色调），
  // 不跟 kind 走——story 与 design 的同状态 chip 必须完全同构。
  it("unifies status chips across story and design: localized label, tone, identical styling", () => {
    const cards = [
      { ...lifecycleCard("story_spec", "已确认故事"), status: "confirmed" },
      { ...lifecycleCard("design_spec", "已确认设计"), status: "confirmed" },
      { ...lifecycleCard("story_spec", "草稿故事"), status: "draft" },
      { ...lifecycleCard("design_spec", "草稿设计"), status: "draft" },
      { ...lifecycleCard("design_spec", "未知状态设计"), status: "weird_state" },
    ];
    render(
      <div>
        {cards.map((card) => (
          <LifecycleCard
            key={`${card.kind}-${card.title}`}
            card={card}
            selected={false}
            onSelect={vi.fn()}
          />
        ))}
      </div>,
    );

    const chips = screen.getAllByTestId("lifecycle-card-status-chip");
    expect(chips).toHaveLength(5);

    // 本地化文案（与抽屉共享同一套 label）
    expect(screen.getAllByText("已确认")).toHaveLength(2);
    expect(screen.getAllByText("草稿")).toHaveLength(2);
    expect(screen.getByText("weird_state")).toBeInTheDocument();

    // 同状态跨 kind：tone 与样式类完全一致
    expect(chips[0]).toHaveAttribute("data-tone", "confirmed");
    expect(chips[1]).toHaveAttribute("data-tone", "confirmed");
    expect(chips[0].className).toBe(chips[1].className);
    expect(chips[2]).toHaveAttribute("data-tone", "draft");
    expect(chips[3]).toHaveAttribute("data-tone", "draft");
    expect(chips[2].className).toBe(chips[3].className);
    // 不同状态样式必须可区分
    expect(chips[0].className).not.toBe(chips[2].className);
    // 未知状态回落中性
    expect(chips[4]).toHaveAttribute("data-tone", "neutral");
  });

  // chips 的数据类元素（id/version）不随 kind 变色——kind 色只属于
  // 边框/图标/标签等结构元素，story 与 design 的 chips 视觉完全同构。
  it("keeps id and version chips neutral instead of kind-tinted", () => {
    const cards = [
      { ...lifecycleCard("story_spec", "故事一"), version: 3 },
      { ...lifecycleCard("design_spec", "设计一"), version: 3 },
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

    const idChips = screen.getAllByTestId("lifecycle-card-id-chip");
    const versionChips = screen.getAllByTestId("lifecycle-card-version-chip");
    expect(idChips[0].className).toBe(idChips[1].className);
    expect(versionChips[0].className).toBe(versionChips[1].className);
    expect(versionChips[0].className).not.toMatch(/emerald|violet|sky|amber/);
    expect(versionChips[0]).toHaveTextContent("v3");
    expect(versionChips[1]).toHaveTextContent("v3");
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

  it("正常态不渲染 aria-busy（ARIA 惯例：省略即不忙）", () => {
    render(
      <LifecycleCard
        card={lifecycleCard("story_spec", "会话过期提示")}
        selected={false}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByTestId("lifecycle-card-story_spec")).not.toHaveAttribute(
      "aria-busy",
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

  it("renders work item kind and waiting reason on work item cards", () => {
    const backend = workItemRaw({
      work_item_id: "work_item_0001",
      title: "后端 API",
      kind: "backend",
      execution_status: "pending",
      depends_on: [],
    });
    const frontend = lifecycleCard(
      "work_item",
      "前端 UI",
      workItemRaw({
        work_item_id: "work_item_0002",
        title: "前端 UI",
        kind: "frontend",
        execution_status: "pending",
        depends_on: ["work_item_0001"],
      }),
    );

    render(
      <LifecycleCard
        card={frontend}
        selected={false}
        onSelect={vi.fn()}
        allWorkItems={[backend, frontend.raw as LifecycleWorkItem]}
      />,
    );

    expect(screen.getByText("前端")).toBeInTheDocument();
    expect(screen.getByText(/等待依赖完成：后端 API/)).toBeInTheDocument();
  });

  // F-26b：timeline reviewer_run 证据（review_status）必须映射到生命周期卡片——
  // 用户从 issue 工作台即可看到「review 已做/进行中」，不必进会话页翻 timeline。
  it("projects workspace review evidence onto spec and plan cards", () => {
    const storyCard = lifecycleCard("story_spec", "登录会话过期提示");
    const storyView = render(
      <LifecycleCard
        card={
          {
            ...storyCard,
            raw: { ...(storyCard.raw as StorySpec), review_status: "completed" },
          } as LifecycleCardData
        }
        selected={false}
        onSelect={() => undefined}
        onDelete={() => undefined}
      />,
    );
    expect(within(storyView.container).getByTestId("lifecycle-card-review-status").textContent).toBe(
      "Review 已完成",
    );

    const designCard = lifecycleCard("design_spec", "会话管理设计");
    const designView = render(
      <LifecycleCard
        card={
          {
            ...designCard,
            raw: { ...(designCard.raw as DesignSpec), review_status: "running" },
          } as LifecycleCardData
        }
        selected={false}
        onSelect={() => undefined}
        onDelete={() => undefined}
      />,
    );
    expect(within(designView.container).getByTestId("lifecycle-card-review-status").textContent).toBe(
      "Review 进行中",
    );

    const planCard = lifecycleCard("work_item_group", "Work Item Group");
    const planView = render(
      <LifecycleCard
        card={
          {
            ...planCard,
            raw: { ...workItemPlanRaw(), review_status: "completed" },
          } as LifecycleCardData
        }
        selected={false}
        onSelect={() => undefined}
        onDelete={() => undefined}
      />,
    );
    expect(within(planView.container).getByTestId("lifecycle-card-review-status").textContent).toBe(
      "Review 已完成",
    );

    // 无 review 证据的 spec 卡不渲染指示（零证据零展示）。
    const bareView = render(
      <LifecycleCard
        card={lifecycleCard("story_spec", "无审核故事")}
        selected={false}
        onSelect={() => undefined}
        onDelete={() => undefined}
      />,
    );
    expect(within(bareView.container).queryByTestId("lifecycle-card-review-status")).toBeNull();
  });
});

function lifecycleCard(
  kind: LifecycleCardData["kind"],
  title: string,
  rawOverride?: Partial<LifecycleWorkItem>,
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
        title,
        current_version: null,
        current_markdown_preview: "摘要",
        confirmation_status: "draft",
        artifact_versions: [],
      },
    };
  }

  if (kind === "work_item_group") {
    return {
      ...base,
      kind,
      artifactVersions: [],
      childWorkItemIds: ["work_item_0001"],
      raw: workItemPlanRaw(),
    };
  }

  return {
    ...base,
    kind,
    artifactVersions: [],
    raw: workItemRaw({
      work_item_id: "work_item_0001",
      issue_id: "issue_0001",
      repository_id: "repository_0001",
      story_spec_ids: ["story_spec_0001"],
      design_spec_ids: ["design_spec_0001"],
      title,
      plan_status: "draft",
      execution_status: "pending",
      latest_attempt: null,
      artifact_versions: [],
      ...rawOverride,
    }),
  };
}

function workItemPlanRaw(
  overrides: Partial<IssueWorkItemPlanDetailDto> = {},
): IssueWorkItemPlanDetailDto {
  return {
    id: "issue_work_item_plan_0001",
    issue_id: "issue_0001",
    project_id: "project_0001",
    status: "draft",
    source_story_spec_ids: ["story_spec_0001"],
    source_design_spec_ids: ["design_spec_0001"],
    work_item_ids: ["work_item_0001"],
    verification_plan_ids: [],
    dependency_graph: [],
    repository_profile_ref: null,
    options: {
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    },
    validator_findings: [],
    created_at: "2026-05-25T00:00:00Z",
    updated_at: "2026-05-25T00:00:00Z",
    ...overrides,
  };
}

function workItemRaw(
  overrides: Partial<LifecycleWorkItem> = {},
): LifecycleWorkItem {
  return {
    work_item_id: "work_item_0001",
    issue_id: "issue_0001",
    repository_id: "repository_0001",
    story_spec_ids: ["story_spec_0001"],
    design_spec_ids: ["design_spec_0001"],
    title: "Work Item",
    plan_status: "draft",
    execution_status: "pending",
    latest_attempt: null,
    artifact_versions: [],
    work_item_set_id: null,
    kind: "backend",
    sequence_hint: null,
    depends_on: [],
    exclusive_write_scopes: [],
    forbidden_write_scopes: [],
    context_budget: {
      target_context_k: "30-50",
      max_summary_chars: 20000,
      max_code_context_chars: 30000,
      max_context_file_refs: 80,
      max_traceability_refs: 40,
    },
    verification_plan_ref: null,
    require_execution_plan_confirm: false,
    execution_plan_status: "not_started",
    completion_commit: null,
    completion_diff_summary_ref: null,
    ...overrides,
  };
}
