import { beforeEach, describe, expect, it } from "vitest";
import type { IssueLifecycleResponse } from "../api/types";
import {
  groupLifecycleCards,
  lifecycleBlockedReason,
  useLifecycleWorkbenchStore,
  visibleLifecycle,
} from "./lifecycle-workbench-store";

const lifecycle: IssueLifecycleResponse = {
  issue: {
    issue_id: "issue_0001",
    project_id: "project_0001",
    repo_id: "repository_0001",
    workspace_id: null,
    task_id: null,
    session_id: null,
    title: "登录会话过期",
    description: "描述",
    change_id: "login-session-expired",
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    artifacts: [],
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  },
  story_specs: [
    {
      story_spec_id: "story_spec_0001",
      issue_id: "issue_0001",
      repository_id: "repository_0001",
      title: "会话过期提示",
      current_version: 1,
      current_markdown_preview: "## 功能需求\n\n[REQ-001] 显示会话过期提示。",
      confirmation_status: "confirmed",
      artifact_versions: [
        {
          version: 1,
          markdown: "## 功能需求\n\n[REQ-001] 显示会话过期提示。",
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "pass",
          confirmed_by: "human",
          created_at: "2026-05-20T00:00:00Z",
          source_node_id: "timeline_node_story_001",
        },
      ],
    },
  ],
  design_specs: [
    {
      design_spec_id: "design_spec_0001",
      issue_id: "issue_0001",
      story_spec_ids: ["story_spec_0001"],
      design_kind: "frontend",
      title: "前端提示设计",
      current_version: 1,
      current_markdown_preview: "## 关键决策\n\n[DEC-001] 使用全局提示条。",
      confirmation_status: "draft",
      artifact_versions: [
        {
          version: 1,
          markdown: "## 关键决策\n\n[DEC-001] 使用全局提示条。",
          generated_by: "claude_code",
          reviewed_by: "codex",
          review_verdict: "revise",
          confirmed_by: null,
          created_at: "2026-05-20T00:01:00Z",
          source_node_id: "timeline_node_design_001",
        },
      ],
    },
  ],
  work_items: [],
  workspace_sessions: [],
  coding_attempts: [],
};

const otherLifecycle: IssueLifecycleResponse = {
  ...lifecycle,
  issue: {
    ...lifecycle.issue,
    issue_id: "issue_0002",
    title: "注册验证码",
  },
  story_specs: [
    {
      story_spec_id: "story_spec_0002",
      issue_id: "issue_0002",
      repository_id: "repository_0001",
      title: "验证码提示",
      current_version: 1,
      current_markdown_preview: "## 功能需求\n\n[REQ-001] 显示验证码提示。",
      confirmation_status: "confirmed",
      artifact_versions: [],
    },
  ],
  design_specs: [
    {
      design_spec_id: "design_spec_0002",
      issue_id: "issue_0002",
      story_spec_ids: ["story_spec_0002"],
      design_kind: "backend",
      title: "验证码服务设计",
      current_version: 1,
      current_markdown_preview: "## 关键决策\n\n[DEC-001] 提供验证码 API。",
      confirmation_status: "confirmed",
      artifact_versions: [],
    },
  ],
  work_items: [
    {
      work_item_id: "work_item_0002",
      issue_id: "issue_0002",
      repository_id: "repository_0001",
      story_spec_ids: ["story_spec_0002"],
      design_spec_ids: ["design_spec_0002"],
      title: "实现验证码服务",
      plan_status: "confirmed",
      execution_status: "pending",
      latest_attempt: null,
    },
  ],
};

describe("lifecycle workbench store", () => {
  it("groups lifecycle response into four columns", () => {
    const grouped = groupLifecycleCards([lifecycle]);
    expect(grouped.issue).toHaveLength(1);
    expect(grouped.story_spec).toHaveLength(1);
    expect(grouped.design_spec).toHaveLength(1);
    expect(grouped.work_item).toHaveLength(0);
    expect(grouped.story_spec[0].preview).toContain("[REQ-001]");
    expect(grouped.design_spec[0].preview).toContain("[DEC-001]");
  });

  it("copies design spec source ids from lifecycle story spec ids", () => {
    const grouped = groupLifecycleCards([lifecycle]);

    grouped.design_spec[0].sourceIds.push("story_spec_mutated");

    expect(lifecycle.design_specs[0].story_spec_ids).toEqual(["story_spec_0001"]);
  });

  it("passes story and design artifact versions through card data", () => {
    const grouped = groupLifecycleCards([lifecycle]);
    const storyCard = grouped.story_spec[0];
    const designCard = grouped.design_spec[0];

    expect(storyCard.kind).toBe("story_spec");
    expect(designCard.kind).toBe("design_spec");
    if (storyCard.kind !== "story_spec" || designCard.kind !== "design_spec") {
      throw new Error("unexpected card kind");
    }
    expect(storyCard.artifactVersions).toEqual(lifecycle.story_specs[0].artifact_versions);
    expect(designCard.artifactVersions).toEqual(lifecycle.design_specs[0].artifact_versions);
  });

  it("filters cards by focused issue", () => {
    const grouped = groupLifecycleCards([lifecycle, otherLifecycle]);
    const visible = visibleLifecycle(grouped, "issue_0001");

    expect(visible).not.toBe(grouped);
    expect(visible.issue).not.toBe(grouped.issue);
    expect(visible.story_spec).not.toBe(grouped.story_spec);
    expect(visible.design_spec).not.toBe(grouped.design_spec);
    expect(visible.work_item).not.toBe(grouped.work_item);
    expect(visible.issue.map((card) => card.id)).toEqual(["issue_0001", "issue_0002"]);
    expect(visible.story_spec.map((card) => card.id)).toEqual(["story_spec_0001"]);
    expect(visible.design_spec.map((card) => card.id)).toEqual(["design_spec_0001"]);
    expect(visible.work_item).toHaveLength(0);
  });

  it("returns copied columns when no issue is focused", () => {
    const grouped = groupLifecycleCards([lifecycle]);
    const visible = visibleLifecycle(grouped, null);

    expect(visible).toEqual(grouped);
    expect(visible).not.toBe(grouped);
    expect(visible.issue).not.toBe(grouped.issue);
    expect(visible.story_spec).not.toBe(grouped.story_spec);
    expect(visible.design_spec).not.toBe(grouped.design_spec);
    expect(visible.work_item).not.toBe(grouped.work_item);
  });

  it("blocks work item generation until design is confirmed", () => {
    expect(lifecycleBlockedReason("work_item", lifecycle)).toBe(
      "需要先确认至少一个 Design Spec",
    );
  });

  it("does not block work item generation after a design is confirmed", () => {
    const confirmedDesign: IssueLifecycleResponse = {
      ...lifecycle,
      design_specs: [
        {
          ...lifecycle.design_specs[0],
          confirmation_status: "confirmed",
        },
      ],
    };

    expect(lifecycleBlockedReason("work_item", confirmedDesign)).toBeNull();
  });
});

describe("drawer state", () => {
  beforeEach(() => {
    useLifecycleWorkbenchStore.setState({
      focusedEntityId: null,
      isDrawerOpen: false,
    });
  });

  it("opens drawer with entity id", () => {
    const store = useLifecycleWorkbenchStore.getState();

    store.openDrawer("story-id");

    expect(useLifecycleWorkbenchStore.getState().focusedEntityId).toBe("story-id");
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(true);
  });

  it("closes drawer and clears focus", () => {
    const store = useLifecycleWorkbenchStore.getState();

    store.openDrawer("story-id");
    store.closeDrawer();

    expect(useLifecycleWorkbenchStore.getState().focusedEntityId).toBeNull();
    expect(useLifecycleWorkbenchStore.getState().isDrawerOpen).toBe(false);
  });
});
