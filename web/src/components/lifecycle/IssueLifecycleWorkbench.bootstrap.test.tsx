// 冷启动投影刷新链回归（2026-10-10 修复）：
//
// - 合并流（添加代码库 → 创建 LC → 内嵌登记向导 → 登记完成）后，bootstrap
//   投影与成员必须重拉——后端在登记批内推进 identity/manifest_checkout，
//   前端不得停留在登记前的 all-not_started 投影（线上症状：管理抽屉冷启动
//   区五步全「未开始」，而 GET bootstrap 实测 identity/manifest_checkout
//   已 completed）。
// - 管理抽屉每次展开（面板挂载）补读 bootstrap 投影，冷启动区与后端一致。
// - LC 切换时投影随选中 LC 重拉（既有行为回归锁定）。
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";
import type {
  BootstrapStepProjectionDto,
  LogicalCodebaseBootstrapProjection,
  LogicalCodebaseBootstrapStepName,
  LogicalCodebaseBootstrapStepStatus,
} from "../../api/types";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

const BOOTSTRAP_URL =
  "/api/projects/project_0001/logical-codebases/lc_0001/bootstrap";
const MEMBERS_URL =
  "/api/projects/project_0001/logical-codebases/lc_0001/members";

const step = (
  name: LogicalCodebaseBootstrapStepName,
  status: LogicalCodebaseBootstrapStepStatus,
): BootstrapStepProjectionDto => ({
  step: name,
  status,
  object_id: `${name}-object`,
  checkpoint: null,
  failure: null,
  allowed_actions: [],
});

function bootstrapProjection(
  steps: BootstrapStepProjectionDto[],
  overrides: Partial<LogicalCodebaseBootstrapProjection> = {},
): LogicalCodebaseBootstrapProjection {
  return {
    project_id: "project_0001",
    logical_codebase_id: "lc_0001",
    authority_root: "/root",
    membership_revision: 5,
    policy: null,
    steps,
    planning_ready: false,
    notices: [],
    ...overrides,
  };
}

/** 登记提交前：五步全部 not_started。 */
function allNotStarted(): LogicalCodebaseBootstrapProjection {
  return bootstrapProjection(
    (
      [
        "identity",
        "manifest_checkout",
        "rules_policy",
        "member_index",
        "aggregate_index_active",
      ] as const
    ).map((name) => step(name, "not_started")),
  );
}

/** 登记完成后的后端实测投影：identity/manifest_checkout completed，其余等待显式 prepare。 */
function afterRegistration(
  overrides: Partial<LogicalCodebaseBootstrapProjection> = {},
): LogicalCodebaseBootstrapProjection {
  return bootstrapProjection(
    [
      step("identity", "completed"),
      step("manifest_checkout", "completed"),
      step("rules_policy", "not_started"),
      step("member_index", "not_started"),
      step("aggregate_index_active", "not_started"),
    ],
    overrides,
  );
}

describe("IssueLifecycleWorkbench 冷启动投影刷新链", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => vi.unstubAllGlobals());

  it("合并流登记完成后重拉 bootstrap 投影与成员，抽屉展示后端实际状态", async () => {
    const calls: string[] = [];
    // 登记批提交成功后，成员列表才从 0 变 1（模拟后端在登记批内落成员）。
    let registrationCompleted = false;
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [{ id: "lc_0001", name: "monorepo", member_count: 0 }],
      logicalCodebaseMembers: [],
      registrationPreflight: {
        preflight_id: "preflight_0001",
        created_at: "2026-08-19T00:00:00Z",
        items: [{ path: "/root/api", class: "eligible", reason: null }],
      },
      registrationSubmit: {
        batch_id: "batch_0001",
        status: "completed",
        items: [
          { path: "/root/api", status: "completed", failure_reason: null },
        ],
      },
      // 挂载 GET #1 返回登记前的 all-not_started；其后返回登记后实测投影。
      bootstrapProjections: [allNotStarted(), afterRegistration()],
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push(`${init?.method ?? "GET"} ${url}`);
        if (
          url === "/api/projects/project_0001/logical-codebases" &&
          init?.method === "POST"
        ) {
          return new Response(
            JSON.stringify({
              id: "lc_0001",
              name: "monorepo",
              aggregate_root: "/root",
              created_at: "2026-08-19T00:00:00Z",
            }),
          );
        }
        if (
          url === "/api/projects/project_0001/logical-codebases/lc_0001/registrations" &&
          init?.method === "POST"
        ) {
          const response = await fetchMock(input, init);
          registrationCompleted = true;
          return response;
        }
        if (url === MEMBERS_URL && registrationCompleted) {
          return new Response(
            JSON.stringify({
              members: [
                {
                  logical_repository_id: "repository_0001",
                  physical_repository_id: "repository_0001",
                  alias: "api",
                  status: "active",
                },
              ],
            }),
          );
        }
        return fetchMock(input, init);
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    // 合并流：添加代码库 → 建逻辑代码库 → 弹窗内嵌登记向导。
    await user.click(await screen.findByRole("button", { name: "添加代码库" }));
    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));
    expect(
      await screen.findByLabelText("选择 /root/api（eligible）"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "提交登记" }));

    await waitFor(() =>
      expect(
        calls.filter(
          (call) =>
            call ===
            "POST /api/projects/project_0001/logical-codebases/lc_0001/registrations",
        ).length,
      ).toBeGreaterThan(0),
    );

    // 登记完成后：bootstrap 投影与成员立即重拉（不等抽屉展开/轮询）。
    await waitFor(() =>
      expect(
        calls.filter((call) => call === `GET ${BOOTSTRAP_URL}`).length,
      ).toBeGreaterThanOrEqual(2),
    );
    await waitFor(() =>
      expect(
        calls.filter((call) => call === `GET ${MEMBERS_URL}`).length,
      ).toBeGreaterThanOrEqual(2),
    );

    // 关闭弹窗、展开管理抽屉（用户实测路径）：冷启动区与后端投影一致。
    const dialog = screen.getByRole("dialog", { name: "添加代码库" });
    await user.click(within(dialog).getByRole("button", { name: "关闭" }));
    await user.click(screen.getByTestId("lc-summary-toggle"));

    await waitFor(() =>
      expect(screen.getByTestId("lc-bootstrap-status-identity")).toHaveTextContent(
        "已完成",
      ),
    );
    expect(
      screen.getByTestId("lc-bootstrap-status-manifest_checkout"),
    ).toHaveTextContent("已完成");
    expect(
      screen.getByTestId("lc-bootstrap-status-rules_policy"),
    ).toHaveTextContent("未开始");
    expect(
      screen.getByTestId("lc-bootstrap-planning-ready"),
    ).toHaveTextContent("冷启动未完成");
    // 成员已重拉：聚合初始化卡可见（此前因成员陈旧为空而被隐藏）。
    expect(
      await screen.findByTestId("aggregate-initialization-card"),
    ).toBeInTheDocument();
  });

  it("展开管理抽屉时补读 bootstrap 投影，冷启动区与后端一致", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [{ id: "lc_0001", name: "monorepo" }],
        // 挂载 GET #1 返回陈旧 all-not_started；抽屉展开的补读 GET #2 返回
        // 后端当前投影——抽屉不得沿用挂载时的陈旧快照。
        bootstrapProjections: [allNotStarted(), afterRegistration()],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("lc-summary-toggle"));

    await waitFor(() =>
      expect(screen.getByTestId("lc-bootstrap-status-identity")).toHaveTextContent(
        "已完成",
      ),
    );
    expect(
      screen.getByTestId("lc-bootstrap-status-manifest_checkout"),
    ).toHaveTextContent("已完成");
    expect(
      screen.getByTestId("lc-bootstrap-status-rules_policy"),
    ).toHaveTextContent("未开始");
    expect(
      screen.getByTestId("lc-bootstrap-planning-ready"),
    ).toHaveTextContent("冷启动未完成");
  });

  it("切换 LC 时冷启动区随选中 LC 重拉投影", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [
          { id: "lc_0001", name: "monorepo" },
          { id: "lc_0002", name: "services" },
        ],
        // 依调用次序：挂载（lc_0001 all-not_started）→ 抽屉展开补读（仍
        // lc_0001）→ 切换 lc_0002（identity/manifest_checkout completed）。
        bootstrapProjections: [
          allNotStarted(),
          allNotStarted(),
          afterRegistration({ logical_codebase_id: "lc_0002" }),
        ],
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await waitFor(() =>
      expect(screen.getByTestId("lc-bootstrap-status-identity")).toHaveTextContent(
        "未开始",
      ),
    );

    await user.click(screen.getByTestId("lc-selector-services"));
    await waitFor(() =>
      expect(screen.getByTestId("lc-bootstrap-status-identity")).toHaveTextContent(
        "已完成",
      ),
    );
    expect(
      screen.getByTestId("lc-bootstrap-status-manifest_checkout"),
    ).toHaveTextContent("已完成");
  });
});
