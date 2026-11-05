// Provider 管理面板（独立于「逻辑代码库」管理面板）单测。
// 2026-11-04：owner 裁定 provider 的探针/核验/更新/安装/版本属全局能力，
// 不该寄居在 LC 仓抽屉里 ⇒ 侧栏「Provider 管理」入口 → 独立面板。
// 2026-11-05 机器级化（REQ-PCR-08）：能力面不再按项目/逻辑代码库定位 ⇒
// 面板不接收任何 id。
// 本文件断言：
// - Provider 管理面板【渲染】provider 能力核验区（行级列表 + 四家 provider）；
// - 「一键核验全部」按钮挂在能力卡头部（四家 provider 行之前）且说明区是
//   **机器级核验**口径（作用域 + 三项成本告知）——这组断言自
//   LogicalCodebaseManagementPanel.test.tsx 迁来（宿主换成 Provider 管理面板，
//   断言内容不弱化）；
// - 抽屉外壳：标题「Provider 管理」、可关闭（× / Esc）、与 LC 管理抽屉互不干扰。
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProviderManagementDrawer } from "./ProviderManagementDrawer";
import { ProviderManagementPanel } from "./ProviderManagementPanel";
import type {
  ProviderCapabilitiesResponse,
  ProviderCapabilityType,
} from "../../api/types";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** 四家 provider 的只读投影（面板挂载即 GET capabilities）。 */
function projection(): ProviderCapabilitiesResponse {
  const provider = (providerType: string) => ({
    provider_type: providerType as ProviderCapabilityType,
    cli_program: providerType,
    version: "1.0.0",
    probed_at: "2026-11-04T00:00:00Z",
    probe_artifact_ref: `artifact://probe/${providerType}`,
    rows: [
      {
        action: "coding_target_write" as const,
        launch: "confirmed" as const,
        resume: "confirmed" as const,
        write_boundary: "confirmed" as const,
        evidence_ref: `evidence/${providerType}`,
      },
    ],
  });
  return {
    providers: [
      provider("claude_code"),
      provider("codex"),
      provider("pi"),
      provider("kimi_code"),
    ],
  };
}

function stubFetch(): void {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString();
      if (url.endsWith("/capabilities")) {
        return jsonResponse(projection());
      }
      if (url === "/api/providers/install-targets") {
        return jsonResponse({ targets: [] });
      }
      return jsonResponse(
        { code: "not_found", message: `no route for ${url}` },
        404,
      );
    }),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  window.localStorage.clear();
});

describe("Provider 管理面板", () => {
  it("面板渲染 provider 能力核验区（四家 provider 的行级列表）", async () => {
    stubFetch();

    render(
      <ProviderManagementDrawer open onClose={() => {}}>
        <ProviderManagementPanel />
      </ProviderManagementDrawer>,
    );

    // 能力核验区在 Provider 管理面板里（不在 LC 面板里）。
    expect(await screen.findByTestId("provider-capability-card")).toBeInTheDocument();
    expect(screen.getByTestId("provider-management-panel")).toBeInTheDocument();
    expect(screen.getByTestId("provider-capability-list")).toBeInTheDocument();
    for (const providerType of ["claude_code", "codex", "pi", "kimi_code"]) {
      expect(
        screen.getByTestId(`provider-capability-provider-${providerType}`),
      ).toBeInTheDocument();
    }
  });

  // 自 LogicalCodebaseManagementPanel.test.tsx 迁来（宿主换成 Provider 管理
  // 面板）：「一键核验全部」按钮挂在能力卡头部（上方）、在四家 provider 行之前，
  // 说明区是机器级核验口径（作用域 + 三项成本告知）。断言内容不弱化。
  it("「一键核验全部」按钮在能力卡头部（上方），说明区是机器级核验口径", async () => {
    stubFetch();

    render(
      <ProviderManagementDrawer open onClose={() => {}}>
        <ProviderManagementPanel />
      </ProviderManagementDrawer>,
    );

    // 按钮挂在能力卡里（不是面板底部另起一条）。
    const card = await screen.findByTestId("provider-capability-card");
    const startButton = screen.getByRole("button", { name: "一键核验全部" });
    expect(card).toContainElement(startButton);
    expect(
      screen.queryByTestId("provider-capability-batch-bar"),
    ).not.toBeInTheDocument();

    // 按钮在卡片内位于四家 provider 行之前（即「上面」）。
    const firstProviderRow = await screen.findByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(
      startButton.compareDocumentPosition(firstProviderRow) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    // 按钮区域说明作用域与成本（REQ-PCR-06，机器级口径）。
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

  it("抽屉标题是「Provider 管理」，可关闭（× 按钮与 Esc）", async () => {
    stubFetch();
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(
      <ProviderManagementDrawer open onClose={onClose}>
        <ProviderManagementPanel />
      </ProviderManagementDrawer>,
    );

    const drawer = screen.getByRole("dialog", { name: "Provider 管理" });
    expect(drawer).toHaveAttribute("aria-modal", "true");
    expect(screen.getByTestId("provider-management-drawer")).toBe(drawer);

    await user.click(screen.getByTestId("provider-management-close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Esc 关闭 Provider 管理抽屉", async () => {
    stubFetch();
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(
      <ProviderManagementDrawer open onClose={onClose}>
        <ProviderManagementPanel />
      </ProviderManagementDrawer>,
    );

    await screen.findByTestId("provider-management-drawer");
    await user.keyboard("{Escape}");

    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });
});
