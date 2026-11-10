// LC 知识库 API 客户端（openspec add-logical-codebase-knowledge-base 计划 Task 8）。
// HTTP 面前缀 /api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base；
// 失败统一抛 ApiRequestError（client.ts 的 normalizeApiError 语义）。
import { ApiRequestError, normalizeApiError } from "./client";
import type {
  KnowledgeBaseChangePackageDto,
  KnowledgeBaseChangePackageLintDto,
  KnowledgeBaseFileContentDto,
  KnowledgeBaseFileNodeDto,
  KnowledgeBaseOperationSnapshotDto,
  KnowledgeBaseStatusDto,
  KnowledgeBaseUserInputConfirmationDto,
  KnowledgeBaseUserInputDigestItem,
  KnowledgeBaseUserInputSummaryDto,
} from "./types/knowledge-base";

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...init?.headers,
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

function knowledgeBasePath(projectId: string, lcId: string): string {
  return `/api/projects/${encodeURIComponent(
    projectId,
  )}/logical-codebases/${encodeURIComponent(lcId)}/knowledge-base`;
}

export function getKnowledgeBaseStatus(
  projectId: string,
  lcId: string,
): Promise<KnowledgeBaseStatusDto> {
  return requestJson<KnowledgeBaseStatusDto>(
    `${knowledgeBasePath(projectId, lcId)}/status`,
  );
}

export async function listKnowledgeBaseFiles(
  projectId: string,
  lcId: string,
): Promise<KnowledgeBaseFileNodeDto[]> {
  const response = await requestJson<{ nodes: KnowledgeBaseFileNodeDto[] }>(
    `${knowledgeBasePath(projectId, lcId)}/files`,
  );
  return response.nodes;
}

export function readKnowledgeBaseFile(
  projectId: string,
  lcId: string,
  path: string,
): Promise<KnowledgeBaseFileContentDto> {
  return requestJson<KnowledgeBaseFileContentDto>(
    `${knowledgeBasePath(projectId, lcId)}/file?path=${encodeURIComponent(path)}`,
  );
}

export function writeKnowledgeBaseFile(
  projectId: string,
  lcId: string,
  path: string,
  content: string,
  baseDigest?: string,
): Promise<{ path: string; digest: string }> {
  return requestJson<{ path: string; digest: string }>(
    `${knowledgeBasePath(projectId, lcId)}/file`,
    {
      method: "PUT",
      body: JSON.stringify(
        baseDigest === undefined
          ? { path, content }
          : { path, content, base_digest: baseDigest },
      ),
    },
  );
}

export async function listKnowledgeBaseChangePackages(
  projectId: string,
  lcId: string,
): Promise<KnowledgeBaseChangePackageDto[]> {
  const response = await requestJson<{ packages: KnowledgeBaseChangePackageDto[] }>(
    `${knowledgeBasePath(projectId, lcId)}/change-packages`,
  );
  return response.packages;
}

export function lintKnowledgeBaseChangePackage(
  projectId: string,
  lcId: string,
  path: string,
): Promise<KnowledgeBaseChangePackageLintDto> {
  return requestJson<KnowledgeBaseChangePackageLintDto>(
    `${knowledgeBasePath(projectId, lcId)}/change-packages/lint?path=${encodeURIComponent(
      path,
    )}`,
  );
}

export function createKnowledgeBaseChangePackage(
  projectId: string,
  lcId: string,
  identifier: string,
): Promise<{ id: string; path: string }> {
  return requestJson<{ id: string; path: string }>(
    `${knowledgeBasePath(projectId, lcId)}/change-packages`,
    {
      method: "POST",
      body: JSON.stringify({ identifier }),
    },
  );
}

export function createKnowledgeBaseBootstrapInputs(
  projectId: string,
  lcId: string,
): Promise<{ created: string[] }> {
  return requestJson<{ created: string[] }>(
    `${knowledgeBasePath(projectId, lcId)}/bootstrap-inputs`,
    {
      method: "POST",
    },
  );
}

export function startKnowledgeBaseInitialization(
  projectId: string,
  lcId: string,
  kind: "bootstrap" | "update",
  idempotencyKey: string,
  packagePath?: string,
): Promise<KnowledgeBaseOperationSnapshotDto> {
  return requestJson<KnowledgeBaseOperationSnapshotDto>(
    `${knowledgeBasePath(projectId, lcId)}/initializations`,
    {
      method: "POST",
      body: JSON.stringify(
        packagePath === undefined
          ? { kind, idempotency_key: idempotencyKey }
          : { kind, package_path: packagePath, idempotency_key: idempotencyKey },
      ),
    },
  );
}

/**
 * LC 当前知识库 operation 投影（任意状态）。LC 尚无知识库 operation
 * （404 + `knowledge_base_operation_not_found`）是正常态，解析为 null；
 * 其它错误照抛 ApiRequestError。
 */
export async function getActiveKnowledgeBaseOperation(
  projectId: string,
  lcId: string,
): Promise<KnowledgeBaseOperationSnapshotDto | null> {
  try {
    return await requestJson<KnowledgeBaseOperationSnapshotDto>(
      `${knowledgeBasePath(projectId, lcId)}/initializations/active`,
    );
  } catch (reason) {
    if (
      reason instanceof ApiRequestError &&
      reason.code === "knowledge_base_operation_not_found"
    ) {
      return null;
    }
    throw reason;
  }
}

export function getKnowledgeBaseOperation(
  projectId: string,
  lcId: string,
  operationId: string,
): Promise<KnowledgeBaseOperationSnapshotDto> {
  return requestJson<KnowledgeBaseOperationSnapshotDto>(
    `${knowledgeBasePath(projectId, lcId)}/initializations/${encodeURIComponent(
      operationId,
    )}`,
  );
}

export function cancelKnowledgeBaseOperation(
  projectId: string,
  lcId: string,
  operationId: string,
): Promise<KnowledgeBaseOperationSnapshotDto> {
  return requestJson<KnowledgeBaseOperationSnapshotDto>(
    `${knowledgeBasePath(
      projectId,
      lcId,
    )}/initializations/${encodeURIComponent(operationId)}/cancel`,
    {
      method: "POST",
    },
  );
}

/** GET /user-input：输入文件的状态投影（filled/confirmed/pending）与汇总。 */
export function getKnowledgeBaseUserInput(
  projectId: string,
  lcId: string,
): Promise<KnowledgeBaseUserInputSummaryDto> {
  return requestJson<KnowledgeBaseUserInputSummaryDto>(
    `${knowledgeBasePath(projectId, lcId)}/user-input`,
  );
}

/**
 * 逐份确认（POST /user-input/confirm，单份形态 `{ path, digest }`）。
 * digest 必须是客户端当前读到的服务端摘要；不符 ⇒ 409
 * `knowledge_base_user_input_confirmation_stale`。
 */
export function confirmKnowledgeBaseUserInputFile(
  projectId: string,
  lcId: string,
  path: string,
  digest: string,
): Promise<KnowledgeBaseUserInputConfirmationDto> {
  return requestJson<KnowledgeBaseUserInputConfirmationDto>(
    `${knowledgeBasePath(projectId, lcId)}/user-input/confirm`,
    {
      method: "POST",
      body: JSON.stringify({ path, digest }),
    },
  );
}

/** 批量确认（POST /user-input/confirm，批量形态 `{ all: true, items }`）。 */
export function confirmAllKnowledgeBaseUserInput(
  projectId: string,
  lcId: string,
  items: KnowledgeBaseUserInputDigestItem[],
): Promise<KnowledgeBaseUserInputConfirmationDto> {
  return requestJson<KnowledgeBaseUserInputConfirmationDto>(
    `${knowledgeBasePath(projectId, lcId)}/user-input/confirm`,
    {
      method: "POST",
      body: JSON.stringify({ all: true, items }),
    },
  );
}
