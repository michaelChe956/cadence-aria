import { create } from "zustand";
import type {
  ArtifactVersion,
  DesignSpec,
  IssueLifecycleResponse,
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

      lifecycle.work_items.forEach((item) => {
        const artifactVersions = item.artifact_versions ?? [];
        columns.work_item.push({
          kind: "work_item",
          id: item.work_item_id,
          issueId: item.issue_id,
          title: item.title,
          status: item.execution_status,
          version: latestArtifactVersion(artifactVersions),
          preview: null,
          sourceIds: [...item.story_spec_ids, ...item.design_spec_ids],
          artifactVersions,
          raw: item,
        });
      });

      return columns;
    },
    { issue: [], story_spec: [], design_spec: [], work_item: [] },
  );
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

  const missingHandoffs = item.required_handoff_from
    .map((id) => allItems.find((candidate) => candidate.work_item_id === id))
    .filter(
      (dependency): dependency is LifecycleWorkItem =>
        dependency !== undefined && dependency.handoff_summary_ref === null,
    );
  if (missingHandoffs.length > 0) {
    const titles = missingHandoffs.map((dependency) => dependency.title).join("、");
    return `等待交接摘要：${titles}`;
  }

  if (
    item.latest_attempt &&
    ["created", "running"].includes(item.latest_attempt.status)
  ) {
    return "正在编码";
  }

  return null;
}

function latestArtifactVersion(versions: ArtifactVersion[]): number | null {
  if (versions.length === 0) {
    return null;
  }
  return Math.max(...versions.map((version) => version.version));
}

export interface LifecycleWorkbenchState {
  focusedEntityId: string | null;
  isDrawerOpen: boolean;
}

export interface LifecycleWorkbenchActions {
  openDrawer: (entityId: string) => void;
  closeDrawer: () => void;
}

export const useLifecycleWorkbenchStore = create<
  LifecycleWorkbenchState & LifecycleWorkbenchActions
>((set) => ({
  focusedEntityId: null,
  isDrawerOpen: false,
  openDrawer: (entityId) => set({ focusedEntityId: entityId, isDrawerOpen: true }),
  closeDrawer: () => set({ focusedEntityId: null, isDrawerOpen: false }),
}));
