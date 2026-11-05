// 2026-11-04：Provider 管理独立入口的接线回归（工作台级）。
// owner 裁定 provider 的探针/核验/更新/安装/版本属平台公用能力，不该寄居
// 在「逻辑代码库」抽屉里 ⇒ 侧栏新增平级分区「Provider 管理」，点开是独立面板。
// 2026-11-05 机器级化（REQ-PCR-08）：能力面不按项目/逻辑代码库定位 ⇒
// 零项目时入口也必须可点、面板照常打开。
// 断言：
// - 侧栏有「Provider 管理」入口，点击后打开独立抽屉（标题「Provider 管理」），
//   面板里渲染四家 provider 的行级列表；
// - 逻辑代码库抽屉里【看不到】provider 核验区（只有一行只读说明 + 去管理入口）；
// - 逻辑代码库抽屉里的「去 Provider 管理」入口能打开独立面板。
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Project } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponse,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

/** 四家 provider 的只读投影（能力面挂载即 GET capabilities）。 */
function projection() {
  const provider = (providerType: string) => ({
    provider_type: providerType,
    cli_program: providerType,
    version: "1.0.0",
    probed_at: "2026-11-04T00:00:00Z",
    probe_artifact_ref: `artifact://probe/${providerType}`,
    rows: [
      {
        action: "coding_target_write",
        launch: "confirmed",
        resume: "confirmed",
        write_boundary: "confirmed",
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

function installFetch(
  options?: {
    projects?: Project[];
    logicalCodebases?: Array<{
      id: string;
      name: string;
      member_count: number;
    }>;
  },
) {
  const base = lifecycleFetch({
    projects: options?.projects ?? [projectRecord("project_0001", "Aria")],
    logicalCodebases:
      options?.logicalCodebases ?? [
        { id: "lc_0001", name: "platform", member_count: 1 },
      ],
  });
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/capabilities")) {
        return Promise.resolve(jsonResponse(projection()));
      }
      if (url === "/api/providers/install-targets") {
        return Promise.resolve(jsonResponse({ targets: [] }));
      }
      return base(input, init);
    }),
  );
}

describe("Provider 管理独立入口（工作台接线）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("侧栏「Provider 管理」入口可打开独立面板，面板显示四家 provider 的行级列表", async () => {
    installFetch();
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // 侧栏分区与入口（与「代码库」「已删除」平级）。
    const section = await screen.findByTestId("provider-management-section");
    expect(section).toHaveAttribute("aria-label", "Provider 管理");
    const openButton = screen.getByTestId("provider-management-open");
    expect(openButton).toHaveTextContent("Provider 管理");
    expect(openButton).toBeEnabled();

    await user.click(openButton);

    // 独立抽屉（不是 LC 管理抽屉）。
    const drawer = await screen.findByTestId("provider-management-drawer");
    expect(drawer).toHaveAttribute("aria-label", "Provider 管理");
    expect(screen.queryByTestId("lc-management-drawer")).toBeNull();

    // 面板里渲染 provider 能力核验区（四家 provider 的行级列表）。
    expect(
      await screen.findByTestId("provider-capability-card"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("provider-capability-list")).toBeInTheDocument();
    for (const providerType of ["claude_code", "codex", "pi", "kimi_code"]) {
      expect(
        screen.getByTestId(`provider-capability-provider-${providerType}`),
      ).toBeInTheDocument();
    }
  });

  it("逻辑代码库抽屉里看不到 provider 核验区（只有只读说明 + 去管理入口）", async () => {
    installFetch();
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    const lcDrawer = await screen.findByTestId("lc-management-drawer");

    // 能力核验区不再出现在 LC 抽屉里。
    expect(
      within(lcDrawer).queryByTestId("provider-capability-card"),
    ).toBeNull();
    expect(within(lcDrawer).queryByTestId("provider-capability-list")).toBeNull();
    expect(
      within(lcDrawer).queryByRole("button", { name: "一键核验全部" }),
    ).toBeNull();

    // 只留一行只读说明 + 「去 Provider 管理」入口。
    expect(
      within(lcDrawer).getByTestId("lc-provider-management-summary"),
    ).toBeInTheDocument();
    expect(
      within(lcDrawer).getByTestId("provider-management-link"),
    ).toHaveTextContent("去 Provider 管理");
  });

  it("LC 抽屉里的「去 Provider 管理」入口能打开独立面板", async () => {
    installFetch();
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    const lcDrawer = await screen.findByTestId("lc-management-drawer");
    await user.click(within(lcDrawer).getByTestId("provider-management-link"));

    expect(
      await screen.findByTestId("provider-management-drawer"),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("provider-capability-card"),
    ).toBeInTheDocument();
    // 两个管理浮层互斥：Provider 管理打开后 LC 管理抽屉收起。
    await waitFor(() =>
      expect(screen.queryByTestId("lc-management-drawer")).toBeNull(),
    );
  });

  // owner 核心诉求（2026-11-05 机器级化，REQ-PCR-08）：provider 是平台公用
  // 能力 ⇒ 即使一个项目都没有，「Provider 管理」入口也必须可点、面板照常
  // 打开（机器一份事实，与项目、逻辑代码库无关）。
  it("零项目时「Provider 管理」入口仍可点，面板照常打开", async () => {
    installFetch({ projects: [], logicalCodebases: [] });
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    const openButton = await screen.findByTestId("provider-management-open");
    expect(openButton).toBeEnabled();

    await user.click(openButton);

    expect(
      await screen.findByTestId("provider-management-drawer"),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("provider-capability-card"),
    ).toBeInTheDocument();
    for (const providerType of ["claude_code", "codex", "pi", "kimi_code"]) {
      expect(
        screen.getByTestId(`provider-capability-provider-${providerType}`),
      ).toBeInTheDocument();
    }
  });
});
