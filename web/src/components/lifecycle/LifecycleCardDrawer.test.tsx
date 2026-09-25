import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { LifecycleCardDrawer } from "./LifecycleCardDrawer";
import { LifecycleCard } from "./LifecycleCard";

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

  it("REQ-PIB-01：issue 抽屉呈现锁定的基准分支，非 issue 实体不呈现", () => {
    const { rerender } = render(
      <LifecycleCardDrawer
        entity={{
          id: "issue_0001",
          kind: "issue",
          title: "分支锚定",
          status: "draft",
          version: null,
          baseBranch: "feature/x",
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );
    expect(screen.getByTestId("drawer-base-branch-chip")).toHaveTextContent(
      "基准 feature/x · 锁定",
    );

    rerender(
      <LifecycleCardDrawer
        entity={{
          id: "issue_0002",
          kind: "issue",
          title: "存量 issue",
          status: "draft",
          version: null,
          baseBranch: null,
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );
    expect(screen.queryByTestId("drawer-base-branch-chip")).toBeNull();
  });

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
    expect(screen.queryByTestId("monaco-viewer")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "查看 Markdown 内容" }));

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

  it("renders delivery status panel inside issue detail", () => {
    render(
      <LifecycleCardDrawer
        entity={{
          id: "issue-id",
          kind: "issue",
          title: "登录会话过期",
          status: "draft",
          version: null,
        }}
        deliverySummary={{
          project_id: "project_0001",
          issue_id: "issue-id",
          overall: "partial",
          entries: [
            {
              repository_name: "cadence-aria",
              work_item_id: "work_item_0001",
              attempt_status: "completed",
              branch_name: "feat/delivery",
              commit_sha: "abc123def",
              push_status: "failed",
              push_error: "remote rejected: non-fast-forward",
            },
          ],
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );

    expect(screen.getByTestId("delivery-status-panel")).toBeInTheDocument();
    expect(screen.getByText("部分交付")).toBeInTheDocument();
    expect(
      screen.getByText("remote rejected: non-fast-forward"),
    ).toBeInTheDocument();
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
    expect(screen.queryByTestId("monaco-viewer")).not.toBeInTheDocument();
    expect(
      screen.getByText("默认隐藏长内容，按需打开 Markdown 大预览。"),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "查看 Markdown 内容" }));

    expect(screen.getByText("Issue 描述预览")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveAttribute(
      "data-height",
      "100%",
    );
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent(
      "会话过期后需要提示用户",
    );
    expect(screen.getByText("关联产物")).toBeInTheDocument();
    expect(screen.getAllByText("story_spec")).toHaveLength(2);
    expect(screen.getByText("会话过期提示 Story")).toBeInTheDocument();
    expect(screen.getByText("阶段: clarification")).toBeInTheDocument();
    expect(screen.getByText("创建时间: 2026-05-16")).toBeInTheDocument();
  });

  it("renders work item scopes and budget in drawer", () => {
    render(
      <LifecycleCardDrawer
        entity={{
          id: "work_item_0001",
          kind: "work_item",
          title: "后端 API",
          status: "pending",
          version: 1,
          workItemKind: "backend",
          exclusiveWriteScopes: ["src/product/**"],
          forbiddenWriteScopes: ["web/**"],
          contextBudget: {
            target_context_k: "30-50",
            max_summary_chars: 20000,
            max_code_context_chars: 30000,
            max_context_file_refs: 80,
            max_traceability_refs: 40,
          },
          allWorkItems: [],
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );

    expect(screen.getByText("src/product/**")).toBeInTheDocument();
    expect(screen.getByText("web/**")).toBeInTheDocument();
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

    expect(screen.queryByTestId("monaco-viewer")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "查看 Markdown 内容" }));

    expect(screen.getByText("版本 v2 预览")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("新增验收标准");

    fireEvent.click(screen.getByRole("button", { name: /v1/ }));

    expect(screen.getByText("版本 v1 预览")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("基础需求");
    fireEvent.click(screen.getByRole("button", { name: "与最新版本对比" }));

    expect(screen.getByTestId("version-diff-original")).toHaveTextContent("# v1");
    expect(screen.getByTestId("version-diff-modified")).toHaveTextContent("# v2");
  });

  it("renders plan group projection panel for work_item_group and hides it when absent", () => {
    // REQ-MTG-04（WP3）：plan 卡片区（work_item_group 抽屉）呈现聚合只读投影；
    // 旧响应无 group_projection（缺省）时不渲染——additive 兼容。
    const { rerender } = render(
      <LifecycleCardDrawer
        entity={{
          id: "plan-1",
          kind: "work_item_group",
          title: "Work Item Group",
          status: "confirmed",
          version: null,
          groupProjection: {
            plan_id: "plan-1",
            overall: "partial",
            entries: [
              {
                target_repository_id: "11111111-1111-1111-1111-111111111111",
                repository_name: "checkout-alpha",
                attempt_id: "coding_attempt_0001",
                attempt_status: "completed",
                stage: "final_confirm",
                branch_name: "aria/issues/issue_0001/checkout-alpha",
                head_commit: "sha1111",
                push_status: "pushed",
                review_request_id: "review_request_0001",
                blocked_reason: null,
              },
              {
                target_repository_id: "22222222-2222-2222-2222-222222222222",
                repository_name: "checkout-beta",
                attempt_id: "coding_attempt_0002",
                attempt_status: "failed",
                stage: "coding",
                branch_name: "aria/issues/issue_0001/checkout-beta",
                head_commit: null,
                push_status: null,
                review_request_id: null,
                blocked_reason: "coder crashed",
              },
            ],
          },
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );

    expect(screen.getByTestId("plan-group-projection-panel")).toBeInTheDocument();
    expect(screen.getByTestId("plan-group-overall-badge")).toHaveAttribute(
      "data-status",
      "partial",
    );
    expect(screen.getByText("checkout-alpha")).toBeInTheDocument();
    expect(screen.getByText(/coder crashed/)).toBeInTheDocument();

    rerender(
      <LifecycleCardDrawer
        entity={{
          id: "plan-1",
          kind: "work_item_group",
          title: "Work Item Group",
          status: "confirmed",
          version: null,
          groupProjection: null,
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );

    expect(
      screen.queryByTestId("plan-group-projection-panel"),
    ).not.toBeInTheDocument();
  });
  // F-26b：spec/plan 详情 overview 头部展示 workspace review 证据——用户在
  // issue 面打开抽屉即可确认「review 已做/进行中」。
  it("projects workspace review evidence in the drawer header", () => {
    const { rerender } = render(
      <LifecycleCardDrawer
        entity={{
          id: "story-1",
          kind: "story_spec",
          title: "用户认证模块",
          status: "confirmed",
          version: 2,
          reviewStatus: "running",
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );
    expect(screen.getByTestId("drawer-review-status").textContent).toBe(
      "Review 进行中",
    );

    rerender(
      <LifecycleCardDrawer
        entity={{
          id: "story-1",
          kind: "story_spec",
          title: "用户认证模块",
          status: "confirmed",
          version: 2,
          reviewStatus: "completed",
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );
    expect(screen.getByTestId("drawer-review-status").textContent).toBe(
      "Review 已完成",
    );

    // 无 review 证据不渲染指示。
    rerender(
      <LifecycleCardDrawer
        entity={{
          id: "story-1",
          kind: "story_spec",
          title: "用户认证模块",
          status: "confirmed",
          version: 2,
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );
    expect(screen.queryByTestId("drawer-review-status")).not.toBeInTheDocument();
  });
  // 统一视觉语言：抽屉 header chips 与卡片 chips 同构——同一状态在两个
  // 表面使用相同文案、tone 与样式类；chips 顺序与卡片一致（id→status→
  // version→review），kind 图标色跟随实体 kind（与卡片图标同色系）。
  it("unifies drawer header chips with card chips: same tone, classes, and order", () => {
    render(
      <div>
        <LifecycleCard
          card={{
            kind: "design_spec",
            id: "design_spec_0001",
            issueId: "issue_0001",
            title: "设计卡片",
            status: "confirmed",
            version: 2,
            preview: null,
            sourceIds: [],
            artifactVersions: [],
            raw: {
              design_spec_id: "design_spec_0001",
              issue_id: "issue_0001",
              story_spec_ids: [],
              title: "设计卡片",
              current_version: 2,
              current_markdown_preview: null,
              confirmation_status: "confirmed",
              artifact_versions: [],
            },
          }}
          selected={false}
          onSelect={vi.fn()}
        />
        <LifecycleCardDrawer
          entity={{
            id: "design_spec_0001",
            kind: "design_spec",
            title: "设计抽屉",
            status: "confirmed",
            version: 2,
            reviewStatus: "completed",
          }}
          onClose={vi.fn()}
          onOpenWorkspace={vi.fn()}
        />
      </div>,
    );

    const cardStatus = screen.getByTestId("lifecycle-card-status-chip");
    const drawerStatus = screen.getByTestId("drawer-status-chip");
    expect(drawerStatus).toHaveTextContent("已确认");
    expect(drawerStatus).toHaveAttribute("data-tone", "confirmed");
    expect(drawerStatus.className).toBe(cardStatus.className);

    const cardVersion = screen.getByTestId("lifecycle-card-version-chip");
    const drawerVersion = screen.getByTestId("drawer-version-chip");
    expect(drawerVersion).toHaveTextContent("v2");
    expect(drawerVersion.className).toBe(cardVersion.className);

    // chips 顺序与卡片一致：id → status → version → review
    const chipsRow = drawerStatus.parentElement as HTMLElement;
    const order = [...chipsRow.children].map(
      (chip) => chip.getAttribute("data-testid") ?? chip.textContent,
    );
    expect(order).toEqual([
      "drawer-id-chip",
      "drawer-status-chip",
      "drawer-version-chip",
      "drawer-review-status",
    ]);
  });

  it("colors the drawer kind icon with the entity kind palette like the card", () => {
    const { rerender } = render(
      <LifecycleCardDrawer
        entity={{
          id: "story-1",
          kind: "story_spec",
          title: "故事",
          status: "confirmed",
          version: 1,
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );
    const storyIcon = screen
      .getByTestId("lifecycle-card-drawer")
      .querySelector("header svg");
    expect(storyIcon?.getAttribute("class")).toContain("text-emerald-700");

    rerender(
      <LifecycleCardDrawer
        entity={{
          id: "design-1",
          kind: "design_spec",
          title: "设计",
          status: "confirmed",
          version: 1,
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
      />,
    );
    const designIcon = screen
      .getByTestId("lifecycle-card-drawer")
      .querySelector("header svg");
    expect(designIcon?.getAttribute("class")).toContain("text-violet-700");
  });


});