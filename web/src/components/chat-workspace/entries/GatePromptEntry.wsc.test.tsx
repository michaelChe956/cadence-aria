// REQ-WSC-12/13/15（change: bound-review-findings-to-work-item-scope）门卡前端面：
// ① advisory finding 不进「必须处理 N 项」计数；② 每条 finding 一键「排除此项」
// → HumanGateFeedback 带 excludedFindingIds（平台 findingKey），成功后本地即时
// 反馈；③ 门 turn validation_reject 的可操作化呈现（根因 E：被拒反馈原文、可读
// 原因、下一步动作、编译诊断逐字）；④ 排除失败兜底——门面返回成功但引擎未
// 受理（空反馈帧被拒/门内协议错误）时不得显示为已排除（REQ-WSC-15）；
// ⑤ P2——排除-only 帧落盘失败回的是 WsOutMessage::Error（无 code/turn/command
// 关联字段，非 protocol_error）：按「最近本地门动作是排除」归因，撤销乐观
// 「已排除」并就地呈现可读原因；受理回执消费归因标记（防误撤销）。
// 独立文件：GatePromptEntry.test.tsx 已 1151 行，行数门禁（1200）余量不足。
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  handleWorkspaceWsMessage,
  type WsServerMessage,
} from "../../../hooks/workspace-ws-message-handler";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import { GatePromptEntry } from "./GatePromptEntry";

function gateEntry(
  actionBlockReason: "terminal_stage" | "phase_mismatch" | null,
  metadata: Record<string, unknown> = {},
): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-09-17T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: actionBlockReason,
      ...metadata,
    },
  };
}

function actions(): CockpitActionFacade {
  return {
    confirm: vi.fn(() => true),
    confirmReview: vi.fn(() => true),
    feedback: vi.fn(() => true),
    terminate: vi.fn(() => true),
    advance: vi.fn(() => true),
    adoptReview: vi.fn(() => true),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
    recoverCandidate: vi.fn(),
    retryInitialization: vi.fn(async () => undefined),
    confirmTakeover: vi.fn(async () => undefined),
    rebind: vi.fn(),
    sendBootstrapAction: vi.fn(),
  };
}

describe("REQ-WSC-12 gate card advisory counting", () => {
  installWorkspaceStoreTestHooks();

  it("counts only actionable findings in the required reason line", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, {
          findings: [
            { severity: "must_fix", message: "验收标准缺口", disposition: "actionable" },
            {
              severity: "must_fix",
              message: "仓库里规范文档的当前状态",
              disposition: "advisory",
            },
          ],
        })}
        actions={actions()}
      />,
    );

    const why = screen.getByTestId("gate-why");
    expect(why.textContent).toContain("1 条必须处理项");
    expect(why.textContent).not.toContain("2 条必须处理项");
  });
});

describe("REQ-WSC-13 gate card finding exclusion", () => {
  installWorkspaceStoreTestHooks();

  function findingsEntry(extra: Record<string, unknown> = {}): ChatEntry {
    return gateEntry(null, {
      findings: [
        {
          severity: "must_fix",
          message: "前端规范文档的中文 locale 缺口必须回写",
          findingKey: "0123456789abcdef",
        },
      ],
      ...extra,
    });
  }

  it("sends an exclusion-only feedback frame with the platform finding key", async () => {
    const gateActions = actions();
    const user = userEvent.setup();
    render(<GatePromptEntry entry={findingsEntry()} actions={gateActions} />);

    await user.click(screen.getByTestId("finding-exclude-button"));

    expect(gateActions.feedback).toHaveBeenCalledWith("", {
      excludedFindingIds: ["0123456789abcdef"],
    });
  });

  it("reflects the exclusion locally once the frame is out", async () => {
    const user = userEvent.setup();
    render(<GatePromptEntry entry={findingsEntry()} actions={actions()} />);

    await user.click(screen.getByTestId("finding-exclude-button"));

    expect(screen.queryByTestId("finding-exclude-button")).toBeNull();
    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");
    expect(screen.getByTestId("gate-finding-excluded-note").textContent).toContain(
      "已排除 1 项",
    );
  });

  it("keeps the exclusion actionable when the facade refuses the send (fail-closed)", async () => {
    const refusingActions = { ...actions(), feedback: vi.fn(() => false) };
    const user = userEvent.setup();
    render(<GatePromptEntry entry={findingsEntry()} actions={refusingActions} />);

    await user.click(screen.getByTestId("finding-exclude-button"));

    expect(screen.getByTestId("finding-exclude-button")).toBeInTheDocument();
    expect(screen.queryByTestId("finding-excluded-badge")).toBeNull();
    expect(screen.queryByTestId("gate-finding-excluded-note")).toBeNull();
  });

  it("does not present the exclusion as accepted when the engine refuses the empty-feedback frame (REQ-WSC-15)", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <GatePromptEntry entry={findingsEntry()} actions={actions()} />,
    );

    await user.click(screen.getByTestId("finding-exclude-button"));
    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");

    // 门面返回 true ≠ 引擎受理：排除-only 帧（空反馈）被引擎拒收，拒收记录
    // （REQ-WSC-14 四要素）随后到达卡面。
    rerender(
      <GatePromptEntry
        entry={findingsEntry({
          rejectedFeedback: "",
          rejectionReason: "INVALID_HUMAN_GATE_FEEDBACK: feedback must not be blank",
        })}
        actions={actions()}
      />,
    );

    // 发起失败 MUST NOT 显示为已排除：恢复「排除此项」入口并说明未受理。
    expect(screen.getByTestId("finding-exclude-button")).toBeInTheDocument();
    expect(screen.queryByTestId("finding-excluded-badge")).toBeNull();
    expect(screen.queryByTestId("gate-finding-excluded-note")).toBeNull();
    expect(screen.getByTestId("gate-finding-exclusion-refused").textContent).toContain(
      "未被引擎受理",
    );
  });

  it("keeps the local exclusion when the rejection carries the owner's non-empty feedback", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <GatePromptEntry entry={findingsEntry()} actions={actions()} />,
    );

    await user.click(screen.getByTestId("finding-exclude-button"));

    // 非空反馈被拒与本卡排除无关（「排除此项」只发空反馈帧）——归因不扩大。
    rerender(
      <GatePromptEntry
        entry={findingsEntry({
          rejectedFeedback: "把 WI-004 挪到别的 Work Item",
          rejectionReason: "回复与 findings 无对应关系",
        })}
        actions={actions()}
      />,
    );

    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");
    expect(screen.queryByTestId("gate-finding-exclusion-refused")).toBeNull();
  });

  it("reverts the exclusion when a gate protocol error arrives while exclusion is the latest local action", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <GatePromptEntry entry={findingsEntry()} actions={actions()} />,
    );

    await user.click(screen.getByTestId("finding-exclude-button"));

    // k3 P0-2A 现场：排除-only 帧被引擎以协议错误拒回（INVALID_HUMAN_GATE_
    // FEEDBACK）——错误帧归因于紧邻的前一次本地动作（排除），不得显示已排除。
    rerender(
      <GatePromptEntry
        entry={findingsEntry({
          inline_error: {
            code: "INVALID_HUMAN_GATE_FEEDBACK",
            message: "feedback must not be blank",
          },
        })}
        actions={actions()}
      />,
    );

    expect(screen.getByTestId("finding-exclude-button")).toBeInTheDocument();
    expect(screen.queryByTestId("finding-excluded-badge")).toBeNull();
    expect(screen.getByTestId("gate-finding-exclusion-refused")).toBeInTheDocument();
  });

  it("does not offer exclusion while the gate actions are blocked", () => {
    render(
      <GatePromptEntry
        entry={gateEntry("terminal_stage", {
          findings: [
            {
              severity: "must_fix",
              message: "范围外事实",
              findingKey: "0123456789abcdef",
            },
          ],
        })}
        actions={actions()}
      />,
    );

    // findings 仍可见（保留可见性），但排除按钮随动作面一起收起。
    expect(screen.getByTestId("gate-findings")).toBeInTheDocument();
    expect(screen.queryByTestId("finding-exclude-button")).toBeNull();
  });
});

describe("gate card validation_reject actionable display (根因 E)", () => {
  installWorkspaceStoreTestHooks();

  function rejectedEntry(): ChatEntry {
    return gateEntry(null, {
      failure_class: "validation_reject",
      failure_message: "反馈被拒",
      rejectedFeedback: "计划里不写文档更新任务，本次只交付代码",
      rejectionReason: "反馈里没有可执行的修订指令，无法据此重写计划",
      rejectionNextActions: ["补充一句明确要修订哪条计划", "或直接确认当前版本"],
      rejectionDiagnostics: [
        "compile markdown source: line 12: 未闭合的代码块 fence",
        "validate plan candidate IR: missing field `steps[0].acceptance`",
      ],
    });
  }

  it("renders the rejected feedback, readable reason, next actions and verbatim diagnostics", () => {
    render(<GatePromptEntry entry={rejectedEntry()} actions={actions()} />);

    expect(screen.getByTestId("gate-rejection-block")).toBeVisible();
    expect(screen.getByTestId("gate-rejected-feedback").textContent).toContain(
      "计划里不写文档更新任务，本次只交付代码",
    );
    expect(screen.getByTestId("gate-rejection-reason").textContent).toContain(
      "反馈里没有可执行的修订指令",
    );
    const nextActions = screen.getByTestId("gate-rejection-next-actions");
    expect(nextActions.textContent).toContain("补充一句明确要修订哪条计划");
    expect(nextActions.textContent).toContain("或直接确认当前版本");
    // 诊断逐字（含空格与反引号），不做改写、不截断。
    const diagnostics = screen.getByTestId("gate-rejection-diagnostics");
    expect(diagnostics.textContent).toContain(
      "compile markdown source: line 12: 未闭合的代码块 fence",
    );
    expect(diagnostics.textContent).toContain(
      "validate plan candidate IR: missing field `steps[0].acceptance`",
    );
  });

  it("renders no rejection block without rejection metadata (legacy failure turns)", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, { failure_class: "validation_reject" })}
        actions={actions()}
      />,
    );

    expect(screen.queryByTestId("gate-rejection-block")).toBeNull();
  });
});

describe("gate card exclusion persistence failure via WsOutMessage::Error (REQ-WSC-15 P2)", () => {
  installWorkspaceStoreTestHooks();

  // 生产真实失败帧：服务端排除-only 帧落盘失败回 WsOutMessage::Error
  //（src/web/workspace_ws_handler/decisions/inbound.rs「排除项没有落盘，请重试：
  // …」），无 code/turn/command 关联字段、非 protocol_error。用例必须把真实
  // 帧喂给真实 handler（workspace-ws-message-handler），不得只改卡面元数据
  //（rejectedFeedback 为空/inline_error 两种形态生产不会出现）。
  const wsOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const exclusionPersistErrorFrame = (
    message = "排除项没有落盘，请重试：durable store write failed",
  ) => ({ type: "error", message }) as WsServerMessage;

  function entryWithKeyedFinding(): ChatEntry {
    return gateEntry(null, {
      findings: [
        {
          severity: "must_fix",
          message: "范围外事实不该由本 Work Item 修复",
          findingKey: "0123456789abcdef",
        },
      ],
    });
  }

  it("reverts the optimistic exclusion with a readable reason when the real error frame arrives", async () => {
    const user = userEvent.setup();
    render(<GatePromptEntry entry={entryWithKeyedFinding()} actions={actions()} />);

    await user.click(screen.getByTestId("finding-exclude-button"));
    // 门面返回 true（帧已出站）→ 乐观「已排除」。
    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");

    act(() => {
      handleWorkspaceWsMessage(exclusionPersistErrorFrame(), wsOptions());
    });

    // 排除失败 MUST NOT 显示为已排除：入口恢复、计数注记消失、就地给出可读原因。
    expect(screen.getByTestId("finding-exclude-button")).toBeInTheDocument();
    expect(screen.queryByTestId("finding-excluded-badge")).toBeNull();
    expect(screen.queryByTestId("gate-finding-excluded-note")).toBeNull();
    expect(screen.getByTestId("gate-finding-exclusion-refused").textContent).toContain(
      "未被引擎受理",
    );
    expect(screen.getByTestId("gate-exclusion-failure").textContent).toContain(
      "排除未生效",
    );
    expect(screen.getByTestId("gate-exclusion-failure").textContent).toContain(
      "排除项没有落盘，请重试：durable store write failed",
    );
  });

  it("keeps a receipt-confirmed exclusion when a later unrelated error frame arrives", async () => {
    const user = userEvent.setup();
    render(<GatePromptEntry entry={entryWithKeyedFinding()} actions={actions()} />);

    await user.click(screen.getByTestId("finding-exclude-button"));
    act(() => {
      handleWorkspaceWsMessage(
        {
          type: "human_gate_exclusions_accepted",
          command_id: "cmd_1",
          excluded_finding_ids: ["0123456789abcdef"],
        } as WsServerMessage,
        wsOptions(),
      );
    });
    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");

    // 受理回执已消费归因标记——紧随其后的无关 error 帧不得误撤销已确认的排除。
    act(() => {
      handleWorkspaceWsMessage(
        exclusionPersistErrorFrame("生成失败：provider 超时"),
        wsOptions(),
      );
    });

    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");
    expect(screen.queryByTestId("gate-exclusion-failure")).toBeNull();
  });

  it("clears the failure line and re-arms the optimistic exclusion on retry", async () => {
    const user = userEvent.setup();
    render(<GatePromptEntry entry={entryWithKeyedFinding()} actions={actions()} />);

    await user.click(screen.getByTestId("finding-exclude-button"));
    act(() => {
      handleWorkspaceWsMessage(exclusionPersistErrorFrame(), wsOptions());
    });
    expect(screen.getByTestId("gate-exclusion-failure")).toBeInTheDocument();

    // 重试排除：新本地动作清旧失败事实，回到即时反馈形态。
    await user.click(screen.getByTestId("finding-exclude-button"));

    expect(screen.queryByTestId("gate-exclusion-failure")).toBeNull();
    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");
  });
});
