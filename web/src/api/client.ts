import type {
  ApiError,
  ArtifactContentResponse,
  CodingAttempt,
  CodingAttemptSnapshotResponse,
  CreateProductIssueRequest,
  CreateRepositoryRequest,
  GenerateDesignSpecsRequest,
  GenerateDesignSpecsResponse,
  GenerateStorySpecsRequest,
  GenerateStorySpecsResponse,
  GenerateWorkItemsRequest,
  GenerateWorkItemsResponse,
  IssueLifecycleResponse,
  ProductIssue,
  ProductIssueListResponse,
  Project,
  Repository,
  RepositoryListResponse,
} from "./types";

export class ApiRequestError extends Error implements ApiError {
  code: string;
  details: Record<string, unknown>;

  constructor(error: ApiError) {
    super(error.message);
    this.name = "ApiRequestError";
    this.code = error.code;
    this.details = error.details;
  }
}

export async function normalizeApiError(response: Response): Promise<ApiError> {
  const body = await response.json().catch(() => ({}));
  return {
    code: typeof body.code === "string" ? body.code : "web_client_error",
    message:
      typeof body.message === "string" ? body.message : response.statusText,
    details:
      typeof body.details === "object" && body.details !== null
        ? body.details
        : {},
  };
}

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    throw new ApiRequestError(await normalizeApiError(response));
  }
  return response.json() as Promise<T>;
}

export function listProjects(): Promise<{ projects: Project[] }> {
  return requestJson<{ projects: Project[] }>("/api/projects");
}

export function createProject(payload: {
  name: string;
  description?: string | null;
}): Promise<Project> {
  return requestJson<Project>("/api/projects", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function deleteProject(projectId: string): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}`,
    {
      method: "DELETE",
    },
  );
}

export function listRepositories(
  projectId: string,
): Promise<RepositoryListResponse> {
  return requestJson<RepositoryListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories`,
  );
}

export function createRepository(
  projectId: string,
  payload: CreateRepositoryRequest,
): Promise<Repository> {
  return requestJson<Repository>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function deleteRepository(
  projectId: string,
  repositoryId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories/${encodeURIComponent(repositoryId)}`,
    {
      method: "DELETE",
    },
  );
}

export function listProductIssues(
  projectId: string,
): Promise<ProductIssueListResponse> {
  return requestJson<ProductIssueListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues`,
  );
}

export function createProductIssue(
  projectId: string,
  payload: CreateProductIssueRequest,
): Promise<ProductIssue> {
  return requestJson<ProductIssue>(
    `/api/projects/${encodeURIComponent(projectId)}/issues`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function deleteProductIssue(
  projectId: string,
  issueId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}`,
    {
      method: "DELETE",
    },
  );
}

export function deleteStorySpec(
  projectId: string,
  issueId: string,
  storySpecId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/story-specs/${encodeURIComponent(storySpecId)}`,
    {
      method: "DELETE",
    },
  );
}

export function deleteDesignSpec(
  projectId: string,
  issueId: string,
  designSpecId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/design-specs/${encodeURIComponent(designSpecId)}`,
    {
      method: "DELETE",
    },
  );
}

export function deleteWorkItem(
  projectId: string,
  issueId: string,
  workItemId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-items/${encodeURIComponent(workItemId)}`,
    {
      method: "DELETE",
    },
  );
}

export function getIssueLifecycle(
  issueId: string,
  projectId: string,
): Promise<IssueLifecycleResponse> {
  return requestJson<IssueLifecycleResponse>(
    `/api/issues/${encodeURIComponent(issueId)}/lifecycle?project_id=${encodeURIComponent(projectId)}`,
  );
}

export function generateStorySpecs(
  projectId: string,
  issueId: string,
  payload: GenerateStorySpecsRequest,
): Promise<GenerateStorySpecsResponse> {
  return requestJson<GenerateStorySpecsResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/story-specs:generate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function generateDesignSpecs(
  projectId: string,
  issueId: string,
  payload: GenerateDesignSpecsRequest,
): Promise<GenerateDesignSpecsResponse> {
  return requestJson<GenerateDesignSpecsResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/design-specs:generate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function generateWorkItems(
  projectId: string,
  issueId: string,
  payload: GenerateWorkItemsRequest,
): Promise<GenerateWorkItemsResponse> {
  return requestJson<GenerateWorkItemsResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-items:generate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function createCodingAttempt(
  projectId: string,
  issueId: string,
  workItemId: string,
): Promise<CodingAttempt> {
  return requestJson<CodingAttempt>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-items/${encodeURIComponent(workItemId)}/coding-attempts`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}

export function getCodingAttemptSnapshot(
  attemptId: string,
): Promise<CodingAttemptSnapshotResponse> {
  return requestJson<CodingAttemptSnapshotResponse>(
    `/api/coding-attempts/${encodeURIComponent(attemptId)}`,
  );
}

export function deleteCodingAttempt(
  attemptId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/coding-attempts/${encodeURIComponent(attemptId)}`,
    {
      method: "DELETE",
    },
  );
}

export function abortCodingAttempt(attemptId: string): Promise<CodingAttempt> {
  return requestJson<CodingAttempt>(
    `/api/coding-attempts/${encodeURIComponent(attemptId)}/abort`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}

export function getCodingAttemptArtifact(
  attemptId: string,
  artifactId: string,
): Promise<ArtifactContentResponse> {
  return requestJson<ArtifactContentResponse>(
    `/api/coding-attempts/${encodeURIComponent(attemptId)}/artifacts/${encodeURIComponent(artifactId)}`,
  );
}
