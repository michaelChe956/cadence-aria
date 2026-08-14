import { ApiRequestError, normalizeApiError } from "./client";
import type {
  PointerPublicationBatchKind,
  PointerPublicationDto,
} from "./types";

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
  const text = await response.text();
  if (!text.trim()) {
    return undefined as T;
  }
  return JSON.parse(text) as T;
}

function publicationsPath(projectId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}/logical-codebase/pointer-publications`;
}

export function listPointerPublications(
  projectId: string,
): Promise<PointerPublicationDto[]> {
  return requestJson<PointerPublicationDto[]>(publicationsPath(projectId));
}

export function getPointerPublication(
  projectId: string,
  publicationId: string,
): Promise<PointerPublicationDto> {
  return requestJson<PointerPublicationDto>(
    `${publicationsPath(projectId)}/${encodeURIComponent(publicationId)}`,
  );
}

export function createPointerPublication(
  projectId: string,
  batchKind: PointerPublicationBatchKind,
): Promise<PointerPublicationDto> {
  return requestJson<PointerPublicationDto>(publicationsPath(projectId), {
    method: "POST",
    body: JSON.stringify({ batch_kind: batchKind }),
  });
}

export function retryPointerPublicationRepo(
  projectId: string,
  publicationId: string,
  memberRepoId: string,
): Promise<PointerPublicationDto> {
  return requestJson<PointerPublicationDto>(
    `${publicationsPath(projectId)}/${encodeURIComponent(publicationId)}/retry-repo`,
    {
      method: "POST",
      body: JSON.stringify({ member_repo_id: memberRepoId }),
    },
  );
}

export function revokePointerPublication(
  projectId: string,
  publicationId: string,
): Promise<PointerPublicationDto> {
  return requestJson<PointerPublicationDto>(
    `${publicationsPath(projectId)}/${encodeURIComponent(publicationId)}/revoke`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}
