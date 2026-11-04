// LC 冷启动「停止原因可见化」回归（2026-11-04）：
//
// 后端把「冷启动推进停在哪、为什么停、下一步做什么」落盘（durable 停止
// 记录），并经 GET 投影透出 `projection.advance_stop`：
//   step / reason / reason_code / detail / stopped_at
// 同时合成恰好一条 `bootstrap:advance-stop:*` 通知（summary 是服务端渲染
// 好的中文成品三段：问题/原因/怎么做）。
//
// 本用例钉住前端三件事：
// - advance_stop 存在时，界面能看到**机器码 reason_code** 与**原始诊断
//   detail**（次要行，等宽小字）——不再只剩一行「未开始」；
// - 两道闸门的区分键在界面上可区分：
//   `aggregate_initialization_provider_needs_verification`（provider 选择门：
//   装了但没核验） vs `aggregate_initialization_trust_waiting`（trust 门：
//   没拿到这台机器的信任）；
// - 通知的成品文案由面板的**通用渲染**给出（summary 非空且可见）——前端
//   不重复自拼中文。
import { render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseBootstrapCard } from "./LogicalCodebaseBootstrapCard";
import type {
  BootstrapAdvanceStopDto,
  BootstrapStepProjectionDto,
  LogicalCodebaseBootstrapNoticeDto,
  LogicalCodebaseBootstrapProjection,
  LogicalCodebaseBootstrapStepName,
  LogicalCodebaseBootstrapStepStatus,
} from "../../api/types";

const PROJECT_ID = "project_0001";
const LC_ID = "logical_codebase_0001";

const PROVIDER_GATE_CODE = "aggregate_initialization_provider_needs_verification";
const TRUST_GATE_CODE = "aggregate_initialization_trust_waiting";

const PROVIDER_GATE_DETAIL =
  "no provider carries verified capability evidence | details: {\"candidates\":[{\"provider\":\"pi\",\"state\":\"Unknown\"}]}";
const TRUST_GATE_DETAIL =
  "workspace trust entry is blocked for the route test | details: {\"provider\":\"kimi_code\",\"reason_code\":\"kimi_trust_blocked_for_route_test\"}";

function step(
  name: LogicalCodebaseBootstrapStepName,
  status: LogicalCodebaseBootstrapStepStatus,
  extra: Partial<BootstrapStepProjectionDto> = {},
): BootstrapStepProjectionDto {
  return {
    step: name,
    status,
    object_id: `${name}-object`,
    checkpoint: null,
    failure: null,
    allowed_actions: [],
    ...extra,
  };
}

function advanceStop(
  overrides: Partial<BootstrapAdvanceStopDto> = {},
): BootstrapAdvanceStopDto {
  return {
    step: "rules_policy",
    reason: "waiting_for_human",
    reason_code: TRUST_GATE_CODE,
    detail: TRUST_GATE_DETAIL,
    stopped_at: "2026-11-04T11:02:08.225852594+00:00",
    ...overrides,
  };
}

/** advance-stop 通知：服务端渲染好的成品三段（问题/原因/怎么做/机器码）。 */
function advanceStopNotice(
  reasonCode: string,
  overrides: Partial<LogicalCodebaseBootstrapNoticeDto> = {},
): LogicalCodebaseBootstrapNoticeDto {
  return {
    key: `bootstrap:advance-stop:rules_policy:${reasonCode}`,
    step: "rules_policy",
    object_id: "rules_policy-object",
    reason_code: reasonCode,
    summary: [
      "问题：正在等待 provider 的信任前置确认",
      "原因：provider 尚未获得这台机器上的信任（登录/工作区信任），配方因此不能开始。",
      "怎么做：按界面提示完成该 provider 的授权或信任确认，然后点「继续」。",
      `机器码：${reasonCode}`,
    ].join("\n"),
    external_side_effect: "none",
    allowed_actions: ["prepare"],
    next_step: "member_index",
    created_at: "2026-11-04T11:02:08.225852594+00:00",
    ...overrides,
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

describe("LogicalCodebaseBootstrapCard 冷启动停止原因可见化", () => {
  beforeEach(() => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("advance_stop 存在时机器码与原始诊断降到次要行可见", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({ advance_stop: advanceStop() })}
      />,
    );

    const block = screen.getByTestId("lc-bootstrap-advance-stop");
    // 停在哪一步 + 停止原因（枚举的中文口径）。
    expect(
      screen.getByTestId("lc-bootstrap-advance-stop-headline").textContent,
    ).toContain("规则/政策");
    expect(
      screen.getByTestId("lc-bootstrap-advance-stop-headline").textContent,
    ).toContain("等待人工处理");
    // 机器码：等宽小字的次要行。
    expect(
      screen.getByTestId("lc-bootstrap-advance-stop-machine-line").textContent,
    ).toContain(TRUST_GATE_CODE);
    // 原始诊断：次要行，可见（不是收进折叠区）。
    expect(
      screen.getByTestId("lc-bootstrap-advance-stop-detail").textContent,
    ).toContain("workspace trust entry is blocked");
    expect(block.textContent).toContain(TRUST_GATE_CODE);
  });

  it("两道闸门的 reason_code 在界面上可区分（provider 选择门 vs trust 门）", () => {
    const { unmount } = render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({
          advance_stop: advanceStop({
            reason_code: PROVIDER_GATE_CODE,
            detail: PROVIDER_GATE_DETAIL,
          }),
        })}
      />,
    );
    const providerGate = screen.getByTestId(
      "lc-bootstrap-advance-stop-gate",
    ).textContent;
    expect(providerGate).toContain("核验");
    expect(providerGate).not.toContain("信任");
    expect(
      screen.getByTestId("lc-bootstrap-advance-stop-machine-line").textContent,
    ).toContain(PROVIDER_GATE_CODE);
    unmount();

    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({ advance_stop: advanceStop() })}
      />,
    );
    const trustGate = screen.getByTestId(
      "lc-bootstrap-advance-stop-gate",
    ).textContent;
    expect(trustGate).toContain("信任");
    expect(trustGate).not.toContain("核验");
    expect(
      screen.getByTestId("lc-bootstrap-advance-stop-machine-line").textContent,
    ).toContain(TRUST_GATE_CODE);
  });

  it("advance-stop 通知的成品文案由通用渲染给出（summary 非空且可见）", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({
          advance_stop: advanceStop(),
          notices: [advanceStopNotice(TRUST_GATE_CODE)],
        })}
      />,
    );

    const notice = screen.getByTestId("lc-bootstrap-notice-rules_policy");
    // 通知 summary（服务端成品三段）完整可见——前端不另拼中文。
    expect(notice.textContent).toContain("问题：");
    expect(notice.textContent).toContain("怎么做：");
    expect(notice.textContent).toContain(TRUST_GATE_CODE);
    // 通知键确实是 advance-stop 家族（后端恰好合成一条）。
    expect(
      screen.getAllByTestId("lc-bootstrap-notice-rules_policy"),
    ).toHaveLength(1);
    expect(
      within(notice).getByTestId(
        "lc-bootstrap-notice-machine-line-rules_policy",
      ).textContent,
    ).toContain(TRUST_GATE_CODE);
  });

  it("没有 advance_stop 时不渲染停止原因块", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection()}
      />,
    );

    expect(screen.queryByTestId("lc-bootstrap-advance-stop")).toBeNull();
  });

  it("detail 为空时不渲染诊断行（机器码仍在）", () => {
    render(
      <LogicalCodebaseBootstrapCard
        projectId={PROJECT_ID}
        projection={projection({
          advance_stop: advanceStop({ detail: null }),
        })}
      />,
    );

    expect(screen.queryByTestId("lc-bootstrap-advance-stop-detail")).toBeNull();
    expect(
      screen.getByTestId("lc-bootstrap-advance-stop-machine-line").textContent,
    ).toContain(TRUST_GATE_CODE);
  });
});
