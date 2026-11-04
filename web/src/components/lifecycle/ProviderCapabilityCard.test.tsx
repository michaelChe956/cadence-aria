// add-provider-revalidate-probe Task 3.1：capability 卡真实 DTO/按钮交互回归。
//
// - ProviderCapabilityCard：挂载即 GET 只读投影（URL 逐字断言），四家
//   provider 以【行级列表】渲染（owner 2026-11-04：删掉 2×2 方格卡片），
//   每行带 launch/write_boundary/resume 三项聚合状态（未核验/已核验/拒绝，
//   聚合口径=三 action 行逐 aspect 全 Confirmed 才显示已核验）与 provider
//   级状态（已确认/未核验）；
// - 点击「核验」经真实 fetch 发送器打 POST capability-revalidate（URL/
//   method/body 逐条断言）；already_confirmed/revalidated 如实回显，成功后
//   重新 GET 同一 capabilities 面刷新三行（不乐观改状态）；
// - POST 在途时按钮禁用＋核验中提示（首次真实探针分钟级，用户需可见等待）；
// - 失败（provider_capability_probe_failed）如实展示稳定码+原始 detail，
//   不伪造 Confirmed，按钮恢复可点击；
// - 批量核验作业的逐项状态并入同一行级列表（不再另起第二组列表）。
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderCapabilityCard } from "./ProviderCapabilityCard";
import type {
  ProviderCapabilityActionName,
  ProviderCapabilityActionRowDto,
  ProviderCapabilityEvidenceStatus,
  ProviderCapabilitiesResponse,
} from "../../api/types";

const PROJECT_ID = "project_0001";
const LC_ID = "logical_codebase_0001";
const BATCH_JOB_ID = "job_0001";
const BATCH_STORAGE_KEY = `aria:capability-batch:${PROJECT_ID}:${LC_ID}`;

type CapturedRequest = { url: string; method: string; body: unknown };
let captured: CapturedRequest[];

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

type RevalidateResponder = () => Promise<Response>;

/** GET 投影 + 安装目标面 + 可配置 POST 应答的 fetch 路由（真实发送器路径）。 */
function installFetchRouter(
  getResponse: () => ProviderCapabilitiesResponse,
  postResponse?: RevalidateResponder,
  installTargets: unknown = { targets: [] },
  batchJob: unknown = null,
): void {
  captured = [];
  const router = async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    const rawBody = typeof init?.body === "string" ? init.body : null;
    captured.push({
      url,
      method,
      body: rawBody === null ? null : JSON.parse(rawBody),
    });
    if (
      method === "GET" &&
      url === `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capabilities`
    ) {
      return jsonResponse(getResponse());
    }
    if (method === "GET" && url === "/api/providers/install-targets") {
      return jsonResponse(installTargets);
    }
    if (
      method === "GET" &&
      url ===
        `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capabilities/revalidate-all/${BATCH_JOB_ID}`
    ) {
      return jsonResponse(batchJob);
    }
    if (
      method === "POST" &&
      url ===
        `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capability-revalidate`
    ) {
      if (!postResponse) {
        return jsonResponse(
          { code: "not_found", message: `no POST route for ${url}` },
          404,
        );
      }
      return postResponse();
    }
    return jsonResponse(
      { code: "not_found", message: `no route for ${method} ${url}` },
      404,
    );
  };
  vi.stubGlobal("fetch", vi.fn(router));
}

const row = (
  action: ProviderCapabilityActionName,
  launch: ProviderCapabilityEvidenceStatus,
  resume: ProviderCapabilityEvidenceStatus,
  writeBoundary: ProviderCapabilityEvidenceStatus,
): ProviderCapabilityActionRowDto => ({
  action,
  launch,
  resume,
  write_boundary: writeBoundary,
  evidence_ref: `evidence/${action}`,
});

/** 未探测基线：bootstrap 默认记录不是真实证据，全 Unknown。 */
function unknownProjection(): ProviderCapabilitiesResponse {
  return {
    providers: [
      {
        provider_type: "claude_code",
        cli_program: "claude",
        version: "1.0.34",
        probed_at: "2026-10-10T00:00:00Z",
        probe_artifact_ref: "artifact://probe/claude",
        rows: [
          row("coding_target_write", "confirmed", "confirmed", "confirmed"),
          row("planning_read_only", "confirmed", "confirmed", "confirmed"),
          row("review_read_only", "confirmed", "confirmed", "confirmed"),
        ],
      },
      {
        provider_type: "codex",
        cli_program: "codex",
        version: null,
        probed_at: null,
        probe_artifact_ref: null,
        rows: [
          row("coding_target_write", "unknown", "unknown", "unknown"),
          row("planning_read_only", "unknown", "unknown", "unknown"),
          row("review_read_only", "unknown", "unknown", "unknown"),
        ],
      },
      {
        provider_type: "pi",
        cli_program: "pi",
        version: null,
        probed_at: null,
        probe_artifact_ref: null,
        rows: [
          row("coding_target_write", "unknown", "unknown", "unknown"),
          row("planning_read_only", "unknown", "unknown", "unknown"),
          row("review_read_only", "unknown", "unknown", "unknown"),
        ],
      },
      {
        provider_type: "kimi_code",
        cli_program: "kimi",
        version: "0.9.1",
        probed_at: "2026-10-09T00:00:00Z",
        probe_artifact_ref: "artifact://probe/kimi",
        rows: [
          row("coding_target_write", "confirmed", "confirmed", "denied"),
          row("planning_read_only", "unknown", "confirmed", "denied"),
          row("review_read_only", "unknown", "confirmed", "denied"),
        ],
      },
    ],
  };
}

/** 全行 Confirmed 投影（revalidate 成功后 GET 的刷新事实）。 */
function confirmedCodexProjection(): ProviderCapabilitiesResponse {
  const projection = unknownProjection();
  const codex = projection.providers.find(
    (provider) => provider.provider_type === "codex",
  )!;
  codex.version = "0.44.0";
  codex.probed_at = "2026-10-10T01:00:00Z";
  codex.probe_artifact_ref = "artifact://probe/codex";
  codex.rows = [
    row("coding_target_write", "confirmed", "confirmed", "confirmed"),
    row("planning_read_only", "confirmed", "confirmed", "confirmed"),
    row("review_read_only", "confirmed", "confirmed", "confirmed"),
  ];
  return projection;
}

/** 批量核验作业（终态）：逐项状态并入行级列表。 */
function batchJob(codexStatus: "confirmed" | "failed" = "failed"): unknown {
  return {
    job_id: BATCH_JOB_ID,
    status: "completed",
    created_at: "2026-11-04T00:00:00Z",
    updated_at: "2026-11-04T00:01:00Z",
    finished_at: "2026-11-04T00:01:00Z",
    results: [
      {
        provider_type: "claude_code",
        status: "confirmed",
        reason: "revalidated",
        detail: null,
        version: "1.0.34",
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "codex",
        status: codexStatus,
        reason:
          codexStatus === "confirmed"
            ? "revalidated"
            : "probe_failed",
        detail:
          codexStatus === "confirmed"
            ? null
            : "codex --version exited 127: command not found",
        version: codexStatus === "confirmed" ? "0.44.0" : null,
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "pi",
        status: "skipped",
        reason: "cli_not_installed",
        detail: null,
        version: null,
        started_at: null,
        finished_at: null,
      },
      {
        provider_type: "kimi_code",
        status: "confirmed",
        reason: "already_confirmed",
        detail: null,
        version: "0.9.1",
        started_at: null,
        finished_at: null,
      },
    ],
  };
}

describe("ProviderCapabilityCard revalidate surface", () => {
  beforeEach(() => {
    captured = [];
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("renders nothing without an active logical codebase", async () => {
    const { container } = render(
      <ProviderCapabilityCard projectId={PROJECT_ID} logicalCodebaseId={null} />,
    );
    expect(container.firstChild).toBeNull();
    // 不发任何请求。
    await Promise.resolve();
    expect(captured).toEqual([]);
  });

  it("fetches the read-only projection and renders three aspect rows per provider", async () => {
    installFetchRouter(unknownProjection);

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    await screen.findByTestId("provider-capability-card");
    // 只读 GET（零写入、零探针触发）：首请求即 capability 投影，全程无 POST。
    expect(captured[0]).toEqual({
      url: `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capabilities`,
      method: "GET",
      body: null,
    });
    expect(captured.filter((request) => request.method === "POST")).toEqual([]);
    // 安装目标面（本地可用性 / 版本）同样是只读 GET。
    expect(
      captured.filter(
        (request) =>
          request.method === "GET" &&
          request.url === "/api/providers/install-targets",
      ),
    ).toHaveLength(1);

    // 四家 provider 全渲染（行级列表：每家一行）。
    expect(
      screen.getByTestId("provider-capability-provider-claude_code"),
    ).toBeTruthy();
    expect(
      screen.getByTestId("provider-capability-provider-codex"),
    ).toBeTruthy();
    expect(screen.getByTestId("provider-capability-provider-pi")).toBeTruthy();
    expect(
      screen.getByTestId("provider-capability-provider-kimi_code"),
    ).toBeTruthy();

    // codex 未探测：三个能力维度全未核验，provider 级状态为「未核验」。
    expect(
      screen.getByTestId("provider-capability-codex-launch").textContent,
    ).toBe("未核验");
    expect(
      screen.getByTestId("provider-capability-codex-write_boundary")
        .textContent,
    ).toBe("未核验");
    expect(
      screen.getByTestId("provider-capability-codex-resume").textContent,
    ).toBe("未核验");
    expect(
      screen.getByTestId("provider-capability-codex-version").textContent,
    ).toBe("未探测");
    expect(
      screen.getByTestId("provider-capability-status-codex").textContent,
    ).toBe("未核验");

    // claude_code 全 Confirmed：三维度已核验，provider 级「已确认」，版本如实展示。
    expect(
      screen.getByTestId("provider-capability-claude_code-launch").textContent,
    ).toBe("已核验");
    expect(
      screen.getByTestId("provider-capability-claude_code-write_boundary")
        .textContent,
    ).toBe("已核验");
    expect(
      screen.getByTestId("provider-capability-claude_code-resume").textContent,
    ).toBe("已核验");
    expect(
      screen.getByTestId("provider-capability-claude_code-version").textContent,
    ).toBe("1.0.34");
    expect(
      screen.getByTestId("provider-capability-status-claude_code").textContent,
    ).toBe("已确认");

    // kimi_code 混合：launch 有 Unknown → 未核验；write_boundary 全 denied →
    // 拒绝；resume 全 confirmed → 已核验（聚合不夸大、不遮蔽）。
    expect(
      screen.getByTestId("provider-capability-kimi_code-launch").textContent,
    ).toBe("未核验");
    expect(
      screen.getByTestId("provider-capability-kimi_code-write_boundary")
        .textContent,
    ).toBe("拒绝");
    expect(
      screen.getByTestId("provider-capability-kimi_code-resume").textContent,
    ).toBe("已核验");
  });

  // owner 2026-11-04 裁定：删掉 2×2 方格卡片，只保留行级列表 ⇒ 断言卡片
  // 【不再渲染】、行级列表【渲染】，且行内承载原卡片里的全部信息。
  it("renders one row list instead of the 2x2 provider cards", async () => {
    installFetchRouter(unknownProjection);

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    // 行级列表渲染。
    const list = await screen.findByTestId("provider-capability-list");
    // 方格卡片（两列网格）不再渲染。
    expect(list.className).not.toContain("grid-cols-2");
    expect(list.className).not.toContain("sm:grid-cols-2");
    // 每家 provider 一行，且行内不再有卡片自带的嵌套维度列表 ⇒ 整块只有 4 行。
    expect(screen.getAllByRole("listitem")).toHaveLength(4);
    const providerRow = screen.getByTestId(
      "provider-capability-provider-claude_code",
    );
    // provider 元素不再是带圆角边框的方格卡片。
    expect(providerRow.className).not.toContain("rounded");
    expect(providerRow.className).not.toContain("border");
    // 原卡片里的信息一个不少地进了这一行：状态 + 三个能力维度 + 版本。
    expect(
      screen.getByTestId("provider-capability-status-claude_code").textContent,
    ).toBe("已确认");
    expect(
      screen.getByTestId("provider-capability-claude_code-launch").textContent,
    ).toBe("已核验");
    expect(
      screen.getByTestId("provider-capability-claude_code-write_boundary")
        .textContent,
    ).toBe("已核验");
    expect(
      screen.getByTestId("provider-capability-claude_code-resume").textContent,
    ).toBe("已核验");
    expect(
      screen.getByTestId("provider-capability-claude_code-version").textContent,
    ).toBe("1.0.34");
    // 单 provider 核验入口仍在行内（能力核验行为不变）。
    expect(
      screen.getByTestId("provider-capability-revalidate-claude_code"),
    ).toBeTruthy();
  });

  it("keeps the version-gap entry (版本落后 / 更新 / 当前→最新) in the row list", async () => {
    installFetchRouter(() => confirmedCodexProjection(), undefined, {
      targets: [
        {
          target: "codex",
          display_name: "Codex",
          cli_available: true,
          local_version: "0.44.0",
          latest_version: "1.0.1",
          latest_checked_at: "2026-11-03T00:00:00Z",
          install_source: "npm @openai/codex",
          action: "update",
        },
      ],
    });

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    const providerRow = await screen.findByTestId(
      "provider-capability-provider-codex",
    );
    // 版本落后提示 +【更新】+ 当前 → 最新版本，全部在行内。
    expect(
      within(providerRow).getByTestId("provider-capability-outdated-codex"),
    ).toHaveTextContent("版本落后");
    expect(
      within(providerRow).getByTestId("provider-install-start-codex"),
    ).toHaveTextContent("更新");
    expect(
      within(providerRow).getByTestId("provider-install-versions-codex"),
    ).toHaveTextContent("当前 0.44.0 → 最新 1.0.1");
  });

  it("folds the batch job item state into the same row list", async () => {
    window.localStorage.setItem(BATCH_STORAGE_KEY, BATCH_JOB_ID);
    installFetchRouter(
      unknownProjection,
      () =>
        Promise.resolve(
          jsonResponse({
            provider_type: "codex",
            outcome: "already_confirmed",
            version: "0.44.0",
          }),
        ),
      { targets: [] },
      batchJob(),
    );

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    // 批量作业的逐项状态并入对应行（不再另起第二组列表）。
    expect(
      await screen.findByTestId("capability-batch-status-claude_code"),
    ).toHaveTextContent("批量：已确认");
    expect(
      screen.getByTestId("capability-batch-status-codex").textContent,
    ).toBe("批量：核验失败");
    expect(
      screen.getByTestId("capability-batch-status-pi").textContent,
    ).toBe("批量：跳过（CLI 不可用）");
    // 失败项保留【重试】（走单 provider 端点，不重跑整批）。
    const retryButton = screen.getByTestId("capability-batch-retry-codex");
    await userEvent.click(retryButton);
    await waitFor(() => {
      expect(
        captured.find(
          (request) =>
            request.method === "POST" &&
            request.url ===
              `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capability-revalidate`,
        ),
      ).toEqual({
        url: `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capability-revalidate`,
        method: "POST",
        body: { provider_type: "codex" },
      });
    });
    // 整块只有一组行级列表（4 行），不再有第二组逐项列表。
    expect(screen.getAllByRole("listitem")).toHaveLength(4);
    expect(
      screen.queryByTestId("provider-capability-batch-results"),
    ).toBeNull();
  });

  it("keeps the update entry for a provider the batch job confirmed", async () => {
    window.localStorage.setItem(BATCH_STORAGE_KEY, BATCH_JOB_ID);
    installFetchRouter(unknownProjection, undefined, {
      targets: [
        {
          target: "codex",
          display_name: "Codex",
          cli_available: true,
          local_version: "0.44.0",
          latest_version: "1.0.1",
          latest_checked_at: "2026-11-03T00:00:00Z",
          install_source: "npm @openai/codex",
          action: "update",
        },
      ],
    }, batchJob("confirmed"));

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    const providerRow = await screen.findByTestId(
      "provider-capability-provider-codex",
    );
    // 批量作业已确认这一家 ⇒ 行内保留【版本落后】+【更新】+ 当前 → 最新版本
    // （批量作业的逐项结论与投影同源，不因投影尚未刷新而丢掉更新入口）。
    expect(
      await screen.findByTestId("capability-batch-status-codex"),
    ).toHaveTextContent("批量：已确认");
    expect(
      within(providerRow).getByTestId("provider-capability-outdated-codex"),
    ).toHaveTextContent("版本落后");
    expect(
      within(providerRow).getByTestId("provider-install-start-codex"),
    ).toHaveTextContent("更新");
    expect(
      within(providerRow).getByTestId("provider-install-versions-codex"),
    ).toHaveTextContent("当前 0.44.0 → 最新 1.0.1");
  });

  it("dispatches POST revalidate for the clicked provider and echoes already_confirmed", async () => {
    let revalidated = false;
    installFetchRouter(
      () => (revalidated ? confirmedCodexProjection() : unknownProjection()),
      () => {
        revalidated = true;
        return Promise.resolve(
          jsonResponse({
            provider_type: "codex",
            outcome: "already_confirmed",
            version: "0.44.0",
          }),
        );
      },
    );

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );
    await screen.findByTestId("provider-capability-card");

    await userEvent.click(
      screen.getByTestId("provider-capability-revalidate-codex"),
    );

    await waitFor(() => {
      expect(
        captured.find((request) => request.method === "POST"),
      ).toEqual({
        url: `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capability-revalidate`,
        method: "POST",
        body: { provider_type: "codex" },
      });
    });
    // already_confirmed 如实回显（含版本）。
    await waitFor(() => {
      expect(
        screen.getByTestId("provider-capability-result-codex").textContent,
      ).toContain("already_confirmed");
      expect(
        screen.getByTestId("provider-capability-result-codex").textContent,
      ).toContain("0.44.0");
    });
    // 成功后同一 GET 面补读刷新三行（不乐观改状态）。
    await waitFor(() => {
      expect(
        screen.getByTestId("provider-capability-codex-launch").textContent,
      ).toBe("已核验");
      expect(
        screen.getByTestId("provider-capability-codex-resume").textContent,
      ).toBe("已核验");
    });
    // 核验成功后 capability 投影被重新读取（POST 之后仍有该面 GET）。
    expect(
      captured.filter(
        (request, index) =>
          request.method === "GET" &&
          request.url ===
            `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/capabilities` &&
          index > captured.findIndex((item) => item.method === "POST"),
      ).length,
    ).toBeGreaterThan(0);
  });

  it("echoes revalidated with imported actions and version", async () => {
    let revalidated = false;
    installFetchRouter(
      () => (revalidated ? confirmedCodexProjection() : unknownProjection()),
      () => {
        revalidated = true;
        return Promise.resolve(
          jsonResponse({
            provider_type: "codex",
            outcome: "revalidated",
            version: "0.44.0",
            imported_actions: [
              "coding_target_write",
              "planning_read_only",
              "review_read_only",
            ],
          }),
        );
      },
    );

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );
    await screen.findByTestId("provider-capability-card");

    await userEvent.click(
      screen.getByTestId("provider-capability-revalidate-codex"),
    );

    await waitFor(() => {
      const result = screen.getByTestId("provider-capability-result-codex");
      expect(result.textContent).toContain("revalidated");
      expect(result.textContent).toContain("0.44.0");
      expect(result.textContent).toContain("coding_target_write");
      expect(result.textContent).toContain("review_read_only");
    });
    await waitFor(() => {
      expect(
        screen.getByTestId("provider-capability-codex-write_boundary")
          .textContent,
      ).toBe("已核验");
    });
  });

  it("shows the probing state with a disabled button while the POST is in flight", async () => {
    let releasePost: ((response: Response) => void) | null = null;
    installFetchRouter(
      unknownProjection,
      () =>
        new Promise<Response>((resolve) => {
          releasePost = resolve;
        }),
    );

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );
    await screen.findByTestId("provider-capability-card");

    await userEvent.click(
      screen.getByTestId("provider-capability-revalidate-codex"),
    );

    // 在途：核验中提示可见（首次真实探针分钟级，用户可见等待）＋按钮禁用。
    await waitFor(() => {
      expect(
        screen.getByTestId("provider-capability-result-codex").textContent,
      ).toContain("核验中");
    });
    const button = screen.getByTestId(
      "provider-capability-revalidate-codex",
    ) as HTMLButtonElement;
    expect(button.disabled).toBe(true);

    // 闭包内赋值对控制流不可见：点击后必已就绪，非空断言释放。
    releasePost!(
      jsonResponse({
        provider_type: "codex",
        outcome: "already_confirmed",
        version: "0.44.0",
      }),
    );

    await waitFor(() => {
      expect(
        screen.getByTestId("provider-capability-result-codex").textContent,
      ).toContain("already_confirmed");
    });
    expect(button.disabled).toBe(false);
  });

  it("surfaces probe failure honestly with the stable code and raw detail", async () => {
    installFetchRouter(
      unknownProjection,
      () =>
        Promise.resolve(
          jsonResponse(
            {
              code: "provider_capability_probe_failed",
              message:
                "provider capability revalidation probe failed; no Confirmed was fabricated",
              details: {
                provider_type: "codex",
                action: "coding_target_write",
                detail: "codex --version exited 127: command not found",
              },
            },
            503,
          ),
        ),
    );

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );
    await screen.findByTestId("provider-capability-card");

    await userEvent.click(
      screen.getByTestId("provider-capability-revalidate-codex"),
    );

    // 失败如实展示：稳定码＋原始 detail；不伪造 Confirmed。
    await waitFor(() => {
      const result = screen.getByTestId("provider-capability-result-codex");
      expect(result.textContent).toContain("provider_capability_probe_failed");
      expect(result.textContent).toContain("command not found");
    });
    expect(
      screen.getByTestId("provider-capability-codex-launch").textContent,
    ).toBe("未核验");
    // 按钮恢复可点击（失败不吞成成功，可重试）。
    const button = screen.getByTestId(
      "provider-capability-revalidate-codex",
    ) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
  });

  it("renders the install entry with its manual command for a provider without a CLI", async () => {
    installFetchRouter(unknownProjection, undefined, {
      targets: [
        {
          target: "codex",
          display_name: "Codex",
          cli_available: false,
          local_version: null,
          latest_version: "1.0.1",
          latest_checked_at: "2026-11-03T00:00:00Z",
          install_source: "npm @openai/codex",
          action: "install",
        },
      ],
    });

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    const installButton = await screen.findByTestId(
      "provider-install-start-codex",
    );
    expect(installButton).toHaveTextContent("安装");
    expect(
      screen.getByTestId("provider-install-manual-command-codex"),
    ).toHaveTextContent("@openai/codex");
  });

  it("offers the update entry when the verified local version is behind latest", async () => {
    installFetchRouter(() => confirmedCodexProjection(), undefined, {
      targets: [
        {
          target: "codex",
          display_name: "Codex",
          cli_available: true,
          local_version: "0.44.0",
          latest_version: "1.0.1",
          latest_checked_at: "2026-11-03T00:00:00Z",
          install_source: "npm @openai/codex",
          action: "update",
        },
      ],
    });

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    const updateButton = await screen.findByTestId(
      "provider-install-start-codex",
    );
    expect(updateButton).toHaveTextContent("更新");
    expect(
      screen.getByTestId("provider-install-versions-codex"),
    ).toHaveTextContent("1.0.1");
  });

  // owner 2026-11-04 裁定：codegraph 不是 provider，不展示在「Provider 能力核验」
  // 面板里 ⇒ 断言【不出现】，不是「可选」。
  it("does not show codegraph in the provider capability panel", async () => {
    installFetchRouter(unknownProjection, undefined, {
      targets: [
        {
          target: "codegraph",
          display_name: "codegraph",
          cli_available: false,
          local_version: null,
          latest_version: "0.85.0",
          latest_checked_at: "2026-11-03T00:00:00Z",
          install_source: "npm @colbymchenry/codegraph",
          action: "install",
        },
      ],
    });

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    await screen.findByTestId("provider-capability-card");
    expect(
      screen.queryByTestId("provider-install-target-codegraph"),
    ).toBeNull();
    expect(
      screen.queryByTestId("provider-install-start-codegraph"),
    ).toBeNull();
    expect(
      screen.queryByTestId("provider-install-manual-command-codegraph"),
    ).toBeNull();
    // 面板里也不出现 codegraph 字样。
    expect(screen.queryByText("codegraph")).toBeNull();
  });

  // owner 2026-11-04 裁定：「一键核验全部」按钮放在上面 ⇒ 挂在能力卡头部，
  // 且在四家 provider 行之前。
  it("puts the batch revalidate button at the top of the card", async () => {
    installFetchRouter(unknownProjection);

    render(
      <ProviderCapabilityCard
        projectId={PROJECT_ID}
        logicalCodebaseId={LC_ID}
      />,
    );

    const card = await screen.findByTestId("provider-capability-card");
    const startButton = screen.getByTestId("capability-batch-start");
    expect(startButton).toHaveTextContent("一键核验全部");
    expect(card).toContainElement(startButton);
    const firstProviderRow = screen.getByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(
      startButton.compareDocumentPosition(firstProviderRow) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // 按钮区域说明作用域与成本（REQ-PCR-06）。
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "逐 LC 核验",
    );
  });
});
