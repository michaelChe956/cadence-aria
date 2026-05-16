import { Plus, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  createProductIssue,
  getIssueLifecycle,
  listProductIssues,
  listProjects,
  listRepositories,
} from "../../api/client";
import type {
  IssueLifecycleResponse,
  ProductIssue,
  Project,
  Repository,
} from "../../api/types";
import {
  groupLifecycleCards,
  visibleLifecycle,
  type LifecycleCard as LifecycleCardData,
} from "../../state/lifecycle-workbench-store";
import { WorkbenchSurface } from "../shell/WorkbenchSurface";
import {
  CreateLifecycleIssueDialog,
  type CreateLifecycleIssuePayload,
} from "./CreateLifecycleIssueDialog";
import { LifecycleColumn } from "./LifecycleColumn";

export function IssueLifecycleWorkbench() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [lifecycles, setLifecycles] = useState<IssueLifecycleResponse[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [focusedIssueId, setFocusedIssueId] = useState<string | null>(null);
  const [selectedCardKey, setSelectedCardKey] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshRequestId = useRef(0);

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refresh() {
    const requestId = refreshRequestId.current + 1;
    refreshRequestId.current = requestId;

    setBusy(true);
    setError(null);
    try {
      const projectResponse = await listProjects();
      if (!isLatestRefresh(requestId)) {
        return;
      }

      const projectId = selectedProjectId ?? projectResponse.projects[0]?.project_id ?? null;
      setProjects(projectResponse.projects);
      setSelectedProjectId(projectId);

      if (!projectId) {
        setRepositories([]);
        setLifecycles([]);
        return;
      }

      const [repositoryResponse, issueResponse] = await Promise.all([
        listRepositories(projectId),
        listProductIssues(projectId),
      ]);
      if (!isLatestRefresh(requestId)) {
        return;
      }

      const lifecycleResponses = await Promise.all(
        (issueResponse.issues ?? []).map(async (issue) =>
          normalizeLifecycleResponse(await getIssueLifecycle(issue.issue_id, projectId), issue),
        ),
      );
      if (!isLatestRefresh(requestId)) {
        return;
      }

      setRepositories(repositoryResponse.repositories ?? []);
      setLifecycles(lifecycleResponses);
    } catch (reason) {
      if (isLatestRefresh(requestId)) {
        setError(reason instanceof Error ? reason.message : "load lifecycle workbench failed");
      }
    } finally {
      if (isLatestRefresh(requestId)) {
        setBusy(false);
      }
    }
  }

  function isLatestRefresh(requestId: number) {
    return requestId === refreshRequestId.current;
  }

  const columns = useMemo(
    () => visibleLifecycle(groupLifecycleCards(lifecycles), focusedIssueId),
    [lifecycles, focusedIssueId],
  );
  const selectedProject = projects.find((project) => project.project_id === selectedProjectId);

  function handleSelectCard(card: LifecycleCardData) {
    setSelectedCardKey(lifecycleCardKey(card));
    if (card.kind === "issue") {
      setFocusedIssueId(card.issueId);
    }
  }

  async function handleCreateIssue(payload: CreateLifecycleIssuePayload) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    await createProductIssue(selectedProjectId, {
      title: payload.title,
      description: payload.description,
      change_id: null,
      repository_id: payload.repository_id,
    });
    setDialogOpen(false);
    await refresh();
  }

  return (
    <>
      <WorkbenchSurface
        mainLabel="Issue 生命周期工作台"
        statusBar={
          busy ? (
            <span className="text-xs font-semibold text-[var(--aria-ink-muted)]">加载中</span>
          ) : null
        }
        alert={error}
        header={
          <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <h1 className="truncate text-base font-semibold text-[var(--aria-ink)]">
                Issue 生命周期工作台
              </h1>
              <p className="truncate text-xs text-[var(--aria-ink-muted)]">
                {selectedProject?.name ?? "未选择 Project"}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              {focusedIssueId ? (
                <button
                  type="button"
                  onClick={() => setFocusedIssueId(null)}
                  className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
                >
                  显示全部
                </button>
              ) : null}
              <button
                type="button"
                aria-label="刷新"
                onClick={() => void refresh()}
                className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)]"
              >
                <RefreshCw className="h-4 w-4" />
              </button>
              <button
                type="button"
                onClick={() => setDialogOpen(true)}
                className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white"
              >
                <Plus className="mr-1 h-4 w-4" />
                新建 Issue
              </button>
            </div>
          </div>
        }
        main={
          <div className="grid min-h-[calc(100vh-6rem)] gap-3 overflow-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 xl:grid-cols-4">
            <LifecycleColumn
              title="Issue"
              ariaLabel="Issue 列"
              cards={columns.issue}
              selectedKey={selectedCardKey}
              onSelect={handleSelectCard}
            />
            <LifecycleColumn
              title="Story Spec"
              ariaLabel="Story Spec 列"
              cards={columns.story_spec}
              selectedKey={selectedCardKey}
              onSelect={handleSelectCard}
            />
            <LifecycleColumn
              title="Design Spec"
              ariaLabel="Design Spec 列"
              cards={columns.design_spec}
              selectedKey={selectedCardKey}
              onSelect={handleSelectCard}
            />
            <LifecycleColumn
              title="Work Item"
              ariaLabel="Work Item 列"
              cards={columns.work_item}
              selectedKey={selectedCardKey}
              onSelect={handleSelectCard}
            />
          </div>
        }
      />
      {dialogOpen ? (
        <CreateLifecycleIssueDialog
          repositories={repositories}
          onCreate={handleCreateIssue}
          onClose={() => setDialogOpen(false)}
        />
      ) : null}
    </>
  );
}

function normalizeLifecycleResponse(
  lifecycle: unknown,
  issue: ProductIssue,
): IssueLifecycleResponse {
  if (
    !isRecord(lifecycle) ||
    !isRecord(lifecycle.issue) ||
    lifecycle.issue.issue_id !== issue.issue_id ||
    !Array.isArray(lifecycle.story_specs) ||
    !Array.isArray(lifecycle.design_specs) ||
    !Array.isArray(lifecycle.work_items)
  ) {
    throw new Error("invalid lifecycle response");
  }

  return lifecycle as IssueLifecycleResponse;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function lifecycleCardKey(card: LifecycleCardData) {
  return `${card.kind}:${card.id}`;
}
