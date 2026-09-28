// C4 Task 10：LC 冷启动恢复面的真实 DTO/按钮交互回归。
//
// - LogicalCodebaseBootstrapCard：真实 bootstrap GET 投影渲染五步状态与
//   等待通知；点击通知按钮经真实 fetch 发送器打统一 action REST（URL/
//   method/body 逐条断言，含稳定 command_id 派生与 expected 身份）；成功
//   后 onChanged 触发同一 GET 补读；失败如实展示，等待事实保留。
// - ChatCockpitPage 的 sendLcBootstrapAction 接线（CockpitInbox 的
//   LcBootstrapCard → facade → 页面 REST 发送器）在 c1-recovery 同款页面
//   级测试模式中覆盖，本文件聚焦生命周期等待卡的产品行为。
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseBootstrapCard } from "./LogicalCodebaseBootstrapCard";
import type { LogicalCodebaseBootstrapProjection } from "../../api/types";

const PROJECT_ID = "project_0001";
const LC_ID = "logical_codebase_0001";

type CapturedRequest = { url: string; method: string; body: unknown };
let captured: CapturedRequest[];

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function installFetchRouter(): void {
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
      method === "POST" &&
      url ===
        `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/bootstrap/actions`
    ) {
      const body: unknown = rawBody === null ? null : JSON.parse(rawBody);
      const command_id =
        body !== null &&
        typeof body === "object" &&
        "command_id" in body &&
        typeof body.command_id === "string"
          ? body.command_id
          : "";
      return jsonResponse({
        command_id,
        outcome: "accepted",
        projection: waitingProjection(),
      });
    }
    return jsonResponse(
      { code: "not_found", message: `no route for ${method} ${url}` },
      404,
    );
  };
  vi.stubGlobal("fetch", vi.fn(router));
}

const step = (
  name: string,
  status: string,
  extra: Record<string, unknown> = {},
) => ({
  step: name,
  status,
  object_id: `${name}-object`,
  checkpoint: null,
  failure: null,
  allowed_actions: [],
  ...extra,
});

function waitingProjection(): LogicalCodebaseBootstrapProjection {
  return {
    project_id: PROJECT_ID,
    logical_codebase_id: LC_ID,
    authority_root: "/workspace/aggregate-root",
    membership_revision: 3,
    policy: {
      policy_id: `policy/${PROJECT_ID}/abc/1`,
      policy_revision: 1,
      policy_digest: "sha256:policy",
      artifact_root: "/workspace/aggregate-root",
    },
    steps: [
      step("identity", "completed"),
      step("manifest_checkout", "completed"),
      step(
        "rules_policy",
        "waiting_for_human",
        {
          failure: {
            reason_code: "member_rules_missing",
            detail: "member alpha missing .claude/rules/language.md",
            retryable: true,
            external_side_effect: "none",
          },
          allowed_actions: ["prepare", "retry"],
        },
      ),
      step("member_index", "failed", {
        object_id: "aggregate_initialization_0001",
        failure: {
          reason_code: "aggregate_pre_check_failed",
          detail: "stage aggregate_initialization: no stderr summary",
          retryable: true,
          external_side_effect: "aggregate_initialization_provider_turn",
        },
        allowed_actions: ["retry"],
      }),
      step("aggregate_index_active", "not_started"),
    ],
    planning_ready: false,
    notices: [
      {
        key: "bootstrap:member_index:aggregate_initialization_0001:aggregate_pre_check_failed",
        step: "member_index",
        object_id: "aggregate_initialization_0001",
        reason_code: "aggregate_pre_check_failed",
        summary: "stage aggregate_initialization: no stderr summary",
        external_side_effect: "aggregate_initialization_provider_turn",
        allowed_actions: ["retry"],
        next_step: "aggregate_index_active",
        created_at: "",
      },
    ],
  };
}

describe("LogicalCodebaseBootstrapCard recovery surface", () => {
  beforeEach(() => {
    captured = [];
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders the durable projection: step statuses, failure reasons and waiting notices", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={waitingProjection()}
      />,
    );

    expect(screen.getByTestId("lc-bootstrap-planning-ready").textContent).toBe(
      "冷启动未完成",
    );
    expect(screen.getByTestId("lc-bootstrap-status-identity").textContent).toBe(
      "已完成",
    );
    expect(
      screen.getByTestId("lc-bootstrap-status-rules_policy").textContent,
    ).toBe("等待人工");
    expect(screen.getByTestId("lc-bootstrap-notice-member_index")).toBeTruthy();
    // 失败原因与可能副作用来自 durable notice，不是前端乐观态。
    expect(screen.getAllByText(/aggregate_pre_check_failed/).length).toBeGreaterThan(0);
    expect(
      screen.getByText(/可能副作用：aggregate_initialization_provider_turn/),
    ).toBeTruthy();
    expect(screen.getByText(/下一步：聚合索引激活/)).toBeTruthy();
    expect(screen.getByTestId("lc-bootstrap-card-action-retry")).toBeTruthy();
  });

  it("dispatches the unified action REST with a stable command id and expected identity", async () => {
    installFetchRouter();
    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={waitingProjection()}
        onChanged={onChanged}
      />,
    );

    await userEvent.click(
      screen.getByTestId("lc-bootstrap-card-action-retry"),
    );

    await waitFor(() => {
      expect(captured).toEqual([
        {
          url: `/api/projects/${PROJECT_ID}/logical-codebases/${LC_ID}/bootstrap/actions`,
          method: "POST",
          body: {
            // 稳定 command id：notice key + action 派生，同命令重放幂等。
            command_id: expect.stringMatching(
              /^cmd-lc-bootstrap-.+-retry$/,
            ),
            step: "member_index",
            action: "retry",
            expected_revision: 3,
            expected_object_id: "aggregate_initialization_0001",
          },
        },
      ]);
    });
    // 成功后经 onChanged 触发同一 bootstrap GET 补读（不乐观改状态）。
    expect(onChanged).toHaveBeenCalled();
    expect(
      screen.queryByTestId("lc-bootstrap-action-error"),
    ).toBeNull();
  });

  it("surfaces the action failure honestly and keeps the waiting facts rendered", async () => {
    captured = [];
    const router = async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ): Promise<Response> => {
      const url = typeof input === "string" ? input : input.toString();
      const method = (init?.method ?? "GET").toUpperCase();
      const rawBody = typeof init?.body === "string" ? init.body : null;
      captured.push({ url, method, body: rawBody });
      return jsonResponse(
        { code: "bootstrap_stale_revision", message: "stale revision" },
        409,
      );
    };
    vi.stubGlobal("fetch", vi.fn(router));

    const onChanged = vi.fn();
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={waitingProjection()}
        onChanged={onChanged}
      />,
    );

    await userEvent.click(
      screen.getByTestId("lc-bootstrap-card-action-retry"),
    );

    await waitFor(() => {
      expect(screen.getByTestId("lc-bootstrap-action-error").textContent).toBe(
        "stale revision",
      );
    });
    // 失败不触发补读，等待事实保留（下轮 GET 刷新仍可见）。
    expect(onChanged).not.toHaveBeenCalled();
    expect(screen.getByTestId("lc-bootstrap-notice-member_index")).toBeTruthy();
    // 按钮恢复可点击（不吞成成功）。
    expect(screen.getByTestId("lc-bootstrap-card-action-retry")).toBeTruthy();
  });
});
