import { useEffect, useMemo, useState } from "react";
import { listIssues, listProjects } from "../../api/client";
import type { Issue, ProductWebEvent, Project } from "../../api/types";
import { createProjectWorkbenchStore } from "../../state/project-workbench-store";
import type { ExecutionContext } from "../task/TaskManagementWorkbench";
import { GateActionBar } from "./GateActionBar";
import { IssueDetailPane } from "./IssueDetailPane";
import { IssueListPane } from "./IssueListPane";
import { ProjectTopBar } from "./ProjectTopBar";
import { ProviderExecutionPanel } from "./ProviderExecutionPanel";
import { RepositoryManager } from "./RepositoryManager";

export function ProjectManagementWorkbench({
  onOpenExecution,
}: {
  onOpenExecution: (context: ExecutionContext) => void;
}) {
  const [store] = useState(() => createProjectWorkbenchStore());
  const [legacyIssues, setLegacyIssues] = useState<Issue[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [, setVersion] = useState(0);

  const refresh = () => {
    setBusy(true);
    setError(null);
    Promise.all([listProjects(), listIssues()])
      .then(([projectResponse, issueResponse]) => {
        store.setProjects(projectResponse.projects);
        store.setIssues(
          issueResponse.issues.map((issue) => ({
            issue_id: issue.issue_id,
            project_id: projectResponse.projects[0]?.project_id ?? "legacy_project",
            repo_id: "legacy_repo",
            title: issue.title,
            description: issue.description,
            change_id: issue.change_id,
            phase: phaseForIssue(issue),
            status: productStatusForIssue(issue.status),
            active_binding_id: issue.task_id,
            created_at: issue.created_at,
            updated_at: issue.updated_at,
          })),
        );
        setLegacyIssues(issueResponse.issues);
        if (!store.snapshot.selectedProjectId && projectResponse.projects[0]) {
          store.selectProject(projectResponse.projects[0].project_id);
        }
        if (!store.snapshot.selectedIssueId && issueResponse.issues[0]) {
          store.selectIssue(issueResponse.issues[0].issue_id);
        }
        seedProviderEvents(store.snapshot.events, (event) => store.pushEvent(event));
        setVersion((version) => version + 1);
      })
      .catch((reason) => {
        setError(reason instanceof Error ? reason.message : "load project workbench failed");
      })
      .finally(() => setBusy(false));
  };

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const selectedProject = useMemo(
    () =>
      store.snapshot.projects.find(
        (project) => project.project_id === store.snapshot.selectedProjectId,
      ) ?? store.snapshot.projects[0] ?? null,
    [store.snapshot.projects, store.snapshot.selectedProjectId],
  );
  const selectedIssue =
    legacyIssues.find((issue) => issue.issue_id === store.snapshot.selectedIssueId) ??
    legacyIssues[0] ??
    null;

  function handleSelectProject(projectId: string) {
    store.selectProject(projectId);
    setVersion((version) => version + 1);
  }

  function handleSelectIssue(issueId: string) {
    store.selectIssue(issueId);
    setVersion((version) => version + 1);
  }

  return (
    <div className="min-h-screen bg-[#F8FAFC] text-[#241B2F]">
      <ProjectTopBar
        projects={store.snapshot.projects}
        selectedProjectId={selectedProject?.project_id ?? null}
        issueCount={legacyIssues.length}
        busy={busy}
        onSelectProject={handleSelectProject}
        onRefresh={refresh}
      />

      {error ? (
        <div
          role="alert"
          className="border-b-2 border-rose-200 bg-rose-100 px-4 py-2 text-sm font-semibold text-rose-800 md:px-6 lg:px-8"
        >
          {error}
        </div>
      ) : null}

      <main
        aria-label="项目管理工作台"
        className="grid min-h-[calc(100vh-4rem)] grid-cols-1 gap-5 px-4 py-5 md:px-6 lg:grid-cols-[19rem_minmax(0,1fr)_22rem] lg:px-8"
      >
        <aside className="space-y-5">
          <IssueListPane
            issues={legacyIssues}
            selectedIssueId={selectedIssue?.issue_id ?? null}
            busy={busy}
            onSelectIssue={handleSelectIssue}
          />
        </aside>

        <section className="min-w-0 space-y-5" aria-label="Issue 工作区">
          <IssueDetailPane issue={selectedIssue} onOpenExecution={onOpenExecution} />
          {selectedIssue?.status === "blocked" ? (
            <GateActionBar
              gate={{ gate_id: "gate_preview", node_id: "N09", status: selectedIssue.status }}
              onConfirm={() => undefined}
              onRequestChange={() => undefined}
              onTerminate={() => undefined}
            />
          ) : null}
        </section>

        <aside className="space-y-5">
          <RepositoryManager project={selectedProject} issueCount={legacyIssues.length} />
          <ProviderExecutionPanel events={store.snapshot.events} />
        </aside>
      </main>
    </div>
  );
}

function seedProviderEvents(
  events: ProductWebEvent[],
  pushEvent: (event: ProductWebEvent) => void,
) {
  if (events.length > 0) {
    return;
  }
  pushEvent({
    cursor: 1,
    event_type: "provider.input_prepared",
    task_id: null,
    project_id: null,
    issue_id: null,
    binding_id: null,
    payload: {
      node_id: "N16",
      input_ref: "run_n16_0001",
      input_summary: { kind: "workbench preview" },
    },
  });
}

function phaseForIssue(issue: Issue): "clarification" | "development" | "acceptance" {
  if (issue.status === "completed") {
    return "acceptance";
  }
  if (issue.status === "draft") {
    return "clarification";
  }
  return "development";
}

function productStatusForIssue(status: string): "draft" | "in_progress" | "completed" | "blocked" {
  if (status === "completed" || status === "blocked" || status === "draft") {
    return status;
  }
  return "in_progress";
}
