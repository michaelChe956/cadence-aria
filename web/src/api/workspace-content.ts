import type {
  WorkspaceArtifactVersionResponse,
  WorkspaceEventOutputResponse,
  WorkspaceNodeDetailResponse,
  WorkspacePromptResponse,
} from "./types";

export async function fetchWorkspaceNodeDetail(
  sessionId: string,
  nodeId: string,
): Promise<WorkspaceNodeDetailResponse> {
  const response = await fetch(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/timeline-node-details/${encodeURIComponent(nodeId)}`,
  );
  if (!response.ok) {
    throw new Error(`加载节点详情失败：${response.status}`);
  }
  return response.json() as Promise<WorkspaceNodeDetailResponse>;
}

export async function fetchWorkspacePrompt(
  sessionId: string,
  nodeId: string,
): Promise<WorkspacePromptResponse> {
  const response = await fetch(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/timeline-node-details/${encodeURIComponent(nodeId)}/prompt`,
  );
  if (!response.ok) {
    throw new Error(`加载 Prompt 失败：${response.status}`);
  }
  return response.json() as Promise<WorkspacePromptResponse>;
}

export async function fetchWorkspaceEventOutput(
  sessionId: string,
  nodeId: string,
  eventId: string,
): Promise<WorkspaceEventOutputResponse> {
  const response = await fetch(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/timeline-node-details/${encodeURIComponent(nodeId)}/events/${encodeURIComponent(eventId)}/output`,
  );
  if (!response.ok) {
    throw new Error(`加载输出失败：${response.status}`);
  }
  return response.json() as Promise<WorkspaceEventOutputResponse>;
}

export async function fetchWorkspaceArtifactVersion(
  sessionId: string,
  version: number,
): Promise<WorkspaceArtifactVersionResponse> {
  const response = await fetch(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/artifact-versions/${String(version)}`,
  );
  if (!response.ok) {
    throw new Error(`加载 Artifact 失败：${response.status}`);
  }
  return response.json() as Promise<WorkspaceArtifactVersionResponse>;
}
