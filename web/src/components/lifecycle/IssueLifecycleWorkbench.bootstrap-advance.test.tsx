// LC 冷启动自动推进（前端侧）回归（2026-10-29）：
//
// - 登记完成后前端自动调一次 POST .../bootstrap/advance（按依赖顺序推进
//   剩余步骤），恰好一次——登记重放/恢复不再重复触发。
// - 自动推进失败只如实提示，不重试（幂等端点，失败面由用户手动
//   「一键推进」补）。
import { render, screen, waitFor } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
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

vi.mock("../shared/MonacoViewer", () => ({ default: () => null }));

const BOOTSTRAP_URL =
  "/api/projects/project_0001/logical-codebases/lc_0001/bootstrap";
const ADVANCE_URL = `${BOOTSTRAP_URL}/advance`;

const step = (
  name: LogicalCodebaseBootstrapStepName,
  status: LogicalCodebaseBootstrapStepStatus,
  allowed: BootstrapStepProjectionDto["allowed_actions"] = [],
): BootstrapStepProjectionDto => ({
  step: name,
  status,
  object_id: `${name}-object`,
  checkpoint: null,
  failure: null,
  allowed_actions: allowed,
});

/** 登记完成后的实测投影：后三步未开始且各自允许 prepare（可自动推进）。 */
function afterRegistration(): LogicalCodebaseBootstrapProjection {
  return {
    project_id: "project_0001",
    logical_codebase_id: "lc_0001",
    authority_root: "/root",
    membership_revision: 1,
    policy: null,
    steps: [
      step("identity", "completed"),
      step("manifest_checkout", "completed"),
      step("rules_policy", "not_started", ["prepare"]),
      step("member_index", "not_started", ["prepare"]),
      step("aggregate_index_active", "not_started", ["prepare"]),
    ],
    planning_ready: false,
    notices: [],
  };
}

function installWorkbenchFetch(advanceStatus: number) {
  const advanceCalls: string[] = [];
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
      items: [{ path: "/root/api", status: "completed", failure_reason: null }],
    },
    // 每次补读都返回「可推进」投影（自动推进前的 GET 判定据此放行）。
    bootstrapProjections: [afterRegistration()],
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
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
      if (url === ADVANCE_URL && init?.method === "POST") {
        advanceCalls.push(url);
        if (advanceStatus !== 200) {
          return new Response(
            JSON.stringify({
              code: "bootstrap_advance_failed",
              message: "推进失败",
            }),
            { status: advanceStatus },
          );
        }
        return new Response(
          JSON.stringify({
            project_id: "project_0001",
            logical_codebase_id: "lc_0001",
            advanced_steps: ["rules_policy", "member_index"],
            stopped_reason: "planning_ready",
            projection: afterRegistration(),
          }),
        );
      }
      return fetchMock(input, init);
    }),
  );
  return advanceCalls;
}

async function completeRegistration(user: UserEvent) {
  await user.click(await screen.findByRole("button", { name: "添加代码库" }));
  await user.click(screen.getByRole("radio", { name: /多仓库逻辑代码库/ }));
  await user.type(screen.getByLabelText("聚合根目录"), "/root");
  await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));
  expect(
    await screen.findByLabelText("选择 /root/api（eligible）"),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "提交登记" }));
}

describe("IssueLifecycleWorkbench 冷启动自动推进", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => vi.unstubAllGlobals());

  it("登记完成后自动调一次 advance 端点", async () => {
    const advanceCalls = installWorkbenchFetch(200);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await completeRegistration(user);

    await waitFor(() => expect(advanceCalls).toHaveLength(1));
    // 恰好一次：登记完成不重复触发（轮询/补读不得再调）。
    // lib 目标为 ES2022（无 Promise.withResolvers）→ 用 executor 形式延时。
    await new Promise<void>((resolve) => {
      setTimeout(resolve, 50);
    });
    expect(advanceCalls).toHaveLength(1);
  });

  it("自动推进失败只提示不重试", async () => {
    const advanceCalls = installWorkbenchFetch(500);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await completeRegistration(user);

    await waitFor(() => expect(advanceCalls).toHaveLength(1));
    expect(await screen.findByText(/冷启动自动推进失败/)).toBeInTheDocument();
    // 失败不重试：轮询/补读不得再调 advance。
    // lib 目标为 ES2022（无 Promise.withResolvers）→ 用 executor 形式延时。
    await new Promise<void>((resolve) => {
      setTimeout(resolve, 50);
    });
    expect(advanceCalls).toHaveLength(1);
  });
});