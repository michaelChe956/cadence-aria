import { readFile, readdir } from "node:fs/promises";
import { mkdirSync, type Dirent } from "node:fs";
import path from "node:path";
import { apiGetSafe } from "./api-reader.ts";
import { readManifest, atomicWriteJson, type RunManifest } from "./run-contract.ts";

/// 断言双面之「backend 面」:每阶段在稳定点做一次最小只读 GET dump
/// (v2.0 §Q6:不重复 EvidenceCell/provider 矩阵,不扩大请求负担)。
/// 后端读取不替代页面断言;页面颜色也不替代 durable 事实。

export type StageDump = {
  runId: string;
  stage: string;
  at: string;
  baseURL: string;
  endpoints: Record<string, { status: number; body: unknown; error?: string }>;
};

/** 产品既有 GET 投影的最小集合(按已观测身份拼接;缺失记 null 不猜)。 */
export async function collectStageDump(baseURL: string, runId: string, stage: string): Promise<StageDump> {
  const manifest = readManifest(runId);
  const observed = manifest.observed;
  const endpoints: StageDump["endpoints"] = {};

  endpoints["/api/health"] = await apiGetSafe(baseURL, "/api/health");
  endpoints["/api/runtime-info"] = await apiGetSafe(baseURL, "/api/runtime-info");
  endpoints["/api/projects"] = await apiGetSafe(baseURL, "/api/projects");

  if (observed.projectId) {
    endpoints[`/api/projects/${observed.projectId}/codebases`] = await apiGetSafe(
      baseURL,
      `/api/projects/${observed.projectId}/codebases`,
    );
    endpoints[`/api/projects/${observed.projectId}/repositories`] = await apiGetSafe(
      baseURL,
      `/api/projects/${observed.projectId}/repositories`,
    );
    endpoints[`/api/projects/${observed.projectId}/issues`] = await apiGetSafe(
      baseURL,
      `/api/projects/${observed.projectId}/issues`,
    );
  }
  if (observed.projectId && observed.logicalCodebaseId) {
    endpoints[`/api/projects/${observed.projectId}/logical-codebases/${observed.logicalCodebaseId}/members`] =
      await apiGetSafe(
        baseURL,
        `/api/projects/${observed.projectId}/logical-codebases/${observed.logicalCodebaseId}/members`,
      );
    endpoints[`/api/projects/${observed.projectId}/logical-codebases/${observed.logicalCodebaseId}/bootstrap`] =
      await apiGetSafe(
        baseURL,
        `/api/projects/${observed.projectId}/logical-codebases/${observed.logicalCodebaseId}/bootstrap`,
      );
    if (observed.initOperationId) {
      endpoints[
        `/api/projects/${observed.projectId}/logical-codebases/${observed.logicalCodebaseId}/initializations/${observed.initOperationId}`
      ] = await apiGetSafe(
        baseURL,
        `/api/projects/${observed.projectId}/logical-codebases/${observed.logicalCodebaseId}/initializations/${observed.initOperationId}`,
      );
    }
  }
  if (observed.issueId && observed.projectId) {
    endpoints[`/api/issues/${observed.issueId}/lifecycle`] = await apiGetSafe(
      baseURL,
      `/api/issues/${observed.issueId}/lifecycle?project_id=${observed.projectId}`,
    );
  }

  const dump: StageDump = { runId, stage, at: new Date().toISOString(), baseURL, endpoints };
  const dir = path.join(manifest.evidenceRoot, "backend-dumps");
  mkdirSync(dir, { recursive: true });
  atomicWriteJson(path.join(dir, `${stage}.json`), dump);
  return dump;
}

/**
 * 从 workspace 的 .aria durable 面只读发现初始化 operation id 与状态
 * (v2.0 允许 supervisor 对无 GET 投影的诊断材料做只读归档)。
 * 返回 null 表示尚未落盘任何 operation 事实。
 */
export type DiscoveredOperation = { operationId: string; status: string; raw: string };

export async function findInitializationOperation(
  manifest: RunManifest,
): Promise<DiscoveredOperation | null> {
  const ariaRoot = path.join(manifest.workspaceRoot, ".aria");
  const candidates: DiscoveredOperation[] = [];
  const queue: string[] = [ariaRoot];
  let guard = 0;
  while (queue.length > 0 && guard < 500) {
    guard += 1;
    const dir = queue.shift();
    if (dir === undefined) break;
    let entries: Dirent[];
    try {
      entries = await readdir(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        queue.push(full);
        continue;
      }
      if (!entry.name.endsWith(".json")) continue;
      try {
        const raw = await readFile(full, "utf8");
        const parsed = JSON.parse(raw) as Record<string, unknown>;
        const operationId = typeof parsed.operation_id === "string" ? parsed.operation_id : null;
        const status = typeof parsed.status === "string" ? parsed.status : null;
        if (operationId !== null && status !== null) {
          candidates.push({ operationId, status, raw });
        }
      } catch {
        /* 非 JSON/无权读:跳过 */
      }
    }
  }
  if (candidates.length === 0) return null;
  // 最近一次:按文件 mtime 不可靠,取数组最后一个(durable store 顺序写)。
  return candidates.at(-1) ?? null;
}

/** 把 supervisor 只读诊断(初始化 receipt 类)归档到 evidence。 */
export async function archiveInitializationDiagnostics(manifest: RunManifest, stage: string): Promise<string[]> {
  const archived: string[] = [];
  const dir = path.join(manifest.evidenceRoot, "backend-dumps", `${stage}-init-scan.json`);
  const operation = await findInitializationOperation(manifest);
  atomicWriteJson(dir, {
    stage,
    at: new Date().toISOString(),
    discovered: operation,
  });
  if (operation) archived.push(dir);
  return archived;
}
