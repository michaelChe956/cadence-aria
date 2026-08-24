// Task 1（队列派生层）：deriveIssueQueue 的表驱动单测。
// Fixture 仿写自 IssueLifecycleWorkbench.test-data.ts 的 lifecycle 响应构造方式
// （本文件自包含 builder，不 import 其 mock fetch）。
import { describe, expect, it } from "vitest";
import type {
  CodingAttempt,
  CodingAttemptStatus,
  DesignSpec,
  IssueLifecycleResponse,
  LifecycleWorkItem,
  ProductIssue,
  StorySpec,
} from "../../api/types";
import {
  defaultCollapsedGroups,
  deriveIssueQueue,
  ISSUE_QUEUE_GROUP_ORDER,
  type IssueQueueGroupKey,
  type IssueStageKey,
  type StagePipState,
} from "./issue-queue-derivation";

// CodingAttempt.status 实际枚举（web/src/api/types/coding.ts）中的冻结取值：
// 终态成功值只有 "completed"；"waiting_for_human" 是进行中、非终态成功值。
const TERMINAL_SUCCESS_STATUS: CodingAttemptStatus = "completed";
const NON_TERMINAL_STATUS: CodingAttemptStatus = "waiting_for_human";
const RUNNING_STATUS: CodingAttemptStatus = "running";

function productIssueRecord(overrides: Partial<ProductIssue> = {}): ProductIssue {
  return {
    issue_id: "issue_0001",
    project_id: "project_0001",
    repo_id: "repository_0001",
    workspace_id: null,
    task_id: null,
    session_id: null,
    title: "会话过期提示",
    description: "描述",
    change_id: "issue_0001-change",
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    artifacts: [],
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
    ...overrides,
  };
}

function storySpecRecord(overrides: Partial<StorySpec> = {}): StorySpec {
  return {
    story_spec_id: "story_spec_0001",
    issue_id: "issue_0001",
    repository_id: "repository_0001",
    title: "会话过期提示",
    current_version: 1,
    current_markdown_preview: "## 功能需求\n\n[REQ-001] 显示会话过期提示。",
    confirmation_status: "confirmed",
    artifact_versions: [],
    ...overrides,
  };
}

function designSpecRecord(overrides: Partial<DesignSpec> = {}): DesignSpec {
  return {
    design_spec_id: "design_spec_0001",
    issue_id: "issue_0001",
    story_spec_ids: ["story_spec_0001"],
    title: "前端提示设计",
    current_version: 1,
    current_markdown_preview: "## 关键决策\n\n[DEC-001] 使用全局提示条。",
    confirmation_status: "confirmed",
    artifact_versions: [],
    ...overrides,
  };
}

function workItemRecord(
  overrides: Partial<LifecycleWorkItem> = {},
): LifecycleWorkItem {
  return {
    work_item_id: "work_item_0001",
    issue_id: "issue_0001",
    repository_id: "repository_0001",
    story_spec_ids: ["story_spec_0001"],
    design_spec_ids: ["design_spec_0001"],
    title: "实现提示组件",
    plan_status: "draft",
    execution_status: "planning",
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

function attemptRecord(
  workItemId: string,
  status: CodingAttemptStatus,
): CodingAttempt {
  return {
    project_id: "project_0001",
    issue_id: "issue_0001",
    attempt_id: `coding_attempt_${workItemId}`,
    work_item_id: workItemId,
    attempt_scope: "work_item",
    work_item_group_id: null,
    current_work_item_id: workItemId,
    active_unit_id: null,
    attempt_no: 1,
    status,
    stage: "coding",
    branch_name: `aria/work-items/${workItemId}/attempt-1`,
    base_branch: "main",
    worktree_path: null,
    rework_count: 0,
    head_commit: null,
    push_status: null,
    review_request_url: null,
    created_at: "2026-05-23T00:00:00Z",
    updated_at: "2026-05-23T00:00:00Z",
  };
}

function lifecycleRecord(
  overrides: {
    issue?: Partial<ProductIssue>;
    storySpecs?: StorySpec[];
    designSpecs?: DesignSpec[];
    workItems?: LifecycleWorkItem[];
  } = {},
): IssueLifecycleResponse {
  return {
    issue: productIssueRecord(overrides.issue),
    story_specs: overrides.storySpecs ?? [storySpecRecord()],
    design_specs: overrides.designSpecs ?? [designSpecRecord()],
    work_item_plans: [],
    work_items: overrides.workItems ?? [workItemRecord()],
    work_item_repository_groups: [],
    workspace_sessions: [],
    coding_attempts: [],
  };
}

// 一对存在依赖关系的 work item：frontend 等待 backend 完成。
function dependencyPairWorkItems(): LifecycleWorkItem[] {
  return [
    workItemRecord({
      work_item_id: "work_item_backend",
      title: "后端 API",
      execution_status: "pending",
      latest_attempt: null,
    }),
    workItemRecord({
      work_item_id: "work_item_frontend",
      title: "前端 UI",
      execution_status: "pending",
      latest_attempt: null,
      depends_on: ["work_item_backend"],
    }),
  ];
}

type GroupBoundaryCase = {
  name: string;
  lifecycle: IssueLifecycleResponse;
  expectedGroup: IssueQueueGroupKey;
  expectedPips: Record<IssueStageKey, StagePipState>;
};

const GROUP_BOUNDARY_CASES: GroupBoundaryCase[] = [
  {
    name: "story 缺失 -> needs_story（story pip active，其余 pending）",
    lifecycle: lifecycleRecord({
      storySpecs: [],
      designSpecs: [],
      workItems: [],
    }),
    expectedGroup: "needs_story",
    expectedPips: {
      story: "active",
      design: "pending",
      work_item: "pending",
      coding: "pending",
    },
  },
  {
    name: "design 缺失 -> needs_design（design pip active）",
    lifecycle: lifecycleRecord({ designSpecs: [], workItems: [] }),
    expectedGroup: "needs_design",
    expectedPips: {
      story: "done",
      design: "active",
      work_item: "pending",
      coding: "pending",
    },
  },
  {
    name: "work item 缺失 -> needs_work_item（work_item pip active）",
    lifecycle: lifecycleRecord({ workItems: [] }),
    expectedGroup: "needs_work_item",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "active",
      coding: "pending",
    },
  },
  {
    name: "work item 等待依赖 -> blocked（work_item/coding pip blocked）",
    lifecycle: lifecycleRecord({ workItems: dependencyPairWorkItems() }),
    expectedGroup: "blocked",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "blocked",
      coding: "blocked",
    },
  },
  {
    name: "attempt 自身 status=blocked -> blocked（无需依赖等待）",
    lifecycle: lifecycleRecord({
      workItems: [
        workItemRecord({
          depends_on: [],
          latest_attempt: attemptRecord("work_item_0001", "blocked"),
        }),
      ],
    }),
    expectedGroup: "blocked",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "blocked",
      coding: "blocked",
    },
  },
  {
    name: "attempt running（正在编码）-> blocked",
    lifecycle: lifecycleRecord({
      workItems: [
        workItemRecord({
          execution_status: "coding",
          latest_attempt: attemptRecord("work_item_0001", RUNNING_STATUS),
        }),
      ],
    }),
    expectedGroup: "blocked",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "blocked",
      coding: "blocked",
    },
  },
  {
    name: "attempt 非终态成功 -> coding（coding pip active）",
    lifecycle: lifecycleRecord({
      workItems: [
        workItemRecord({
          execution_status: "coding",
          latest_attempt: attemptRecord("work_item_0001", NON_TERMINAL_STATUS),
        }),
      ],
    }),
    expectedGroup: "coding",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "done",
      coding: "active",
    },
  },
  {
    name: "latest_attempt 缺失且无等待原因 -> coding",
    lifecycle: lifecycleRecord({
      workItems: [
        workItemRecord({ execution_status: "pending", latest_attempt: null }),
      ],
    }),
    expectedGroup: "coding",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "done",
      coding: "active",
    },
  },
  {
    name: "部分 attempt 非终态成功 -> coding（非 completed）",
    lifecycle: lifecycleRecord({
      workItems: [
        workItemRecord({
          work_item_id: "work_item_backend",
          execution_status: "completed",
          latest_attempt: attemptRecord(
            "work_item_backend",
            TERMINAL_SUCCESS_STATUS,
          ),
        }),
        workItemRecord({
          work_item_id: "work_item_frontend",
          execution_status: "coding",
          latest_attempt: attemptRecord(
            "work_item_frontend",
            NON_TERMINAL_STATUS,
          ),
          depends_on: ["work_item_backend"],
        }),
      ],
    }),
    expectedGroup: "coding",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "done",
      coding: "active",
    },
  },
  {
    name: "所有 attempt 均为终态成功 -> completed（coding pip done）",
    lifecycle: lifecycleRecord({
      workItems: [
        workItemRecord({
          work_item_id: "work_item_backend",
          execution_status: "completed",
          latest_attempt: attemptRecord(
            "work_item_backend",
            TERMINAL_SUCCESS_STATUS,
          ),
        }),
        workItemRecord({
          work_item_id: "work_item_frontend",
          execution_status: "completed",
          latest_attempt: attemptRecord(
            "work_item_frontend",
            TERMINAL_SUCCESS_STATUS,
          ),
          depends_on: ["work_item_backend"],
        }),
      ],
    }),
    expectedGroup: "completed",
    expectedPips: {
      story: "done",
      design: "done",
      work_item: "done",
      coding: "done",
    },
  },
  {
    name: "优先级冲突：design 缺失且 work item 等待依赖 -> 仍 needs_design",
    lifecycle: lifecycleRecord({
      designSpecs: [],
      workItems: dependencyPairWorkItems(),
    }),
    expectedGroup: "needs_design",
    expectedPips: {
      story: "done",
      design: "active",
      work_item: "done",
      coding: "pending",
    },
  },
  {
    name: "优先级冲突：story 缺失且 work item 等待依赖 -> 仍 needs_story",
    lifecycle: lifecycleRecord({
      storySpecs: [],
      workItems: dependencyPairWorkItems(),
    }),
    expectedGroup: "needs_story",
    expectedPips: {
      story: "active",
      design: "done",
      work_item: "done",
      coding: "pending",
    },
  },
];

describe.each(GROUP_BOUNDARY_CASES)("分组边界：$name", (testCase) => {
  it(`归类为 ${testCase.expectedGroup} 并生成对应 stagePips`, () => {
    const groups = deriveIssueQueue([testCase.lifecycle]);

    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe(testCase.expectedGroup);
    expect(groups[0].total).toBe(1);
    expect(groups[0].rows).toHaveLength(1);

    const [row] = groups[0].rows;
    expect(row.group).toBe(testCase.expectedGroup);
    expect(
      row.stagePips.map((pip) => `${pip.stage}:${pip.state}`),
    ).toEqual([
      `story:${testCase.expectedPips.story}`,
      `design:${testCase.expectedPips.design}`,
      `work_item:${testCase.expectedPips.work_item}`,
      `coding:${testCase.expectedPips.coding}`,
    ]);
  });
});

describe("deriveIssueQueue 行数据", () => {
  it("透出 issue 的 id/title/status 与产物计数", () => {
    const groups = deriveIssueQueue([lifecycleRecord()]);

    expect(groups).toHaveLength(1);
    const [row] = groups[0].rows;
    expect(row.issueId).toBe("issue_0001");
    expect(row.title).toBe("会话过期提示");
    expect(row.status).toBe("draft");
    expect(row.storyCount).toBe(1);
    expect(row.designCount).toBe(1);
    expect(row.workItemCount).toBe(1);
  });

  it("空输入返回空数组", () => {
    expect(deriveIssueQueue([])).toEqual([]);
  });

  it("多个分组按 ISSUE_QUEUE_GROUP_ORDER 顺序输出且只包含非空分组", () => {
    const groups = deriveIssueQueue([
      lifecycleRecord({
        issue: productIssueRecord({ issue_id: "issue_done", title: "完成项" }),
        workItems: [
          workItemRecord({
            execution_status: "completed",
            latest_attempt: attemptRecord(
              "work_item_0001",
              TERMINAL_SUCCESS_STATUS,
            ),
          }),
        ],
      }),
      lifecycleRecord({
        issue: productIssueRecord({ issue_id: "issue_coding", title: "编码项" }),
      }),
      lifecycleRecord({
        issue: productIssueRecord({ issue_id: "issue_story", title: "新事项" }),
        storySpecs: [],
        designSpecs: [],
        workItems: [],
      }),
      lifecycleRecord({
        issue: productIssueRecord({
          issue_id: "issue_blocked",
          title: "阻塞项",
        }),
        workItems: dependencyPairWorkItems(),
      }),
    ]);

    expect(groups.map((group) => group.key)).toEqual([
      "needs_story",
      "blocked",
      "coding",
      "completed",
    ]);
  });
});

describe("filterText 过滤", () => {
  const lifecycles = [
    lifecycleRecord({
      issue: productIssueRecord({
        issue_id: "issue_0001",
        title: "会话过期提示",
      }),
    }),
    lifecycleRecord({
      issue: productIssueRecord({ issue_id: "issue_0002", title: "Login Flow" }),
    }),
  ];

  it("按 title 大小写不敏感匹配", () => {
    const groups = deriveIssueQueue(lifecycles, { filterText: "LOGIN" });

    expect(groups).toHaveLength(1);
    expect(groups[0].rows.map((row) => row.issueId)).toEqual(["issue_0002"]);
  });

  it("按 issueId 大小写不敏感匹配", () => {
    const groups = deriveIssueQueue(lifecycles, { filterText: "Issue_0001" });

    expect(groups).toHaveLength(1);
    expect(groups[0].rows.map((row) => row.issueId)).toEqual(["issue_0001"]);
  });

  it("中文 title 子串匹配", () => {
    const groups = deriveIssueQueue(lifecycles, { filterText: "过期" });

    expect(groups).toHaveLength(1);
    expect(groups[0].rows.map((row) => row.issueId)).toEqual(["issue_0001"]);
  });

  it("无匹配时返回空数组", () => {
    expect(
      deriveIssueQueue(lifecycles, { filterText: "不存在的关键词" }),
    ).toEqual([]);
  });

  it("空字符串不过滤", () => {
    const groups = deriveIssueQueue(lifecycles, { filterText: "" });

    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe("coding");
    expect(groups[0].rows).toHaveLength(2);
  });

  it("过滤在分组之前应用：被过滤掉的 lifecycle 不产生分组也不计入 total", () => {
    const mixed = [
      lifecycleRecord({
        issue: productIssueRecord({ issue_id: "issue_a", title: "Alpha" }),
      }),
      lifecycleRecord({
        issue: productIssueRecord({ issue_id: "issue_b", title: "Beta" }),
      }),
      lifecycleRecord({
        issue: productIssueRecord({ issue_id: "issue_c", title: "Gamma" }),
        workItems: [
          workItemRecord({
            execution_status: "completed",
            latest_attempt: attemptRecord(
              "work_item_0001",
              TERMINAL_SUCCESS_STATUS,
            ),
          }),
        ],
      }),
    ];

    const groups = deriveIssueQueue(mixed, { filterText: "gamma" });

    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe("completed");
    expect(groups[0].total).toBe(1);
    expect(groups[0].rows.map((row) => row.issueId)).toEqual(["issue_c"]);
  });
});

describe("perGroupLimit 截断", () => {
  function needsStoryLifecycle(index: number): IssueLifecycleResponse {
    const issueId = `issue_${String(index).padStart(4, "0")}`;
    return lifecycleRecord({
      issue: productIssueRecord({ issue_id: issueId, title: `事项 ${index}` }),
      storySpecs: [],
      designSpecs: [],
      workItems: [],
    });
  }

  it("默认上限 50：rows 截断而 total 保留真实总数", () => {
    const lifecycles = Array.from({ length: 60 }, (_, index) =>
      needsStoryLifecycle(index + 1),
    );

    const groups = deriveIssueQueue(lifecycles);

    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe("needs_story");
    expect(groups[0].rows).toHaveLength(50);
    expect(groups[0].total).toBe(60);
    expect(groups[0].rows[0].issueId).toBe("issue_0001");
    expect(groups[0].rows[49].issueId).toBe("issue_0050");
  });

  it("自定义上限：rows 截断、total 为真实值且保持输入顺序", () => {
    const lifecycles = Array.from({ length: 3 }, (_, index) =>
      needsStoryLifecycle(index + 1),
    );

    const groups = deriveIssueQueue(lifecycles, { perGroupLimit: 2 });

    expect(groups).toHaveLength(1);
    expect(groups[0].rows).toHaveLength(2);
    expect(groups[0].total).toBe(3);
    expect(groups[0].rows.map((row) => row.issueId)).toEqual([
      "issue_0001",
      "issue_0002",
    ]);
  });

  it("上限大于总数时不截断", () => {
    const lifecycles = Array.from({ length: 3 }, (_, index) =>
      needsStoryLifecycle(index + 1),
    );

    const groups = deriveIssueQueue(lifecycles, { perGroupLimit: 10 });

    expect(groups[0].rows).toHaveLength(3);
    expect(groups[0].total).toBe(3);
  });
});

describe("分组顺序与默认折叠", () => {
  it("ISSUE_QUEUE_GROUP_ORDER 锁定六个分组的顺序", () => {
    expect(ISSUE_QUEUE_GROUP_ORDER).toEqual([
      "needs_story",
      "needs_design",
      "needs_work_item",
      "blocked",
      "coding",
      "completed",
    ]);
  });

  it("defaultCollapsedGroups 返回 completed", () => {
    expect(defaultCollapsedGroups()).toEqual(["completed"]);
  });

  it("defaultCollapsedGroups 每次返回新数组", () => {
    expect(defaultCollapsedGroups()).not.toBe(defaultCollapsedGroups());
  });
});
