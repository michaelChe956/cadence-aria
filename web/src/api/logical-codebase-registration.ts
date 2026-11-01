import { ApiRequestError, normalizeApiError } from "./client";
import type {
  RegistrationBatchDto,
  RegistrationPreflightRequest,
  RegistrationPreflightResponse,
  RegistrationSubmitRequest,
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

function registrationsPath(projectId: string, logicalCodebaseId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}/logical-codebases/${encodeURIComponent(logicalCodebaseId)}/registrations`;
}

export function preflightLogicalCodebaseRegistration(
  projectId: string,
  logicalCodebaseId: string,
  request: RegistrationPreflightRequest,
): Promise<RegistrationPreflightResponse> {
  return requestJson<RegistrationPreflightResponse>(
    `${registrationsPath(projectId, logicalCodebaseId)}/preflight`,
    { method: "POST", body: JSON.stringify(request) },
  );
}

export function submitLogicalCodebaseRegistration(
  projectId: string,
  logicalCodebaseId: string,
  request: RegistrationSubmitRequest,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(registrationsPath(projectId, logicalCodebaseId), {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export function getLogicalCodebaseRegistration(
  projectId: string,
  logicalCodebaseId: string,
  batchId: string,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(
    `${registrationsPath(projectId, logicalCodebaseId)}/${encodeURIComponent(batchId)}`,
  );
}

export function resumeLogicalCodebaseRegistration(
  projectId: string,
  logicalCodebaseId: string,
  batchId: string,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(
    `${registrationsPath(projectId, logicalCodebaseId)}/${encodeURIComponent(batchId)}/resume`,
    { method: "POST", body: JSON.stringify({}) },
  );
}

export function cancelLogicalCodebaseRegistration(
  projectId: string,
  logicalCodebaseId: string,
  batchId: string,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(
    `${registrationsPath(projectId, logicalCodebaseId)}/${encodeURIComponent(batchId)}/cancel`,
    { method: "POST", body: JSON.stringify({}) },
  );
}
