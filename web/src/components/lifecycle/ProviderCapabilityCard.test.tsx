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
//
// 2026-11-04（宿主迁移）：owner 裁定能力核验已全局化 ⇒ provider 管理不再寄居
// 在「逻辑代码库」抽屉里，本文件的渲染宿主换成独立的「Provider 管理」面板
// （ProviderManagementDrawer + ProviderManagementPanel）。断言一条未删、未改松，
// 只换宿主；结构性断言（块间分割线 / 行1 两端对齐 / 行2 胶囊 / 行3 维度在前
// 操作在后 / 核验按钮次按钮 / 块内 ≥3 独立行容器 / 无圆角边框壳 / 无两列网格
// / 4 个 listitem）全部保留。
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { InstallTargetView, InstallTargetWire } from "../../api/types";
import { ProviderCapabilityCard } from "./ProviderCapabilityCard";
import { ProviderManagementDrawer } from "./ProviderManagementDrawer";
import { ProviderManagementPanel } from "./ProviderManagementPanel";
import {
  BATCH_JOB_ID,
  BATCH_STORAGE_KEY,
  BATCH_URL,
  CAPABILITIES_URL,
  REVALIDATE_URL,
  batchJob,
  captured,
  confirmedCodexProjection,
  installFetchRouter,
  jsonResponse,
  runningBatchJob,
  unknownProjection,
} from "./ProviderCapabilityCard.test-utils";

/** F3：安装目标面夹具——只写用例关心的字段，其余给合理默认。 */
function installTargetFixture(
  overrides: Partial<InstallTargetView> & { target: InstallTargetWire },
): InstallTargetView {
  return {
    display_name: overrides.target,
    cli_available: true,
    local_version: "1.0.0",
    latest_version: "1.0.0",
    latest_checked_at: "2026-11-13T00:00:00Z",
    install_source: "npm",
    action: "none",
    off_path: false,
    ...overrides,
  };
}

/** F3：本地 fetch 桩——能力投影（未核验基线）+ 安装目标面 + expose 端点。 */
function stubCatalogFetch(targets: InstallTargetView[]): void {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString();
      const method = (init?.method ?? "GET").toUpperCase();
      captured.push({ url, method, body: null });
      if (method === "GET" && url === CAPABILITIES_URL) {
        return jsonResponse(unknownProjection());
      }
      if (method === "GET" && url === "/api/providers/install-targets") {
        return jsonResponse({ targets });
      }
      if (method === "POST" && url.endsWith("/expose")) {
        return jsonResponse({
          exposed: true,
          command: "codex",
          dir: "/home/demo/.local/bin",
          target: "codex",
        });
      }
      return jsonResponse(
        { code: "not_found", message: `no route for ${method} ${url}` },
        404,
      );
    }),
  );
}

describe("ProviderCapabilityCard revalidate surface", () => {
  beforeEach(() => {
    captured.length = 0;
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  /** 2026-11-04：能力核验已全局化 ⇒ 能力卡不再寄居在「逻辑代码库」抽屉里，
   *  宿主换成独立的「Provider 管理」面板（抽屉 + 面板）。断言内容一条未减，
   *  只换宿主。 */
  function renderProviderManagement() {
    return render(
      <ProviderManagementDrawer open onClose={() => {}}>
        <ProviderManagementPanel />
      </ProviderManagementDrawer>,
    );
  }

  // 原「renders nothing without an active logical codebase」用例随 2026-11-05
  // 机器级化（REQ-PCR-08）删除：能力卡不再接收 id，「缺 LC id 就不渲染」的
  // 门控行为不复存在 ⇒ 由下方机器级渲染用例接替（不传任何 id 仍渲染四家）。


  // 2026-11-05 机器级化（REQ-PCR-08：能力面不再按项目/逻辑代码库定位）：
  // 能力卡 props 不再携带任何 id，仍要渲染四家 provider 行（机器一份事实，
  // 与项目、逻辑代码库无关）。
  it("capability card renders without any project or lc id (machine-level surface)", async () => {
    installFetchRouter(unknownProjection);

    render(<ProviderCapabilityCard />);

    for (const providerType of [
      "claude_code",
      "codex",
      "pi",
      "kimi_code",
    ]) {
      expect(
        await screen.findByTestId(
          `provider-capability-provider-${providerType}`,
        ),
      ).toBeInTheDocument();
    }
    // 只读 GET 打机器级路由（路径不含项目/逻辑代码库标识）。
    expect(
      captured.find(
        (request) =>
          request.method === "GET" &&
          request.url === "/api/providers/capabilities",
      ),
    ).toBeTruthy();
  });

  it("fetches the read-only projection and renders three aspect rows per provider", async () => {
    installFetchRouter(unknownProjection);

    renderProviderManagement();

    await screen.findByTestId("provider-capability-card");
    // 只读 GET（零写入、零探针触发）：首请求即 capability 投影，全程无 POST。
    expect(captured[0]).toEqual({
      url: CAPABILITIES_URL,
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

    renderProviderManagement();

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

  // owner 2026-11-04 视觉重排：每家 provider 一个**块**（块间 1px 分割线
  // --aria-line，不套圆角边框卡片壳），块内固定三行 grid 对齐——行1 名与版本
  // 两端对齐、行2 状态胶囊、行3 左维度胶囊 + 右操作按钮，且不退化成一段
  // 连续文字流。断言对象换成新结构，语义只增不减。
  it("lays out each provider as a separated block with three aligned rows", async () => {
    installFetchRouter(unknownProjection);

    renderProviderManagement();

    const list = await screen.findByTestId("provider-capability-list");
    // 块间 1px 分割线挂在列表容器上（每个块自身没有边框壳）。
    expect(list.className).toContain("divide-y");
    expect(list.className).toContain("divide-[var(--aria-line)]");
    const providerRow = screen.getByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(providerRow.className).not.toContain("rounded");
    expect(providerRow.className).not.toContain("border");

    // 行1：provider 名（600 字重）在左，版本在右（次要色 + 等宽字体）。
    const head = within(providerRow).getByTestId(
      "provider-capability-head-claude_code",
    );
    expect(head.className).toContain("grid");
    expect(head.className).toContain("grid-cols-[1fr_auto]");
    expect(
      within(providerRow).getByTestId("provider-capability-name-claude_code"),
    ).toHaveClass("font-semibold");
    const version = within(providerRow).getByTestId(
      "provider-capability-claude_code-version",
    );
    expect(version).toHaveClass("text-right");
    expect(version).toHaveClass("aria-mono");
    expect(version).toHaveClass("text-[var(--aria-ink-muted)]");

    // 行2：状态胶囊们（flex-wrap + gap），胶囊是 rounded-full 不是方角 badge。
    const statusRow = within(providerRow).getByTestId(
      "provider-capability-status-row-claude_code",
    );
    expect(statusRow.className).toContain("flex-wrap");
    expect(statusRow.className).toContain("gap-x-2");
    const statusChip = within(providerRow).getByTestId(
      "provider-capability-status-claude_code",
    );
    expect(statusChip).toHaveClass("rounded-full");
    expect(statusChip).toHaveClass("bg-[var(--aria-success-soft)]");
    expect(statusChip).toHaveClass("text-[var(--aria-success)]");

    // 行3：左 = 三个能力维度胶囊（rounded-full），右 = 操作按钮（右对齐）。
    const actionsRow = within(providerRow).getByTestId(
      "provider-capability-actions-row-claude_code",
    );
    // 行3 用 gap 排布；空间不够时操作区整体换行，不靠自然换行拼行。
    expect(actionsRow.className).toContain("flex-wrap");
    expect(actionsRow.className).toContain("gap-y-2");
    const aspects = within(providerRow).getByTestId(
      "provider-capability-aspects-claude_code",
    );
    const operations = within(providerRow).getByTestId(
      "provider-capability-operations-claude_code",
    );
    // 维度在左、操作在右（DOM 顺序 + 右对齐容器 + 推到行尾）。
    expect(
      aspects.compareDocumentPosition(operations) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(operations.className).toContain("ml-auto");
    expect(operations.className).toContain("justify-end");
    for (const aspect of ["launch", "write_boundary", "resume"]) {
      const chip = within(providerRow).getByTestId(
        `provider-capability-aspect-chip-claude_code-${aspect}`,
      );
      expect(chip).toHaveClass("rounded-full");
      expect(chip.className).not.toContain("rounded-md");
    }
    // 操作按钮是次按钮（MASTER §5：淡蓝紫底 + 蓝紫字 + rounded-xl）。
    expect(
      within(providerRow).getByTestId(
        "provider-capability-revalidate-claude_code",
      ),
    ).toHaveClass("btn-secondary");

    // 未退化成一段连续文字流：块内至少三个独立行容器（名/版本、状态、操作）。
    expect(providerRow.children.length).toBeGreaterThanOrEqual(3);
    expect(
      within(providerRow).getByTestId("provider-capability-head-claude_code"),
    ).not.toBe(aspects);
    expect(aspects).not.toBe(operations);
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

    renderProviderManagement();

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

  it("shows the off-path badge for an installed CLI missing from the login shell PATH (REQ-PCI-08)", async () => {
    installFetchRouter(() => confirmedCodexProjection(), undefined, {
      targets: [
        {
          target: "claude_code",
          display_name: "Claude Code",
          cli_available: true,
          local_version: "1.0.0",
          latest_version: "1.0.0",
          latest_checked_at: "2026-11-03T00:00:00Z",
          install_source: "npm @anthropic-ai/claude-code",
          action: "none",
          off_path: false,
        },
        {
          target: "codex",
          display_name: "Codex",
          cli_available: true,
          local_version: "1.0.0",
          latest_version: "1.0.0",
          latest_checked_at: "2026-11-03T00:00:00Z",
          install_source: "npm @openai/codex",
          action: "none",
          off_path: true,
        },
      ],
    });

    renderProviderManagement();

    // 已安装但不在登录 shell PATH ⇒ 行内提示徽标（行动入口走既有
    // 「暴露到终端 PATH」按钮，不重复实现）。
    const codexRow = await screen.findByTestId(
      "provider-capability-provider-codex",
    );
    expect(
      within(codexRow).getByTestId("provider-capability-off-path-codex"),
    ).toHaveTextContent("已安装，但不在你的终端 PATH 上");
    // 在登录 shell PATH 上（off_path=false）⇒ 无徽标。
    const claudeRow = screen.getByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(
      within(claudeRow).queryByTestId(
        "provider-capability-off-path-claude_code",
      ),
    ).toBeNull();
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

    renderProviderManagement();

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
          (request) => request.method === "POST" && request.url === REVALIDATE_URL,
        ),
      ).toEqual({
        url: REVALIDATE_URL,
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

  // ↓↓↓ 以下 4 组断言迁自 ProviderCapabilityBatch.test.tsx（原
  // `ProviderCapabilityBatchResults` 逐项结果组件的 6 条回归断言中与逐项结果
  // 相关的 4 组）：owner 2026-11-04 组件删除 ⇒ 断言对象换成**行级列表**的对应
  // 行（`provider-capability-provider-<type>`），断言内容不弱化。

  it("keeps the per-item batch status in the row list (逐项状态随轮询更新)", async () => {
    window.localStorage.setItem(BATCH_STORAGE_KEY, BATCH_JOB_ID);
    installFetchRouter(
      unknownProjection,
      undefined,
      { targets: [] },
      runningBatchJob(),
    );

    renderProviderManagement();

    // 逐项状态并入对应行（已确认 / 核验中）。
    const claudeRow = await screen.findByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(
      within(claudeRow).getByTestId("capability-batch-status-claude_code"),
    ).toHaveTextContent("已确认");
    const codexRow = screen.getByTestId("provider-capability-provider-codex");
    expect(
      within(codexRow).getByTestId("capability-batch-status-codex"),
    ).toHaveTextContent("核验中");
  });

  it("keeps the retry entry for a failed batch item and does not re-run the whole batch", async () => {
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

    renderProviderManagement();

    const codexRow = await screen.findByTestId(
      "provider-capability-provider-codex",
    );
    expect(
      within(codexRow).getByTestId("capability-batch-status-codex"),
    ).toHaveTextContent("核验失败");
    // 只有失败项保留【重试】：已确认的行没有。
    const claudeRow = screen.getByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(
      within(claudeRow).queryByTestId("capability-batch-retry-claude_code"),
    ).toBeNull();

    await userEvent.click(screen.getByTestId("capability-batch-retry-codex"));
    await waitFor(() => {
      expect(
        captured.filter(
          (request) =>
            request.method === "POST" && request.url === REVALIDATE_URL,
        ),
      ).toHaveLength(1);
    });
    const singleRetries = captured.filter(
      (request) =>
        request.method === "POST" && request.url === REVALIDATE_URL,
    );
    expect(singleRetries[0].body).toEqual({ provider_type: "codex" });
    // 不重跑整批。
    expect(
      captured.filter(
        (request) =>
          request.method === "POST" && request.url === BATCH_URL,
      ),
    ).toHaveLength(0);
  });

  it("keeps the install entry for a skipped batch item (not a probe failure)", async () => {
    window.localStorage.setItem(BATCH_STORAGE_KEY, BATCH_JOB_ID);
    installFetchRouter(
      unknownProjection,
      undefined,
      {
        targets: [
          {
            target: "pi",
            display_name: "Pi",
            cli_available: false,
            local_version: null,
            latest_version: "1.2.3",
            latest_checked_at: "2026-11-03T00:00:00Z",
            install_source: "npm",
            action: "install",
          },
        ],
      },
      batchJob(),
    );

    renderProviderManagement();

    const piRow = await screen.findByTestId("provider-capability-provider-pi");
    expect(
      within(piRow).getByTestId("capability-batch-status-pi"),
    ).toHaveTextContent("跳过");
    expect(within(piRow).queryByText(/核验失败/)).toBeNull();
    expect(
      within(piRow).getByRole("button", { name: "安装" }),
    ).toBeInTheDocument();
    // 安装行必须同时给出可抄的手动命令（块2 组件按 cliAvailable=false 渲染，
    // 行级列表不隐藏、不改写）。
    expect(
      within(piRow).getByTestId("provider-install-manual-command-pi"),
    ).toHaveTextContent("npm install -g");
    expect(
      within(piRow).queryByRole("button", { name: "重试" }),
    ).not.toBeInTheDocument();
  });

  it("keeps the version-gap entry (版本落后 / 更新 / 当前→最新, not 安装) in the row list", async () => {
    window.localStorage.setItem(BATCH_STORAGE_KEY, BATCH_JOB_ID);
    installFetchRouter(
      unknownProjection,
      undefined,
      {
        targets: [
          {
            target: "codex",
            display_name: "Codex",
            cli_available: true,
            local_version: "1.0.0",
            latest_version: "2.0.0",
            latest_checked_at: "2026-11-03T00:00:00Z",
            install_source: "npm @openai/codex",
            action: "update",
          },
        ],
      },
      batchJob("confirmed"),
    );

    renderProviderManagement();

    const codexRow = await screen.findByTestId(
      "provider-capability-provider-codex",
    );
    expect(
      within(codexRow).getByTestId("provider-capability-outdated-codex"),
    ).toHaveTextContent("版本落后");
    expect(
      within(codexRow).getByRole("button", { name: "更新" }),
    ).toBeInTheDocument();
    const versions = within(codexRow).getByTestId(
      "provider-install-versions-codex",
    );
    expect(versions).toHaveTextContent("1.0.0");
    expect(versions).toHaveTextContent("2.0.0");
    expect(
      within(codexRow).queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
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

    renderProviderManagement();

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

    renderProviderManagement();
    await screen.findByTestId("provider-capability-card");

    await userEvent.click(
      screen.getByTestId("provider-capability-revalidate-codex"),
    );

    await waitFor(() => {
      expect(
        captured.find((request) => request.method === "POST"),
      ).toEqual({
        url: REVALIDATE_URL,
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
          request.url === CAPABILITIES_URL &&
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

    renderProviderManagement();
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

    renderProviderManagement();
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

    renderProviderManagement();
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

    renderProviderManagement();

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

    renderProviderManagement();

    const updateButton = await screen.findByTestId(
      "provider-install-start-codex",
    );
    expect(updateButton).toHaveTextContent("更新");
    expect(
      screen.getByTestId("provider-install-versions-codex"),
    ).toHaveTextContent("1.0.1");
  });

  // 2026-11-09 bug 修复：操作区里的「核验」与「更新」两个按钮错位。
  // 根因：操作区用 items-center 垂直居中，而安装块是多行（按钮行 +
  // 状态文案行）⇒ 「核验」按钮被整块高度居中，顶边与「更新」按钮不齐。
  // 断言：操作区顶端对齐（items-start），安装块可收缩（min-w-0）且不
  // 撑出卡片（max-w-full）——长状态文案折行而不是把整行拉宽。
  it("keeps the operation buttons on one top edge and keeps the install block inside the row", async () => {
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

    renderProviderManagement();

    const operations = await screen.findByTestId(
      "provider-capability-operations-codex",
    );
    expect(operations.className).toContain("items-start");
    expect(operations.className).not.toContain("items-center");

    const installAction = within(operations).getByTestId(
      "provider-install-action-codex",
    );
    expect(installAction.className).toContain("min-w-0");
    expect(installAction.className).toContain("max-w-full");
  });

  // 2026-11-09 bug 修复：安装/更新成功后，能力投影里的版本是**旧**的
  // （只有点「核验」才刷新），而安装目标面（provider health）是新的 ⇒
  // 行首版本必须显示新版本，否则用户看到「更新完了版本号却没变」。
  it("shows the fresh local version in the row header after an install/update", async () => {
    installFetchRouter(() => confirmedCodexProjection(), undefined, {
      targets: [
        {
          target: "codex",
          display_name: "Codex",
          cli_available: true,
          // 本地已是新版本（刚更新完），能力投影仍是旧的 0.44.0。
          local_version: "0.167.0",
          latest_version: "0.167.0",
          latest_checked_at: "2026-11-09T00:00:00Z",
          install_source: "npm @openai/codex",
          action: "none",
        },
      ],
    });

    renderProviderManagement();

    expect(
      await screen.findByTestId("provider-capability-codex-version").then(
        (element) => element.textContent,
      ),
    ).toBe("0.167.0");
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

    renderProviderManagement();

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

    renderProviderManagement();

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
    // 按钮区域说明作用域与成本（REQ-PCR-06 机器级口径：作用域 + 三项成本
    // 告知逐项保留，仍是子串匹配，不弱化）。
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "机器级核验",
    );
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "核验只核验一次",
    );
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "机器一份事实",
    );
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "会真实跑一次探测",
    );
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "可能几分钟",
    );
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "消耗 provider 额度",
    );
  });

  // F3（终审 2026-11-13）：已安装但**未核验**的行，既有父级 gate
  // （`!cli_available || confirmed`）不渲染安装块 ⇒「暴露到终端 PATH」入口
  // 不可达：用户刚装完（已安装、尚未做能力核验）时看不到暴露入口，功能不可用。
  it("keeps the expose-to-terminal-PATH entry reachable for an installed but unverified provider", async () => {
    stubCatalogFetch([
      installTargetFixture({
        target: "codex",
        install_source: "npm @openai/codex",
        off_path: true,
      }),
    ]);

    render(<ProviderCapabilityCard />);

    const codexRow = await screen.findByTestId(
      "provider-capability-provider-codex",
    );
    // 未核验（unknown 投影）⇒ 既有 gate 不渲染安装块，但暴露入口必须可达。
    const exposeButton = within(codexRow).getByTestId(
      "provider-capability-expose-codex",
    );
    expect(exposeButton).toBeEnabled();

    await userEvent.click(exposeButton);

    await waitFor(() => {
      expect(
        captured.find(
          (request) =>
            request.method === "POST" &&
            request.url === "/api/providers/codex/expose",
        ),
      ).toBeTruthy();
    });
    expect(
      within(codexRow).getByTestId("provider-capability-expose-result-codex"),
    ).toHaveTextContent("已暴露到 /home/demo/.local/bin");
  });

  // F3 配套：暴露入口只对「已安装且不在终端 PATH 上」的行出现 —— 已在 PATH 上
  // （off_path=false，用户终端本来就能跑）或压根没安装时都不出现（不加噪音）。
  it("offers the expose entry only for an installed CLI that is missing from the terminal PATH", async () => {
    stubCatalogFetch([
      installTargetFixture({
        target: "claude_code",
        install_source: "npm @anthropic-ai/claude-code",
      }),
      installTargetFixture({
        target: "pi",
        cli_available: false,
        local_version: null,
        latest_version: "1.2.3",
        action: "install",
      }),
    ]);

    render(<ProviderCapabilityCard />);

    // 已在终端 PATH 上（off_path=false）⇒ 本行**没有任何**暴露入口（卡片级
    // 入口不出现，共享安装块里那个也不出现——口径统一由卡片按 off_path 决定）。
    const claudeRow = await screen.findByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(
      within(claudeRow).queryByTestId("provider-capability-expose-claude_code"),
    ).toBeNull();
    expect(
      within(claudeRow).queryByTestId("provider-install-expose-claude_code"),
    ).toBeNull();
    // 未安装 ⇒ 无暴露入口（只给【安装】+ 手动命令）。
    const piRow = await screen.findByTestId("provider-capability-provider-pi");
    expect(
      within(piRow).queryByTestId("provider-capability-expose-pi"),
    ).toBeNull();
    expect(within(piRow).queryByTestId("provider-install-expose-pi")).toBeNull();
  });

  // F3 收口（终审授权后）：能力卡里的暴露入口**只有卡片自己渲染的那一个**，
  // 口径统一为「已安装 + off_path=true」（无论是否已核验）——共享安装块
  // （`ProviderInstallActionButton`）不再重复渲染第二个同义 expose 按钮。
  it("renders exactly one expose entry for a verified provider whose CLI is missing from the terminal PATH", async () => {
    installFetchRouter(() => confirmedCodexProjection(), undefined, {
      targets: [
        installTargetFixture({
          target: "codex",
          install_source: "npm @openai/codex",
          off_path: true,
        }),
      ],
    });

    render(<ProviderCapabilityCard />);

    const codexRow = await screen.findByTestId(
      "provider-capability-provider-codex",
    );
    // 已核验 + 已安装 + off_path=true ⇒ 卡片行内入口出现。
    expect(
      within(codexRow).getByTestId("provider-capability-expose-codex"),
    ).toBeInTheDocument();
    // 共享安装块不再渲染第二个同义按钮（同一动作只留一个入口）。
    expect(
      within(codexRow).queryByTestId("provider-install-expose-codex"),
    ).toBeNull();
  });
});
