import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import { ApiRequestError } from "./client";
import {
  cancelKnowledgeBaseOperation,
  createKnowledgeBaseBootstrapInputs,
  createKnowledgeBaseChangePackage,
  getActiveKnowledgeBaseOperation,
  getKnowledgeBaseOperation,
  getKnowledgeBaseStatus,
  lintKnowledgeBaseChangePackage,
  listKnowledgeBaseChangePackages,
  listKnowledgeBaseFiles,
  readKnowledgeBaseFile,
  startKnowledgeBaseInitialization,
  writeKnowledgeBaseFile,
} from "./knowledge-base";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function notFoundOperationResponse(): Response {
  return jsonResponse({ code: "knowledge_base_operation_not_found", message: "无进行中的知识库操作" }, 404);
}

function operationSnapshot() {
  return {
    operation_id: "kb_op_0001",
    project_id: "project 01",
    logical_codebase_id: "lc 01",
    kind: "bootstrap" as const,
    input: {
      idempotency_key: "idem-1",
      provider: "pi",
      package_path: null,
      package_id: null,
    },
    status: "running" as const,
    steps: [
      {
        step_id: "bootstrap",
        status: "running" as const,
        started_at: "2026-11-09T00:00:00Z",
        completed_at: null,
        output_summary: null,
      },
    ],
    provider_selection: null,
    error: null,
    created_at: "2026-11-09T00:00:00Z",
    started_at: "2026-11-09T00:00:01Z",
    completed_at: null,
  };
}

type FetchCall = [string, RequestInit | undefined];

function lastCall(fetchMock: Mock): FetchCall {
  const [path, init] = fetchMock.mock.calls.at(-1) as FetchCall;
  return [path, init];
}

describe("knowledge base api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("gets the knowledge base status with encoded ids", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        exists: true,
        schema_version: "4.0",
        initialization_status: "complete",
        global_validation: "passed",
        completed_stages: ["base-info"],
        skipped_stages: [],
        skills_available: true,
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const status = await getKnowledgeBaseStatus("project 01", "lc 01");

    expect(status.exists).toBe(true);
    expect(status.schema_version).toBe("4.0");
    const [path, init] = lastCall(fetchMock);
    expect(path).toBe(
      "/api/projects/project%2001/logical-codebases/lc%2001/knowledge-base/status",
    );
    expect(init?.method).toBeUndefined();
    expect(init?.body).toBeUndefined();
  });

  it("lists knowledge base files unwrapping nodes", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        nodes: [
          { path: "overview", is_dir: true, size: 0, editable: false },
          { path: "overview/base-info.md", is_dir: false, size: 128, editable: true },
        ],
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const nodes = await listKnowledgeBaseFiles("p1", "lc1");

    expect(nodes).toHaveLength(2);
    expect(nodes[1]).toEqual({
      path: "overview/base-info.md",
      is_dir: false,
      size: 128,
      editable: true,
    });
    const [path] = lastCall(fetchMock);
    expect(path).toBe("/api/projects/p1/logical-codebases/lc1/knowledge-base/files");
  });

  it("reads a knowledge base file with an encoded path query", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ path: "docs/概述.md", content: "# 内容", digest: "digest-1" }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const file = await readKnowledgeBaseFile("p1", "lc1", "docs/概述.md");

    expect(file.content).toBe("# 内容");
    expect(file.digest).toBe("digest-1");
    const [path] = lastCall(fetchMock);
    expect(path).toBe(
      "/api/projects/p1/logical-codebases/lc1/knowledge-base/file?path=docs%2F%E6%A6%82%E8%BF%B0.md",
    );
  });

  it("writes a knowledge base file with base digest", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ path: "docs/a.md", digest: "digest-2" }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = await writeKnowledgeBaseFile(
      "p1",
      "lc1",
      "docs/a.md",
      "# 新内容",
      "digest-1",
    );

    expect(result).toEqual({ path: "docs/a.md", digest: "digest-2" });
    const [path, init] = lastCall(fetchMock);
    expect(path).toBe("/api/projects/p1/logical-codebases/lc1/knowledge-base/file");
    expect(init?.method).toBe("PUT");
    expect(JSON.parse(String(init?.body))).toEqual({
      path: "docs/a.md",
      content: "# 新内容",
      base_digest: "digest-1",
    });
  });

  it("omits base digest when writing without optimistic concurrency", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ path: "docs/a.md", digest: "d" }));
    vi.stubGlobal("fetch", fetchMock);

    await writeKnowledgeBaseFile("p1", "lc1", "docs/a.md", "# 内容");

    const [, init] = lastCall(fetchMock);
    expect(JSON.parse(String(init?.body))).toEqual({
      path: "docs/a.md",
      content: "# 内容",
    });
  });

  it("lists change packages unwrapping packages", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        packages: [
          {
            id: "CHANGE-2026-1109-登录改造",
            path: "cadence/knowledge-base/user-input/updates/CHANGE-2026-1109-登录改造",
            violation_count: 1,
            violations: [
              {
                code: "knowledge_base_change_package_field_empty",
                document: "code-change.md",
                field: "Merge Request 地址或编号",
                detail: "字段为空",
              },
            ],
          },
        ],
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const packages = await listKnowledgeBaseChangePackages("p1", "lc1");

    expect(packages).toHaveLength(1);
    expect(packages[0]?.violation_count).toBe(1);
    expect(packages[0]?.violations[0]?.code).toBe("knowledge_base_change_package_field_empty");
    const [path] = lastCall(fetchMock);
    expect(path).toBe("/api/projects/p1/logical-codebases/lc1/knowledge-base/change-packages");
  });

  it("lints a change package with an encoded path query", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        violations: [],
        fields: [{ document: "summary.md", field: "变更标题", filled: true }],
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const lint = await lintKnowledgeBaseChangePackage(
      "p1",
      "lc1",
      "cadence/knowledge-base/user-input/updates/CHANGE-x",
    );

    expect(lint.violations).toEqual([]);
    expect(lint.fields[0]?.filled).toBe(true);
    const [path] = lastCall(fetchMock);
    expect(path).toBe(
      "/api/projects/p1/logical-codebases/lc1/knowledge-base/change-packages/lint" +
        "?path=cadence%2Fknowledge-base%2Fuser-input%2Fupdates%2FCHANGE-x",
    );
  });

  it("creates a change package with identifier body", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        id: "CHANGE-2026-1109-登录改造",
        path: "cadence/knowledge-base/user-input/updates/CHANGE-2026-1109-登录改造",
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const created = await createKnowledgeBaseChangePackage(
      "p1",
      "lc1",
      "2026-1109-登录改造",
    );

    expect(created.id).toBe("CHANGE-2026-1109-登录改造");
    const [path, init] = lastCall(fetchMock);
    expect(path).toBe("/api/projects/p1/logical-codebases/lc1/knowledge-base/change-packages");
    expect(init?.method).toBe("POST");
    expect(JSON.parse(String(init?.body))).toEqual({ identifier: "2026-1109-登录改造" });
  });

  it("creates bootstrap inputs", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ created: ["base-info.md", "product.md"] }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = await createKnowledgeBaseBootstrapInputs("p1", "lc1");

    expect(result.created).toEqual(["base-info.md", "product.md"]);
    const [path, init] = lastCall(fetchMock);
    expect(path).toBe("/api/projects/p1/logical-codebases/lc1/knowledge-base/bootstrap-inputs");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBeUndefined();
  });

  it("starts a bootstrap initialization without package path", async () => {
    const fetchMock = vi.fn(async () => jsonResponse(operationSnapshot(), 202));
    vi.stubGlobal("fetch", fetchMock);

    const snapshot = await startKnowledgeBaseInitialization(
      "p1",
      "lc1",
      "bootstrap",
      "idem-1",
    );

    expect(snapshot.operation_id).toBe("kb_op_0001");
    const [path, init] = lastCall(fetchMock);
    expect(path).toBe("/api/projects/p1/logical-codebases/lc1/knowledge-base/initializations");
    expect(init?.method).toBe("POST");
    expect(JSON.parse(String(init?.body))).toEqual({
      kind: "bootstrap",
      idempotency_key: "idem-1",
    });
  });

  it("starts an update initialization passing package path through", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ ...operationSnapshot(), kind: "update" }, 202));
    vi.stubGlobal("fetch", fetchMock);

    const snapshot = await startKnowledgeBaseInitialization(
      "p1",
      "lc1",
      "update",
      "idem-2",
      "cadence/knowledge-base/user-input/updates/CHANGE-x",
    );

    expect(snapshot.kind).toBe("update");
    const [, init] = lastCall(fetchMock);
    expect(JSON.parse(String(init?.body))).toEqual({
      kind: "update",
      package_path: "cadence/knowledge-base/user-input/updates/CHANGE-x",
      idempotency_key: "idem-2",
    });
  });

  it("gets the active knowledge base operation with encoded ids", async () => {
    const fetchMock = vi.fn(async () => jsonResponse(operationSnapshot()));
    vi.stubGlobal("fetch", fetchMock);

    const snapshot = await getActiveKnowledgeBaseOperation("project 01", "lc 01");

    expect(snapshot?.operation_id).toBe("kb_op_0001");
    const [path] = lastCall(fetchMock);
    expect(path).toBe(
      "/api/projects/project%2001/logical-codebases/lc%2001/knowledge-base/initializations/active",
    );
  });

  it("resolves a missing active operation as null", async () => {
    const fetchMock = vi.fn(async () => notFoundOperationResponse());
    vi.stubGlobal("fetch", fetchMock);

    const snapshot = await getActiveKnowledgeBaseOperation("p1", "lc1");

    expect(snapshot).toBeNull();
  });

  it("propagates other active operation failures as ApiRequestError", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ code: "web_server_error", message: "故障" }, 500));
    vi.stubGlobal("fetch", fetchMock);

    await expect(getActiveKnowledgeBaseOperation("p1", "lc1")).rejects.toBeInstanceOf(
      ApiRequestError,
    );
  });

  it("gets an operation by encoded operation id", async () => {
    const fetchMock = vi.fn(async () => jsonResponse(operationSnapshot()));
    vi.stubGlobal("fetch", fetchMock);

    const snapshot = await getKnowledgeBaseOperation("p1", "lc1", "kb op/1");

    expect(snapshot.operation_id).toBe("kb_op_0001");
    const [path] = lastCall(fetchMock);
    expect(path).toBe(
      "/api/projects/p1/logical-codebases/lc1/knowledge-base/initializations/kb%20op%2F1",
    );
  });

  it("cancels an operation and returns the snapshot", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ ...operationSnapshot(), status: "cancelled" }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const snapshot = await cancelKnowledgeBaseOperation("p1", "lc1", "kb_op_0001");

    expect(snapshot.status).toBe("cancelled");
    const [path, init] = lastCall(fetchMock);
    expect(path).toBe(
      "/api/projects/p1/logical-codebases/lc1/knowledge-base/initializations/kb_op_0001/cancel",
    );
    expect(init?.method).toBe("POST");
  });

  it("normalizes failed requests as ApiRequestError", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ code: "knowledge_base_not_initialized", message: "知识库尚未初始化" }, 409),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(getKnowledgeBaseStatus("p1", "lc1")).rejects.toMatchObject({
      name: "ApiRequestError",
      code: "knowledge_base_not_initialized",
    });
  });
});
