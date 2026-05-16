import { describe, expect, it } from "vitest";
import type { IssueLifecycleResponse } from "../api/types";
import {
  groupLifecycleCards,
  lifecycleBlockedReason,
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
      confirmation_status: "confirmed",
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
      confirmation_status: "draft",
    },
  ],
  work_items: [],
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
      confirmation_status: "confirmed",
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
      confirmation_status: "confirmed",
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
  });

  it("filters cards by focused issue", () => {
    const grouped = visibleLifecycle(groupLifecycleCards([lifecycle, otherLifecycle]), "issue_0001");
    expect(grouped.issue.map((card) => card.id)).toEqual(["issue_0001", "issue_0002"]);
    expect(grouped.story_spec.map((card) => card.id)).toEqual(["story_spec_0001"]);
    expect(grouped.design_spec.map((card) => card.id)).toEqual(["design_spec_0001"]);
    expect(grouped.work_item).toHaveLength(0);
  });

  it("returns original columns when no issue is focused", () => {
    const grouped = groupLifecycleCards([lifecycle]);
    expect(visibleLifecycle(grouped, null)).toBe(grouped);
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
