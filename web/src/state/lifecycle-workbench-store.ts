import { create } from "zustand";
import type {
  ArtifactVersion,
  DesignSpec,
  IssueLifecycleResponse,
  IssueWorkItemPlanDetailDto,
  LifecycleWorkItem,
  ProductIssue,
  StorySpec,
  WorkItemKind,
} from "../api/types";

export type LifecycleCard =
  | {
      kind: "issue";
      id: string;
      issueId: string;
      title: string;
      status: string;
      version: number | null;
      preview: string | null;
      sourceIds: string[];
      raw: ProductIssue;
    }
  | {
      kind: "story_spec";
      id: string;
      issueId: string;
      title: string;
      status: string;
      version: number | null;
      preview: string | null;
      sourceIds: string[];
      artifactVersions: ArtifactVersion[];
      raw: StorySpec;
    }
  | {
      kind: "design_spec";
      id: string;
      issueId: string;
      title: string;
      status: string;
      version: number | null;
      preview: string | null;
      sourceIds: string[];
      artifactVersions: ArtifactVersion[];
      raw: DesignSpec;
    }
  | {
      kind: "work_item";
      id: string;
      issueId: string;
      title: string;
      status: string;
      version: number | null;
      preview: string | null;
      sourceIds: string[];
      artifactVersions: ArtifactVersion[];
      raw: LifecycleWorkItem;
    }
  | {
      kind: "work_item_group";
      id: string;
      issueId: string;
      title: string;
      status: string;
      version: number | null;
      preview: string | null;
      sourceIds: string[];
      childWorkItemIds: string[];
      artifactVersions: ArtifactVersion[];
      raw: IssueWorkItemPlanDetailDto;
    };

export type LifecycleColumns = {
  issue: LifecycleCard[];
  story_spec: LifecycleCard[];
  design_spec: LifecycleCard[];
  work_item: LifecycleCard[];
};

export type LifecycleBlockedTarget = "design_spec" | "work_item" | "coding";

export function groupLifecycleCards(lifecycles: IssueLifecycleResponse[]): LifecycleColumns {
  return lifecycles.reduce<LifecycleColumns>(
    (columns, lifecycle) => {
      columns.issue.push({
        kind: "issue",
        id: lifecycle.issue.issue_id,
        issueId: lifecycle.issue.issue_id,
        title: lifecycle.issue.title,
        status: lifecycle.issue.status,
        version: null,
        preview: lifecycle.issue.description,
        sourceIds: [],
        raw: lifecycle.issue,
      });

      lifecycle.story_specs.forEach((story) => {
        columns.story_spec.push({
          kind: "story_spec",
          id: story.story_spec_id,
          issueId: story.issue_id,
          title: story.title,
          status: story.confirmation_status,
          version: story.current_version,
          preview: story.current_markdown_preview,
          sourceIds: [story.issue_id],
          artifactVersions: story.artifact_versions,
          raw: story,
        });
      });

      lifecycle.design_specs.forEach((design) => {
        columns.design_spec.push({
          kind: "design_spec",
          id: design.design_spec_id,
          issueId: design.issue_id,
          title: design.title,
          status: design.confirmation_status,
          version: design.current_version,
          preview: design.current_markdown_preview,
          sourceIds: [...design.story_spec_ids],
          artifactVersions: design.artifact_versions,
          raw: design,
        });
      });

      const workItemPlan = latestIssueWorkItemPlan(lifecycle.work_item_plans ?? []);
      if (workItemPlan) {
        columns.work_item.push({
          kind: "work_item_group",
          id: workItemPlan.id,
          issueId: workItemPlan.issue_id,
          title: "Work Item Group",
          status: workItemPlan.status,
          version: null,
          preview: `${workItemPlan.work_item_ids.length} 个 Work Item`,
          sourceIds: [
            ...workItemPlan.source_story_spec_ids,
            ...workItemPlan.source_design_spec_ids,
          ],
          childWorkItemIds: [...workItemPlan.work_item_ids],
          artifactVersions: [],
          raw: workItemPlan,
        });
      }

      return columns;
    },
    { issue: [], story_spec: [], design_spec: [], work_item: [] },
  );
}

function latestIssueWorkItemPlan(plans: IssueWorkItemPlanDetailDto[]) {
  return plans.reduce<IssueWorkItemPlanDetailDto | null>((latest, plan) => {
    if (!latest) {
      return plan;
    }
    return planTimestamp(plan) >= planTimestamp(latest) ? plan : latest;
  }, null);
}

function planTimestamp(plan: IssueWorkItemPlanDetailDto) {
  const parsed = Date.parse(plan.updated_at || plan.created_at);
  return Number.isNaN(parsed) ? 0 : parsed;
}

export function visibleLifecycle(
  columns: LifecycleColumns,
  focusedIssueId: string | null,
): LifecycleColumns {
  if (!focusedIssueId) {
    return {
      issue: [...columns.issue],
      story_spec: [...columns.story_spec],
      design_spec: [...columns.design_spec],
      work_item: [...columns.work_item],
    };
  }

  return {
    issue: [...columns.issue],
    story_spec: columns.story_spec.filter((card) => card.issueId === focusedIssueId),
    design_spec: columns.design_spec.filter((card) => card.issueId === focusedIssueId),
    work_item: columns.work_item.filter((card) => card.issueId === focusedIssueId),
  };
}

export function lifecycleBlockedReason(
  target: LifecycleBlockedTarget,
  lifecycle: IssueLifecycleResponse,
): string | null {
  if (
    target === "design_spec" &&
    !lifecycle.story_specs.some((story) => story.confirmation_status === "confirmed")
  ) {
    return "需要先确认至少一个 Story Spec";
  }

  if (
    target === "work_item" &&
    !lifecycle.design_specs.some((design) => design.confirmation_status === "confirmed")
  ) {
    return "需要先确认至少一个 Design Spec";
  }

  if (
    target === "coding" &&
    !lifecycle.work_items.some((item) => item.plan_status === "confirmed")
  ) {
    return "需要先确认 Work Item Plan";
  }

  return null;
}

export function workItemKindLabel(kind: WorkItemKind): string {
  switch (kind) {
    case "backend":
      return "后端";
    case "frontend":
      return "前端";
    case "integration":
      return "贯通";
    case "e2e":
      return "E2E";
    case "docs":
      return "文档";
    case "infra":
      return "基础设施";
    case "other":
      return "其他";
  }
}

export function workItemWaitingReason(
  item: LifecycleWorkItem,
  allItems: LifecycleWorkItem[],
): string | null {
  const pendingDependencies = item.depends_on
    .map((id) => allItems.find((candidate) => candidate.work_item_id === id))
    .filter(
      (dependency): dependency is LifecycleWorkItem =>
        dependency !== undefined && dependency.execution_status !== "completed",
    );
  if (pendingDependencies.length > 0) {
    const titles = pendingDependencies.map((dependency) => dependency.title).join("、");
    return `等待依赖完成：${titles}`;
  }

  if (
    item.latest_attempt &&
    ["created", "running"].includes(item.latest_attempt.status)
  ) {
    return "正在编码";
  }

  return null;
}

export interface LifecycleWorkbenchState {
  focusedEntityKey: string | null;
  isDrawerOpen: boolean;
}

export interface LifecycleWorkbenchActions {
  openDrawer: (entityKey: string) => void;
  closeDrawer: () => void;
}

export const useLifecycleWorkbenchStore = create<
  LifecycleWorkbenchState & LifecycleWorkbenchActions
>((set) => ({
  focusedEntityKey: null,
  isDrawerOpen: false,
  openDrawer: (entityKey) =>
    set({ focusedEntityKey: entityKey, isDrawerOpen: true }),
  closeDrawer: () => set({ focusedEntityKey: null, isDrawerOpen: false }),
}));

// ---------------------------------------------------------------------------
// F-29：lifecycle invalidation 总线
// ---------------------------------------------------------------------------
// HTTP confirm（story/design AuthorConfirm、Work Item 执行计划）成功后，服务端
// durable 投影已变化，但 Workbench 的 lifecycles 是 REST 一次性拉取——这里提供
// 进程内 + 跨 tab 的失效通知：confirm 调用点 notifyLifecycleInvalidated(issueId)，
// Workbench 订阅后定向刷新对应 issue 的 lifecycle。同页走进程内监听器
// （BroadcastChannel 不回环到发送方自身），跨 tab 走 'lifecycle-invalidated'
// 频道。环境无 BroadcastChannel 时自动降级为仅同页生效（不引入新库）。

export type LifecycleInvalidationEvent = { issueId: string };

type LifecycleInvalidationMessage = {
  type: "lifecycle-invalidated";
  issueId: string;
};

const LIFECYCLE_INVALIDATION_CHANNEL_NAME = "lifecycle-invalidated";

const lifecycleInvalidationListeners = new Set<
  (event: LifecycleInvalidationEvent) => void
>();
let lifecycleInvalidationChannel: BroadcastChannel | null = null;

function dispatchLifecycleInvalidation(event: LifecycleInvalidationEvent): void {
  for (const listener of [...lifecycleInvalidationListeners]) {
    listener(event);
  }
}

function ensureLifecycleInvalidationChannel(): BroadcastChannel | null {
  if (lifecycleInvalidationChannel) {
    return lifecycleInvalidationChannel;
  }
  if (typeof BroadcastChannel === "undefined") {
    return null;
  }
  const channel = new BroadcastChannel(LIFECYCLE_INVALIDATION_CHANNEL_NAME);
  channel.onmessage = (event: MessageEvent<LifecycleInvalidationMessage>) => {
    const data = event.data;
    if (
      data &&
      data.type === LIFECYCLE_INVALIDATION_CHANNEL_NAME &&
      typeof data.issueId === "string"
    ) {
      dispatchLifecycleInvalidation({ issueId: data.issueId });
    }
  };
  lifecycleInvalidationChannel = channel;
  return channel;
}

function releaseLifecycleInvalidationChannelIfIdle(): void {
  if (lifecycleInvalidationListeners.size > 0) {
    return;
  }
  lifecycleInvalidationChannel?.close();
  lifecycleInvalidationChannel = null;
}

/** confirm 成功后通知 lifecycle 失效（同页立即派发 + 跨 tab 广播）。 */
export function notifyLifecycleInvalidated(issueId: string): void {
  dispatchLifecycleInvalidation({ issueId });
  try {
    ensureLifecycleInvalidationChannel()?.postMessage({
      type: LIFECYCLE_INVALIDATION_CHANNEL_NAME,
      issueId,
    } satisfies LifecycleInvalidationMessage);
  } catch {
    // 跨 tab 广播失败（如受限环境抛错）不影响同页刷新——静默降级。
  }
}

/** 订阅 lifecycle 失效事件；返回取消订阅函数（最后一个订阅者退出时关闭频道）。 */
export function subscribeToLifecycleInvalidation(
  listener: (event: LifecycleInvalidationEvent) => void,
): () => void {
  lifecycleInvalidationListeners.add(listener);
  ensureLifecycleInvalidationChannel();
  return () => {
    lifecycleInvalidationListeners.delete(listener);
    releaseLifecycleInvalidationChannelIfIdle();
  };
}
