import type { ProductIssue, ProductWebEvent, Project } from "../api/types";

export type ProjectWorkbenchSnapshot = {
  projects: Project[];
  issues: ProductIssue[];
  selectedProjectId: string | null;
  selectedIssueId: string | null;
  events: ProductWebEvent[];
};

export function createProjectWorkbenchStore() {
  const snapshot: ProjectWorkbenchSnapshot = {
    projects: [],
    issues: [],
    selectedProjectId: null,
    selectedIssueId: null,
    events: [],
  };

  return {
    snapshot,
    setProjects(projects: Project[]) {
      snapshot.projects = projects;
    },
    selectProject(projectId: string) {
      snapshot.selectedProjectId = projectId;
    },
    setIssues(issues: ProductIssue[]) {
      snapshot.issues = issues;
    },
    selectIssue(issueId: string) {
      snapshot.selectedIssueId = issueId;
    },
    pushEvent(event: ProductWebEvent) {
      snapshot.events = [...snapshot.events.slice(-199), event];
    },
  };
}
