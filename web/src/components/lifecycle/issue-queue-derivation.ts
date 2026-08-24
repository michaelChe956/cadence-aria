// Task 1（队列派生层）：从 IssueLifecycleResponse 列表派生 workbench 队列分组的纯函数。
// 无 React 依赖、无副作用；分组判定与 stagePips 规则与 Plan 逐字一致。
import type { CodingAttempt, IssueLifecycleResponse } from "../../api/types";
import { workItemWaitingReason } from "../../state/lifecycle-workbench-store";

export type IssueStageKey = "story" | "design" | "work_item" | "coding";

export type StagePipState = "done" | "active" | "blocked" | "pending";

export type IssueQueueGroupKey =
  | "needs_story"
  | "needs_design"
  | "needs_work_item"
  | "blocked"
  | "coding"
  | "completed";

export interface StagePip {
  stage: IssueStageKey;
  state: StagePipState;
}

export interface IssueQueueRowData {
  issueId: string;
  title: string;
  status: string;
  stagePips: StagePip[];
  group: IssueQueueGroupKey;
  storyCount: number;
  designCount: number;
  workItemCount: number;
}

export interface IssueQueueGroup {
  key: IssueQueueGroupKey;
  rows: IssueQueueRowData[];
  total: number;
}

export const ISSUE_QUEUE_GROUP_ORDER: IssueQueueGroupKey[] = [
  "needs_story",
  "needs_design",
  "needs_work_item",
  "blocked",
  "coding",
  "completed",
];

// perGroupLimit 默认值：rows 截断到上限，total 保留组内真实总数。
const DEFAULT_PER_GROUP_LIMIT = 50;

// CodingAttempt["status"] 的终态成功集。实现时枚举确认（web/src/api/types/coding.ts）：
// 仅 "completed" 表示成功终态；failed/aborted 为失败或中断，waiting_for_human/blocked 等为进行中态。
const TERMINAL_SUCCESS_ATTEMPT_STATUSES: ReadonlySet<CodingAttempt["status"]> =
  new Set<CodingAttempt["status"]>(["completed"]);

// needs_* 分组对应的“下一个动作”阶段，用于 stagePips 的 active 判定。
const NEXT_ACTION_STAGE_BY_GROUP: Partial<
  Record<IssueQueueGroupKey, IssueStageKey>
> = {
  needs_story: "story",
  needs_design: "design",
  needs_work_item: "work_item",
};

const STAGE_KEYS: IssueStageKey[] = ["story", "design", "work_item", "coding"];

export function defaultCollapsedGroups(): IssueQueueGroupKey[] {
  return ["completed"];
}

export function deriveIssueQueue(
  lifecycles: IssueLifecycleResponse[],
  options?: { filterText?: string; perGroupLimit?: number },
): IssueQueueGroup[] {
  const filterText = options?.filterText ?? "";
  const perGroupLimit = options?.perGroupLimit ?? DEFAULT_PER_GROUP_LIMIT;

  // 过滤在分组之前应用：filterText 非空时大小写不敏感匹配 title 或 issueId。
  const candidates =
    filterText.length > 0
      ? lifecycles.filter((lifecycle) =>
          matchesFilterText(lifecycle, filterText),
        )
      : lifecycles;

  const rowsByGroup = new Map<IssueQueueGroupKey, IssueQueueRowData[]>();
  for (const lifecycle of candidates) {
    const row = deriveIssueQueueRow(lifecycle);
    const bucket = rowsByGroup.get(row.group);
    if (bucket === undefined) {
      rowsByGroup.set(row.group, [row]);
    } else {
      bucket.push(row);
    }
  }

  // 只返回非空分组，按 ISSUE_QUEUE_GROUP_ORDER 排序；
  // rows 截断到 perGroupLimit（组内保持输入顺序），total 保留组内真实总数。
  return ISSUE_QUEUE_GROUP_ORDER.filter((key) => rowsByGroup.has(key)).map(
    (key) => {
      const rows = rowsByGroup.get(key) ?? [];
      return {
        key,
        rows: rows.slice(0, perGroupLimit),
        total: rows.length,
      };
    },
  );
}

function matchesFilterText(
  lifecycle: IssueLifecycleResponse,
  filterText: string,
): boolean {
  const needle = filterText.toLowerCase();
  return (
    lifecycle.issue.title.toLowerCase().includes(needle) ||
    lifecycle.issue.issue_id.toLowerCase().includes(needle)
  );
}

function deriveIssueQueueRow(
  lifecycle: IssueLifecycleResponse,
): IssueQueueRowData {
  const storyCount = lifecycle.story_specs.length;
  const designCount = lifecycle.design_specs.length;
  const workItemCount = lifecycle.work_items.length;
  const group = deriveIssueQueueGroup(lifecycle);

  return {
    issueId: lifecycle.issue.issue_id,
    title: lifecycle.issue.title,
    status: lifecycle.issue.status,
    stagePips: buildStagePips(group, {
      storyCount,
      designCount,
      workItemCount,
    }),
    group,
    storyCount,
    designCount,
    workItemCount,
  };
}

// 分组判定规则（按优先级顺序，命中即停）：
// 1. story_specs.length === 0 -> needs_story
// 2. design_specs.length === 0 -> needs_design
// 3. work_items.length === 0 -> needs_work_item
// 4. 任一 work item 使 workItemWaitingReason(item, allItems) 非 null -> blocked
// 5. 所有 work item 的 latest_attempt 均存在且 status 属于终态成功集 -> completed
// 6. 其余 -> coding
function deriveIssueQueueGroup(
  lifecycle: IssueLifecycleResponse,
): IssueQueueGroupKey {
  if (lifecycle.story_specs.length === 0) {
    return "needs_story";
  }
  if (lifecycle.design_specs.length === 0) {
    return "needs_design";
  }
  const workItems = lifecycle.work_items;
  if (workItems.length === 0) {
    return "needs_work_item";
  }
  if (
    workItems.some((item) => workItemWaitingReason(item, workItems) !== null)
  ) {
    return "blocked";
  }
  if (isAllWorkItemsTerminallySuccessful(workItems)) {
    return "completed";
  }
  return "coding";
}

function isAllWorkItemsTerminallySuccessful(
  workItems: IssueLifecycleResponse["work_items"],
): boolean {
  return workItems.every(
    (item) =>
      item.latest_attempt !== null &&
      TERMINAL_SUCCESS_ATTEMPT_STATUSES.has(item.latest_attempt.status),
  );
}

// stagePips 规则：
// - story/design/work_item：blocked 分组时 work_item pip 为 blocked；否则对应产物数量 > 0
//   -> done；数量 === 0 且该阶段为分组判定的下一个动作 -> active；其余 pending。
// - coding：completed 分组 -> done；coding 分组 -> active；blocked 分组 -> blocked；其余 pending。
function buildStagePips(
  group: IssueQueueGroupKey,
  counts: { storyCount: number; designCount: number; workItemCount: number },
): StagePip[] {
  const nextActionStage = NEXT_ACTION_STAGE_BY_GROUP[group] ?? null;
  return STAGE_KEYS.map((stage) => ({
    stage,
    state: deriveStagePipState(stage, group, counts, nextActionStage),
  }));
}

function deriveStagePipState(
  stage: IssueStageKey,
  group: IssueQueueGroupKey,
  counts: { storyCount: number; designCount: number; workItemCount: number },
  nextActionStage: IssueStageKey | null,
): StagePipState {
  if (stage === "coding") {
    if (group === "completed") {
      return "done";
    }
    if (group === "coding") {
      return "active";
    }
    if (group === "blocked") {
      return "blocked";
    }
    return "pending";
  }

  if (group === "blocked" && stage === "work_item") {
    return "blocked";
  }

  const count =
    stage === "story"
      ? counts.storyCount
      : stage === "design"
        ? counts.designCount
        : counts.workItemCount;

  if (count > 0) {
    return "done";
  }
  if (nextActionStage === stage) {
    return "active";
  }
  return "pending";
}
