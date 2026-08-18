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

function registrationsPath(projectId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}/logical-codebase/registrations`;
}

export function preflightLogicalCodebaseRegistration(
  projectId: string,
  request: RegistrationPreflightRequest,
): Promise<RegistrationPreflightResponse> {
  return requestJson<RegistrationPreflightResponse>(
    `${registrationsPath(projectId)}/preflight`,
    { method: "POST", body: JSON.stringify(request) },
  );
}

export function submitLogicalCodebaseRegistration(
  projectId: string,
  request: RegistrationSubmitRequest,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(registrationsPath(projectId), {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export function getLogicalCodebaseRegistration(
  projectId: string,
  batchId: string,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(
    `${registrationsPath(projectId)}/${encodeURIComponent(batchId)}`,
  );
}

export function resumeLogicalCodebaseRegistration(
  projectId: string,
  batchId: string,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(
    `${registrationsPath(projectId)}/${encodeURIComponent(batchId)}/resume`,
    { method: "POST", body: JSON.stringify({}) },
  );
}

export function cancelLogicalCodebaseRegistration(
  projectId: string,
  batchId: string,
): Promise<RegistrationBatchDto> {
  return requestJson<RegistrationBatchDto>(
    `${registrationsPath(projectId)}/${encodeURIComponent(batchId)}/cancel`,
    { method: "POST", body: JSON.stringify({}) },
  );
}
