// LC 冷启动「步骤动作可见性」回归（2026-10-29）：
//
// - 步骤自身的 allowed_actions 直接渲染为可点按钮：此前只有 durable notice
//   带动作时才可见，后三步 not_started（actions=['prepare']）时用户
//   「看不见也点不着」——登记完成后卡在冷启动未完成且无任何入口。
// - 同一步骤既有 notice 又有同名 allowed_action 时只渲染一个按钮（去重）。
// - 点击步骤按钮经统一 action REST 发送（step/action/expected_object_id
//   逐条断言，command_id 由步骤稳定 object_id 派生 → 重放幂等）。
// - 「一键推进」POST .../bootstrap/advance（幂等端点），成功后补读同一 GET。
// - 规则漂移（policy/rule/reference digest 漂移）：卡片给一条显眼提示
//   「规则已变更，需重新生成政策」+ republish 按钮（provider 启动会被
//   PolicyDrift 拒）；三种原因码各有说明文案；同一动作不重复渲染按钮。
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseBootstrapCard } from "./LogicalCodebaseBootstrapCard";
import type {
  BootstrapStepProjectionDto,
  LogicalCodebaseBootstrapNoticeDto,
  LogicalCodebaseBootstrapProjection,
  LogicalCodebaseBootstrapStepName,
  LogicalCodebaseBootstrapStepStatus,
} from "../../api/types";

const PROJECT_ID = "project_0001";
const LC_ID = "logical_codebase_0001";
const ACTIONS_URL = `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/bootstrap/actions`;
const ADVANCE_URL = `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/bootstrap/advance`;

type CapturedRequest = { url: string; method: string; body: unknown };
let captured: CapturedRequest[];

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function installFetchRouter(advanceStatus = 200, advanceBody?: unknown): void {
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
    if (method === "GET" && url === "/api/providers/install-targets") {
      return jsonResponse({
        targets: [
          {
            target: "codegraph",
            display_name: "codegraph",
            cli_available: false,
            local_version: null,
            latest_version: "1.7.0",
            latest_checked_at: "2026-11-04T00:00:00Z",
            install_source: "npm @colbymchenry/codegraph",
            action: "install",
          },
        ],
      });
    }
    if (method === "POST" && url === ACTIONS_URL) {
      return jsonResponse({
        command_id: "cmd",
        outcome: "accepted",
        projection: projection(),
      });
    }
    if (method === "POST" && url === ADVANCE_URL) {
      if (advanceStatus !== 200) {
        return jsonResponse(
          { code: "bootstrap_advance_failed", message: "推进失败" },
          advanceStatus,
        );
      }
      return jsonResponse(
        advanceBody ?? {
          project_id: PROJECT_ID,
          logical_codebase_id: LC_ID,
          advanced_steps: ["rules_policy", "member_index"],
          stopped_reason: "planning_ready",
          projection: projection(),
        },
      );
    }
    return jsonResponse(
      { code: "not_found", message: `no route for ${method} ${url}` },
      404,
    );
  };
  vi.stubGlobal("fetch", vi.fn(router));
}

const step = (
  name: LogicalCodebaseBootstrapStepName,
  status: LogicalCodebaseBootstrapStepStatus,
  extra: Partial<BootstrapStepProjectionDto> = {},
): BootstrapStepProjectionDto => ({
  step: name,
  status,
  object_id: `${name}-object`,
  checkpoint: null,
  failure: null,
  allowed_actions: [],
  ...extra,
});

function notice(
  extra: Partial<LogicalCodebaseBootstrapNoticeDto> = {},
): LogicalCodebaseBootstrapNoticeDto {
  return {
    key: "bootstrap:rules_policy:rules_policy-object:rules_missing",
    step: "rules_policy",
    object_id: "rules_policy-object",
    reason_code: "rules_missing",
    summary: "成员缺少 .claude/rules",
    external_side_effect: "none",
    allowed_actions: ["prepare"],
    next_step: "member_index",
    created_at: "",
    ...extra,
  };
}

function projection(
  overrides: Partial<LogicalCodebaseBootstrapProjection> = {},
): LogicalCodebaseBootstrapProjection {
  return {
    project_id: PROJECT_ID,
    logical_codebase_id: LC_ID,
    authority_root: "/workspace/aggregate-root",
    membership_revision: 3,
    policy: null,
    steps: [
      step("identity", "completed"),
      step("manifest_checkout", "completed"),
      step("rules_policy", "not_started", { allowed_actions: ["prepare"] }),
      step("member_index", "not_started", { allowed_actions: ["prepare"] }),
      step("aggregate_index_active", "not_started", {
        allowed_actions: ["prepare"],
      }),
    ],
    planning_ready: false,
    notices: [],
    ...overrides,
  };
}

/** 规则漂移：rules_policy 等待人工 + 稳定漂移原因码（allowed_actions=[republish]）。 */
function driftProjection(
  reasonCode: string,
  overrides: Partial<LogicalCodebaseBootstrapProjection> = {},
): LogicalCodebaseBootstrapProjection {
  return projection({
    steps: [
      step("identity", "completed"),
      step("manifest_checkout", "completed"),
      step("rules_policy", "waiting_for_human", {
        failure: {
          reason_code: reasonCode,
          detail: "receipt froze digest but the current digest differs",
          retryable: true,
          external_side_effect: "none",
        },
        allowed_actions: ["republish"],
      }),
      step("member_index", "not_started", { allowed_actions: ["prepare"] }),
      step("aggregate_index_active", "not_started", {
        allowed_actions: ["prepare"],
      }),
    ],
    ...overrides,
  });
}

describe("LogicalCodebaseBootstrapCard 步骤动作可见性", () => {
  beforeEach(() => {
    captured = [];
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("未开始步骤的 allowed_actions 渲染为可点按钮（此前无任何入口）", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
      />,
    );

    const prepare = screen.getByTestId(
      "lc-bootstrap-step-action-rules_policy-prepare",
    );
    expect(prepare.textContent).toBe("准备");
    expect(prepare).not.toBeDisabled();
    // 三步各自可点（后三步 not_started + actions=['prepare']）。
    expect(
      screen.getByTestId("lc-bootstrap-step-action-member_index-prepare"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId(
        "lc-bootstrap-step-action-aggregate_index_active-prepare",
      ),
    ).toBeInTheDocument();
  });

  it("同一步骤既有 notice 又有同名 allowed_action 时只渲染一个按钮", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({ notices: [notice()] })}
      />,
    );

    // notice 已渲染「准备」→ 步骤行不再重复渲染同名按钮（每步只留一个入口）。
    expect(
      within(screen.getByTestId("lc-bootstrap-notice-rules_policy")).getAllByRole(
        "button",
        { name: "准备" },
      ),
    ).toHaveLength(1);
    expect(
      within(screen.getByTestId("lc-bootstrap-step-rules_policy")).queryByRole(
        "button",
        { name: "准备" },
      ),
    ).toBeNull();
    expect(
      screen.queryByTestId("lc-bootstrap-step-action-rules_policy-prepare"),
    ).toBeNull();
    // notice 未覆盖的动作仍由步骤行暴露（不吞掉可动步骤的入口）。
    expect(
      within(screen.getByTestId("lc-bootstrap-step-member_index")).getAllByRole(
        "button",
        { name: "准备" },
      ),
    ).toHaveLength(1);
  });

  it("点击步骤按钮经统一 action REST 发送 step/action/expected 身份", async () => {
    installFetchRouter();
    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
        onChanged={onChanged}
      />,
    );

    await userEvent.click(
      screen.getByTestId("lc-bootstrap-step-action-rules_policy-prepare"),
    );

    await waitFor(() => {
      expect(captured).toEqual([
        {
          url: ACTIONS_URL,
          method: "POST",
          body: {
            // 稳定 command id：步骤 object_id + action 派生，重放幂等。
            command_id: "cmd-lc-bootstrap-rules_policy-object-prepare",
            step: "rules_policy",
            action: "prepare",
            expected_revision: 3,
            expected_object_id: "rules_policy-object",
          },
        },
      ]);
    });
    expect(onChanged).toHaveBeenCalled();
    expect(screen.queryByTestId("lc-bootstrap-action-error")).toBeNull();
  });

  it("一键推进调用 advance 端点并回显推进结果", async () => {
    installFetchRouter();
    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
        onChanged={onChanged}
      />,
    );

    await userEvent.click(screen.getByTestId("lc-bootstrap-advance"));

    await waitFor(() => {
      expect(captured).toEqual([
        { url: ADVANCE_URL, method: "POST", body: {} },
      ]);
    });
    expect(
      screen.getByTestId("lc-bootstrap-advance-summary").textContent,
    ).toContain("规则/政策");
    expect(onChanged).toHaveBeenCalled();
  });

  it("推进成功后把服务端最终投影回传给 onChanged（免一次 GET）", async () => {
    installFetchRouter();
    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
        onChanged={onChanged}
      />,
    );

    await userEvent.click(screen.getByTestId("lc-bootstrap-advance"));

    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    // 响应里的 projection 就是最终投影 → 直接采用，不再补读 GET。
    expect(onChanged.mock.calls[0]?.[0]).toMatchObject({
      logical_codebase_id: LC_ID,
      planning_ready: false,
    });
  });

  it("一键推进失败时如实展示错误且不吞成成功", async () => {
    installFetchRouter(500);
    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
        onChanged={onChanged}
      />,
    );

    await userEvent.click(screen.getByTestId("lc-bootstrap-advance"));

    await waitFor(() => {
      expect(screen.getByTestId("lc-bootstrap-action-error").textContent).toBe(
        "推进失败",
      );
    });
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("长任务在跑（in_progress）时如实说明进行中，不当成失败", async () => {
    // 幂等重放/长任务在跑：advanced_steps 为空 + stopped_reason=in_progress。
    installFetchRouter(200, {
      project_id: PROJECT_ID,
      logical_codebase_id: LC_ID,
      advanced_steps: [],
      stopped_reason: "in_progress",
      stopped_step: "rules_policy",
      projection: projection(),
    });
    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
        onChanged={onChanged}
      />,
    );

    await userEvent.click(screen.getByTestId("lc-bootstrap-advance"));

    await waitFor(() => {
      expect(
        screen.getByTestId("lc-bootstrap-advance-summary").textContent,
      ).toContain("进行中");
    });
    expect(screen.queryByTestId("lc-bootstrap-action-error")).toBeNull();
    expect(onChanged).toHaveBeenCalled();
  });

  it("规则漂移渲染显眼提示并可一键重新生成政策", async () => {
    installFetchRouter();
    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={driftProjection("rule_digest_drift")}
        onChanged={onChanged}
      />,
    );

    const banner = screen.getByTestId("lc-bootstrap-rule-drift");
    expect(banner.textContent).toContain("规则已变更，需重新生成政策");
    expect(banner.textContent).toContain("provider");
    expect(banner.textContent).toContain("版本号 +1");
    expect(
      screen.getByTestId("lc-bootstrap-rule-drift-detail").textContent,
    ).toContain("根规则入口变了");

    await userEvent.click(
      screen.getByTestId("lc-bootstrap-rule-drift-republish"),
    );

    await waitFor(() => {
      expect(captured).toEqual([
        {
          url: ACTIONS_URL,
          method: "POST",
          body: {
            command_id: "cmd-lc-bootstrap-rules_policy-object-republish",
            step: "rules_policy",
            action: "republish",
            expected_revision: 3,
            expected_object_id: "rules_policy-object",
          },
        },
      ]);
    });
    expect(onChanged).toHaveBeenCalled();
  });

  it("三种漂移原因码各有说明文案（标题统一）", () => {
    const details: Array<[string, string]> = [
      ["policy_digest_drift", "政策正文变了"],
      ["rule_digest_drift", "根规则入口变了"],
      ["policy_reference_drift", "政策引用漂移"],
    ];

    for (const [reasonCode, expected] of details) {
      const { unmount } = render(
        <LogicalCodebaseBootstrapCard
          projectId={PROJECT_ID}
          projection={driftProjection(reasonCode)}
        />,
      );
      expect(
        screen.getByTestId("lc-bootstrap-rule-drift").textContent,
      ).toContain("规则已变更，需重新生成政策");
      expect(
        screen.getByTestId("lc-bootstrap-rule-drift-detail").textContent,
      ).toContain(expected);
      unmount();
    }
  });

  it("规则漂移提示与 notice 不重复渲染同名按钮", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={driftProjection("policy_digest_drift", {
          notices: [
            notice({
              reason_code: "policy_digest_drift",
              allowed_actions: ["republish"],
            }),
          ],
        })}
      />,
    );

    // 同一 (step, action) 只渲染一个「重新生成政策」按钮。
    expect(screen.getAllByRole("button", { name: "重新生成政策" })).toHaveLength(
      1,
    );
    expect(
      screen.queryByTestId("lc-bootstrap-card-action-republish"),
    ).toBeNull();
    expect(
      screen.queryByTestId(
        "lc-bootstrap-step-action-rules_policy-republish",
      ),
    ).toBeNull();
  });


  // 块2 11.10：codegraph 缺失（`bootstrap:codegraph:codegraph_missing` 通知，
  // allowed_actions 为空——聚合索引根本没开始）时给【安装】入口：用户装完
  // codegraph 才能重试索引。
  it("codegraph 阻断通知挂【安装】入口", async () => {
    installFetchRouter();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({
          notices: [
            notice({
              key: "bootstrap:codegraph:codegraph_missing",
              step: "member_index",
              object_id: "member_index-object",
              reason_code: "codegraph_missing",
              summary:
                "未检测到 codegraph\n原因：聚合索引依赖 codegraph CLI，但当前 PATH 上找不到它。\n怎么做：执行 npm i -g @colbymchenry/codegraph 安装（或把已有安装加入 PATH），装好后点「重试」。",
              allowed_actions: [],
              next_step: "aggregate_index_active",
            }),
          ],
        })}
      />,
    );

    const install = await screen.findByTestId("provider-install-start-codegraph");
    expect(install).toHaveTextContent("安装");
    expect(
      screen.getByTestId("lc-bootstrap-notice-codegraph-install"),
    ).toBeTruthy();
    // 安装来源用后端安装目标面的原文。
    expect(
      screen.getByTestId("lc-codegraph-install-source").textContent,
    ).toContain("npm @colbymchenry/codegraph");
  });

  it("非 codegraph 的通知不挂【安装】入口", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({ notices: [notice()] })}
      />,
    );

    expect(
      screen.queryByTestId("lc-bootstrap-notice-codegraph-install"),
    ).toBeNull();
    expect(screen.queryByTestId("provider-install-start-codegraph")).toBeNull();
  });

  it("已就绪或规则步骤正常时不显示规则漂移提示", () => {
    const { unmount } = render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={driftProjection("rule_digest_drift", {
          planning_ready: true,
        })}
      />,
    );
    expect(screen.queryByTestId("lc-bootstrap-rule-drift")).toBeNull();
    unmount();

    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
      />,
    );
    expect(screen.queryByTestId("lc-bootstrap-rule-drift")).toBeNull();
  });
});