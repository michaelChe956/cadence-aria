import { PanelRightOpen } from "lucide-react";
import type {
  IssueLifecycleResponse,
  LifecycleWorkItem,
  ProductIssue,
  WorkspaceSession,
} from "../../api/types";
import type {
  LifecycleCard as LifecycleCardData,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import { LifecycleCard } from "./LifecycleCard";
import type { DrawerEntity } from "./LifecycleCardDrawer";

type ProviderWorkspaceLaunchTarget = "story" | "design" | "work_item";

const DELETE_EXIT_ANIMATION_MS = 220;

export function IssueCardList({
  cards,
  selectedKey,
  deletingKey,
  onSelect,
  onGenerateStorySpec,
  onDeleteIssue,
}: {
  cards: LifecycleCardData[];
  selectedKey: string | null;
  deletingKey: string | null;
  onSelect: (card: LifecycleCardData) => void;
  onGenerateStorySpec: (card: LifecycleCardData) => void;
  onDeleteIssue: (issueId: string) => void;
}) {
  return (
    <section
      role="region"
      aria-label="Issue 卡片列表"
      className="min-h-0 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
    >
      <div className="mb-3 flex items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
            Issues
          </h2>
          <p className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">
            选择 Issue 后查看它的 Story、Design 和 Work Item。
          </p>
        </div>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {cards.length}
        </span>
      </div>
      {cards.length === 0 ? (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 text-sm text-[var(--aria-ink-muted)]">
          当前 Project 还没有 Issue。
        </div>
      ) : (
        <ul className="space-y-2">
          {cards.map((card) => (
            <li key={`${card.kind}:${card.id}`}>
              <LifecycleCard
                card={card}
                selected={selectedKey === lifecycleCardKey(card)}
                deleting={deletingKey === lifecycleCardKey(card)}
                onSelect={() => onSelect(card)}
                onGenerateStorySpec={() => onGenerateStorySpec(card)}
                onDelete={() => onDeleteIssue(card.id)}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function IssueLifecycleDetail({
  issue,
  storySpecs,
  designSpecs,
  workItems,
  selectedKey,
  deletingKey,
  onSelect,
  onOpenFullIssue,
  onDelete,
}: {
  issue: LifecycleCardData | null;
  storySpecs: LifecycleCardData[];
  designSpecs: LifecycleCardData[];
  workItems: LifecycleCardData[];
  selectedKey: string | null;
  deletingKey: string | null;
  onSelect: (card: LifecycleCardData) => void;
  onOpenFullIssue: (card: LifecycleCardData) => void;
  onDelete: (card: LifecycleCardData) => void;
}) {
  const allWorkItems = workItems.flatMap((card) =>
    card.kind === "work_item" ? [card.raw] : [],
  );
  if (!issue) {
    return (
      <section
        role="region"
        aria-label="Issue 生命周期详情"
        className="flex min-h-96 items-center justify-center rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-6 text-center"
      >
        <div className="max-w-sm">
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
            选择一个 Issue
          </h2>
          <p className="mt-2 text-sm leading-6 text-[var(--aria-ink-muted)]">
            Story Spec、Design Spec 和 Work Item 都会作为该 Issue
            的内容展示在这里。
          </p>
        </div>
      </section>
    );
  }
  const showFullIssueAction = issue.preview
    ? shouldShowFullIssueAction(issue.preview)
    : false;

  return (
    <section
      role="region"
      aria-label="Issue 生命周期详情"
      className="min-h-0 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)]"
    >
      <div className="border-b border-[var(--aria-line)] px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
              Selected Issue
            </p>
            <h2 className="mt-1 truncate text-base font-semibold text-[var(--aria-ink)]">
              {issue.title}
            </h2>
            {issue.preview ? (
              <div className="relative mt-2 max-w-3xl">
                <p
                  data-testid="selected-issue-preview"
                  className={[
                    "whitespace-pre-wrap break-words text-sm leading-6 text-[var(--aria-ink-muted)]",
                    showFullIssueAction ? "line-clamp-6" : "",
                  ].join(" ")}
                >
                  {issue.preview}
                </p>
                {showFullIssueAction ? (
                  <div className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-gradient-to-b from-transparent to-[var(--aria-panel)]" />
                ) : null}
              </div>
            ) : null}
            {showFullIssueAction ? (
              <button
                type="button"
                onClick={() => onOpenFullIssue(issue)}
                className="mt-2 inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2.5 text-xs font-semibold text-[var(--aria-primary)] hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel-muted)]"
              >
                <PanelRightOpen className="h-3.5 w-3.5" />
                查看完整 Issue
              </button>
            ) : null}
          </div>
        </div>
        <div className="mt-3 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
            {issue.id}
          </span>
          <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
            {issue.status}
          </span>
        </div>
      </div>
      <div className="grid gap-3 p-3 xl:grid-cols-3">
        <LifecycleContentSection
          title="Story Spec"
          ariaLabel="Story Spec 内容"
          cards={storySpecs}
          selectedKey={selectedKey}
          deletingKey={deletingKey}
          onSelect={onSelect}
          onDelete={onDelete}
        />
        <LifecycleContentSection
          title="Design Spec"
          ariaLabel="Design Spec 内容"
          cards={designSpecs}
          selectedKey={selectedKey}
          deletingKey={deletingKey}
          onSelect={onSelect}
          onDelete={onDelete}
        />
        <LifecycleContentSection
          title="Work Item"
          ariaLabel="Work Item 内容"
          cards={workItems}
          selectedKey={selectedKey}
          deletingKey={deletingKey}
          onSelect={onSelect}
          onDelete={onDelete}
          allWorkItems={allWorkItems}
        />
      </div>
    </section>
  );
}

function shouldShowFullIssueAction(preview: string) {
  return preview.split(/\r?\n/u).length > 6 || preview.length > 520;
}

function LifecycleContentSection({
  title,
  ariaLabel,
  cards,
  selectedKey,
  deletingKey,
  onSelect,
  onDelete,
  allWorkItems,
}: {
  title: string;
  ariaLabel: string;
  cards: LifecycleCardData[];
  selectedKey: string | null;
  deletingKey: string | null;
  onSelect: (card: LifecycleCardData) => void;
  onDelete: (card: LifecycleCardData) => void;
  allWorkItems?: LifecycleWorkItem[];
}) {
  return (
    <section
      role="region"
      aria-label={ariaLabel}
      className="min-h-72 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
    >
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-[var(--aria-ink)]">
          {title}
        </h3>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {cards.length}
        </span>
      </div>
      {cards.length === 0 ? (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-sm text-[var(--aria-ink-muted)]">
          暂无内容
        </div>
      ) : (
        <ul className="space-y-2">
          {cards.map((card) => (
            <li key={`${card.kind}:${card.id}`}>
              <LifecycleCard
                card={card}
                selected={selectedKey === lifecycleCardKey(card)}
                deleting={deletingKey === lifecycleCardKey(card)}
                onSelect={() => onSelect(card)}
                onDelete={
                  () => onDelete(card)
                }
                allWorkItems={allWorkItems}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function defaultOpenWorkspace(sessionId: string) {
  window.location.assign(
    `/workbench/workspace/${encodeURIComponent(sessionId)}`,
  );
}

export function waitForDeleteExitAnimation() {
  return new Promise<void>((resolve) => {
    window.setTimeout(resolve, DELETE_EXIT_ANIMATION_MS);
  });
}

export function errorMessage(reason: unknown, fallback: string) {
  return reason instanceof Error ? reason.message : fallback;
}

export function defaultOpenCodingWorkspace(attemptId: string) {
  window.location.assign(`/workbench/coding/${encodeURIComponent(attemptId)}`);
}

export function normalizeLifecycleResponse(
  lifecycle: unknown,
  issue: ProductIssue,
): IssueLifecycleResponse {
  if (
    !isRecord(lifecycle) ||
    !isRecord(lifecycle.issue) ||
    lifecycle.issue.issue_id !== issue.issue_id ||
    !Array.isArray(lifecycle.story_specs) ||
    !Array.isArray(lifecycle.design_specs) ||
    !Array.isArray(lifecycle.work_item_plans) ||
    !lifecycle.work_item_plans.every(isIssueWorkItemPlanDetail) ||
    !Array.isArray(lifecycle.work_items) ||
    !Array.isArray(lifecycle.workspace_sessions) ||
    !Array.isArray(lifecycle.coding_attempts)
  ) {
    throw new Error("invalid lifecycle response");
  }

  return lifecycle as IssueLifecycleResponse;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isIssueWorkItemPlanDetail(value: unknown) {
  if (!isRecord(value)) {
    return false;
  }

  return (
    typeof value.id === "string" &&
    typeof value.issue_id === "string" &&
    typeof value.project_id === "string" &&
    typeof value.status === "string" &&
    isStringArray(value.source_story_spec_ids) &&
    isStringArray(value.source_design_spec_ids) &&
    isStringArray(value.work_item_ids) &&
    isStringArray(value.verification_plan_ids) &&
    isDependencyGraph(value.dependency_graph) &&
    (typeof value.repository_profile_ref === "string" ||
      value.repository_profile_ref === null) &&
    isWorkItemSplitOptions(value.options) &&
    isWorkItemSplitFindings(value.validator_findings) &&
    typeof value.created_at === "string" &&
    typeof value.updated_at === "string"
  );
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function isWorkItemSplitOptions(value: unknown) {
  return (
    isRecord(value) &&
    typeof value.include_integration_tests === "boolean" &&
    typeof value.include_e2e_tests === "boolean" &&
    typeof value.force_frontend_backend_split === "boolean" &&
    typeof value.require_execution_plan_confirm === "boolean"
  );
}

function isDependencyGraph(value: unknown) {
  return (
    Array.isArray(value) &&
    value.every(
      (edge) =>
        isRecord(edge) &&
        typeof edge.from_work_item_id === "string" &&
        typeof edge.to_work_item_id === "string",
    )
  );
}

function isWorkItemSplitFindings(value: unknown) {
  return (
    Array.isArray(value) &&
    value.every(
      (finding) =>
        isRecord(finding) &&
        typeof finding.finding_id === "string" &&
        typeof finding.level === "string" &&
        (finding.code === undefined || typeof finding.code === "string") &&
        typeof finding.message === "string" &&
        isStringArray(finding.affected_scopes),
    )
  );
}

export function lifecycleCardKey(card: LifecycleCardData) {
  return `${card.kind}:${card.id}`;
}

export function selectedLifecycleColumns(
  columns: LifecycleColumns,
  focusedIssueId: string | null,
): LifecycleColumns {
  if (!focusedIssueId) {
    return { issue: [], story_spec: [], design_spec: [], work_item: [] };
  }

  return {
    issue: columns.issue.filter((card) => card.issueId === focusedIssueId),
    story_spec: columns.story_spec.filter(
      (card) => card.issueId === focusedIssueId,
    ),
    design_spec: columns.design_spec.filter(
      (card) => card.issueId === focusedIssueId,
    ),
    work_item: columns.work_item.filter(
      (card) => card.issueId === focusedIssueId,
    ),
  };
}

export function findCardInColumns(
  columns: LifecycleColumns,
  entityId: string | null,
): LifecycleCardData | null {
  if (!entityId) {
    return null;
  }

  return (
    [
      ...columns.issue,
      ...columns.story_spec,
      ...columns.design_spec,
      ...columns.work_item,
    ].find((card) => card.id === entityId) ?? null
  );
}

export function toDrawerEntity(
  card: LifecycleCardData,
  allWorkItems?: LifecycleWorkItem[],
): DrawerEntity {
  const base = {
    id: card.id,
    kind: card.kind,
    title: card.title,
    status: card.status,
    version: card.version,
  };

  if (card.kind === "issue") {
    return {
      ...base,
      description: card.raw.description ?? undefined,
      artifacts: card.raw.artifacts,
      phase: card.raw.phase,
      createdAt: card.raw.created_at,
    };
  }

  if (card.kind === "story_spec" || card.kind === "design_spec") {
    return {
      ...base,
      artifactVersions: card.artifactVersions,
    };
  }

  if (card.kind === "work_item_group") {
    const itemsById = new Map(
      (allWorkItems ?? []).map((item) => [item.work_item_id, item]),
    );
    return {
      ...base,
      artifactVersions: card.artifactVersions,
      childWorkItems: card.childWorkItemIds
        .map((id) => itemsById.get(id))
        .filter((item): item is LifecycleWorkItem => item !== undefined),
      workItemPlanSourceStorySpecIds: card.raw.source_story_spec_ids,
      workItemPlanSourceDesignSpecIds: card.raw.source_design_spec_ids,
      workItemPlanValidatorFindings: card.raw.validator_findings,
      workItemPlanDependencyGraph: card.raw.dependency_graph,
    };
  }

  return {
    ...base,
    artifactVersions: card.artifactVersions,
    latestAttempt: card.raw.latest_attempt,
    workItemKind: card.raw.kind,
    dependsOn: card.raw.depends_on,
    exclusiveWriteScopes: card.raw.exclusive_write_scopes,
    forbiddenWriteScopes: card.raw.forbidden_write_scopes,
    contextBudget: card.raw.context_budget,
    requiredHandoffFrom: card.raw.required_handoff_from,
    verificationPlanRef: card.raw.verification_plan_ref,
    requireExecutionPlanConfirm: card.raw.require_execution_plan_confirm,
    executionPlanStatus: card.raw.execution_plan_status,
    handoffSummaryRef: card.raw.handoff_summary_ref,
    completionCommit: card.raw.completion_commit,
    completionDiffSummaryRef: card.raw.completion_diff_summary_ref,
    allWorkItems,
  };
}

export function defaultLaunchTitle(launchTarget: {
  target: ProviderWorkspaceLaunchTarget;
  card: LifecycleCardData;
}) {
  const title = compactLifecycleTitle(launchTarget.card.title);

  if (launchTarget.target === "story") {
    return `${title} Story Spec`;
  }
  if (launchTarget.target === "design") {
    return `${title} Design Spec`;
  }
  return `${title} Work Item`;
}

function compactLifecycleTitle(title: string) {
  const normalizedTitle = title.trim();
  const baseTitle = normalizedTitle
    .replace(/(?:\s+(?:Story Spec|Design Spec|Work Item))+$/u, "")
    .trim();

  return baseTitle || normalizedTitle;
}

export function findWorkspaceSession(
  lifecycles: IssueLifecycleResponse[],
  card: LifecycleCardData,
): WorkspaceSession | null {
  const workspaceType = workspaceTypeForCard(card);
  if (!workspaceType) {
    return null;
  }

  return (
    lifecycles
      .find((lifecycle) => lifecycle.issue.issue_id === card.issueId)
      ?.workspace_sessions.find(
        (session) =>
          session.entity_id === card.id &&
          session.workspace_type === workspaceType,
      ) ?? null
  );
}

function workspaceTypeForCard(
  card: LifecycleCardData,
): WorkspaceSession["workspace_type"] | null {
  if (card.kind === "story_spec") {
    return "story";
  }
  if (card.kind === "design_spec") {
    return "design";
  }
  if (card.kind === "work_item") {
    return "work_item";
  }
  if (card.kind === "work_item_group") {
    return "work_item_plan";
  }
  return null;
}
