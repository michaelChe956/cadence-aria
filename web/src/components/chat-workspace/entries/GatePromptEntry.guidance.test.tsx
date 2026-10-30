// owner 2026-10-30 真机反馈：门卡写「✓ 可直接确认（需先处理预判）」「引擎判定
// 人工 + 已回复 1 轮 + 剩余修复 2 次」，owner「不知道如何往下继续」。本文件钉住
// 门卡的「现在请做 X」状态指引与机器文案人话化：
//
// 1. 后端投递的 content/summary/下一步动作含机器词（预判/validation_reject）时，
//    前端做**片段级**人话化——已知码映射、黑话只摘除命中片段，句中其余事实
//    原样保留（k3：整段兜底会把真实拒收原因/结论一并丢掉）。
// 2. 被拒原因不套黑话兜底（REQ-WSC-14「可读原因」）：已知码映射、黑话逐字
//    保留——owner 必读的事实优先于措辞美观。
// 3. 门卡顶部「现在请做 X」指引行随门状态分支（≥3 态）且必含下一步动作。
// 4. 「确认当前版本」：反馈被拒时可点+后果说明；不可确认时禁用+人话原因。
// 5. 元数据行计数人话化（信息保留）；advisory/建议级 actionable 归因各自准确。
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import {
  GATE_CONFIRM_CONSEQUENCE_REJECTED,
  GATE_NEXT_STEP_PREFIX,
} from "./gate-next-step";
import { GatePromptEntry } from "./GatePromptEntry";

function gateEntry(
  actionBlockReason: "terminal_stage" | "phase_mismatch" | null,
  gateIdentity?: string,
  metadata: Record<string, unknown> = {},
): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-10-30T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: actionBlockReason,
      ...(gateIdentity ? { gate_identity: gateIdentity } : {}),
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

describe("门卡机器文案人话化（owner 2026-10-30 真机反馈）", () => {
  installWorkspaceStoreTestHooks();

  it("summary 含「需先处理预判」时只摘除黑话片段，句中真实事实保留", () => {
    render(
      <GatePromptEntry
        entry={{
          ...gateEntry(null),
          content: "需要判断: reviewer 是否回退",
          metadata: {
            action_facade: "typed",
            command_id: "cmd_1",
            summary: "✓ 可直接确认（需先处理预判）",
          },
        }}
        actions={actions()}
      />,
    );

    const card = screen.getByTestId("gate-prompt-entry");
    expect(card.textContent).not.toContain("预判");
    // 片段级摘除：黑话本身消失，结论（可直接确认）与要求（需先处理）都保留——
    // 不再整段降级把真实信息一并丢掉（k3 瑕疵 ①）。
    expect(card.textContent).toContain("可直接确认（需先处理）");
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      GATE_NEXT_STEP_PREFIX,
    );
  });

  it("content 与 summary 同时含机器词时人话化后只保留一份（同义去重不受影响）", () => {
    render(
      <GatePromptEntry
        entry={{
          ...gateEntry(null),
          content: "可直接确认（需先处理预判）",
          metadata: {
            action_facade: "typed",
            command_id: "cmd_1",
            summary: "可直接确认（需先处理预判）",
          },
        }}
        actions={actions()}
      />,
    );

    expect(screen.getByTestId("gate-prompt-entry").textContent).not.toContain("预判");
    expect(screen.getAllByText("可直接确认（需先处理）")).toHaveLength(1);
  });

  it("被拒原因的机器码映射为人话；下一步动作黑话只摘除命中片段", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          rejectedFeedback: "把 WI-004 挪到别的 Work Item",
          rejectionReason: "validation_reject",
          rejectionNextActions: ["validation_reject: 请改写后重新提交", "需先处理预判后重试"],
          rejectionDiagnostics: ["raw diagnostics 逐字保留"],
        })}
        actions={actions()}
      />,
    );

    // 已知机器码 → 确认语义的人话映射。
    expect(screen.getByTestId("gate-rejection-reason")).toHaveTextContent(
      "原因：回复未通过校验",
    );
    expect(screen.getByTestId("gate-rejection-reason").textContent).not.toContain(
      "validation_reject",
    );
    // 下一步动作：已知码映射、未知黑话只摘除片段——动作指令本身保留。
    const nextActions = screen.getByTestId("gate-rejection-next-actions");
    expect(nextActions.textContent).toContain("回复未通过校验: 请改写后重新提交");
    expect(nextActions.textContent).toContain("需先处理后重试");
    expect(nextActions.textContent).not.toContain("validation_reject");
    expect(nextActions.textContent).not.toContain("预判");
    // 被拒反馈原文（owner 自己的话）与诊断逐字保留，不改写。
    expect(screen.getByTestId("gate-rejected-feedback")).toHaveTextContent(
      "把 WI-004 挪到别的 Work Item",
    );
    expect(screen.getByTestId("gate-rejection-diagnostics")).toHaveTextContent(
      "raw diagnostics 逐字保留",
    );
  });

  it("被拒原因含黑话时原文保留（REQ-WSC-14 可读原因，不套黑话兜底）", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          rejectedFeedback: "",
          rejectionReason: "需先处理预判：finding FK-1 的验收标准缺口",
        })}
        actions={actions()}
      />,
    );

    // 被拒原因是 owner 必读事实（REQ-WSC-14「可读原因」）：整段替换或片段改写
    // 都会丢真实原因——除已知机器码外逐字保留，黑话不摘除。
    expect(screen.getByTestId("gate-rejection-reason")).toHaveTextContent(
      "原因：需先处理预判：finding FK-1 的验收标准缺口",
    );
  });

  it("正常门卡不含「预判」黑话（回归护栏）", () => {
    render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

    expect(screen.getByTestId("gate-prompt-entry").textContent).not.toContain("预判");
  });
});

describe("门卡 findings 归因（k3 瑕疵 ②：advisory 只认 disposition）", () => {
  installWorkspaceStoreTestHooks();

  it("建议级 actionable 不冒充 advisory：落「可选建议」组、不渲染「仅供参考」徽章", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          findings: [
            {
              severity: "suggestion",
              disposition: "actionable",
              message: "建议补充复杂度说明",
            },
          ],
          verdict: "pass",
          review_gate: "user_confirm_allowed",
        })}
        actions={actions()}
      />,
    );

    // 指引仍走「没有必须处理项」（建议级 actionable 与 advisory 同分支），
    // 但呈现归因准确：行落「可选建议」组，不戴 advisory 的「仅供参考」徽章。
    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("没有必须处理项");
    expect(guidance).toHaveTextContent("采纳建议为反馈");
    expect(screen.getByText("可选建议")).toBeInTheDocument();
    expect(screen.queryByTestId("finding-advisory-badge")).toBeNull();
  });

  it("disposition=advisory 才落「仅供参考（不阻断）」组，不驱「必须处理」指引", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          findings: [
            {
              severity: "must_fix",
              disposition: "advisory",
              message: "仓库里规范文档的当前状态",
            },
          ],
          verdict: "revise",
          review_gate: "requires_revision",
        })}
        actions={actions()}
      />,
    );

    // REQ-WSC-12 前端镜像：advisory（范围外事实）不进必须处理口径。
    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("没有必须处理项");
    expect(guidance.textContent).not.toContain("必须处理：");
    expect(screen.getByText("仅供参考（不阻断）")).toBeInTheDocument();
    expect(screen.getByTestId("finding-advisory-badge")).toBeInTheDocument();
  });
});

describe("门卡「现在请做 X」指引行（按门状态分支）", () => {
  installWorkspaceStoreTestHooks();

  it("正常待决策（有人话裁决）：指引选一个选项或写说明", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          review_gate: "user_triage_required",
          verdict: "needs_human",
          findings: [
            {
              severity: "suggestion",
              message: "WI-004 归属待裁决",
              owner_question: "WI-004 还要不要继续负责 scripts/dev.sh 的改动？",
              owner_options: [
                { label: "不负责，只做验证", consequence: "WI-004 变成纯验证项" },
                { label: "继续负责", consequence: "把 scripts/ 纳入可写入范围" },
              ],
            },
          ],
        })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent(GATE_NEXT_STEP_PREFIX);
    expect(guidance).toHaveTextContent("选一个选项");
    expect(guidance).toHaveTextContent("写一两句说明");
  });

  it("反馈被拒：指引改写重提或确认当前版本继续", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          rejectedFeedback: "把 WI-004 挪走",
          rejectionReason: "回复与 findings 无对应关系",
        })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("没被采纳");
    expect(guidance).toHaveTextContent("重新提交");
    expect(guidance).toHaveTextContent("确认当前版本");
  });

  it("无 finding、只有结论：指引确认当前版本即可继续", () => {
    render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("确认当前版本即可继续");
    expect(guidance).toHaveTextContent("写一两句说明");
  });

  it("有必须处理项：指引写明怎么改，计数保留", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          findings: [
            { severity: "must_fix", message: "缺少验证命令" },
            { severity: "must_fix", message: "缺少回滚说明" },
          ],
          verdict: "revise",
          review_gate: "requires_revision",
        })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("还有 2 条必须处理");
    expect(guidance).toHaveTextContent("写明要怎么改");
    expect(guidance).toHaveTextContent("排除此项");
  });
  it("动作被阻断但终止可用：指引给出人话原因与出路（终止）", () => {
    render(
      <GatePromptEntry
        entry={gateEntry("terminal_stage", undefined, { terminate_block_reason: null })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("已离开人工确认门");
    expect(guidance).toHaveTextContent("终止此门后重新发起");
    expect(screen.getByRole("button", { name: "终止此门" })).toBeVisible();
  });

  it("动作与终止都被阻断：指引明示此卡不再接收操作", () => {
    render(<GatePromptEntry entry={gateEntry("terminal_stage")} actions={actions()} />);

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("已离开人工确认门");
    expect(guidance).toHaveTextContent("此卡不再接收操作");
  });

  it("上下文阻断门：指引补充上下文或终止", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          gate_kind: "work_item_plan_context_blocker",
        })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("输入缺失的上下文");
    expect(guidance).toHaveTextContent("终止此门");
  });

  it("修订进行中：指引等待，不误导成选选项", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateTurnOpen("turn_rev", "cmd_rev", 1);

    render(
      <GatePromptEntry
        entry={gateEntry(null, "turn_rev", { turn_id: "turn_rev" })}
        actions={actions()}
      />,
    );

    const guidance = screen.getByTestId("gate-next-step");
    expect(guidance).toHaveTextContent("请稍候");
    expect(guidance).toHaveTextContent("确认当前版本");
  });

  it("排除成功后指引追加已排除计数（下一轮不再报为需处理）", () => {
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          findings: [
            { severity: "must_fix", message: "缺少验证命令", findingKey: "key_1" },
          ],
          verdict: "revise",
          review_gate: "requires_revision",
        })}
        actions={actions()}
      />,
    );

    expect(screen.getByTestId("gate-next-step").textContent).not.toContain("已排除");

    fireEvent.click(screen.getByTestId("finding-exclude-button"));

    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "你已排除 1 条，下一轮不会再报为需处理",
    );
  });

  it("已收口的门卡不渲染指引行", () => {
    render(
      <GatePromptEntry
        entry={{ ...gateEntry(null), resolved: true, resolution: "confirm" }}
        actions={actions()}
      />,
    );

    expect(screen.queryByTestId("gate-next-step")).toBeNull();
  });
});

describe("「确认当前版本」可用性与后果说明", () => {
  installWorkspaceStoreTestHooks();

  it("反馈被拒时确认可点，按钮旁给后果说明", () => {
    const gateActions = actions();
    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          rejectedFeedback: "把 WI-004 挪走",
          rejectionReason: "回复与 findings 无对应关系",
        })}
        actions={gateActions}
      />,
    );

    const confirm = screen.getByRole("button", { name: "确认当前版本" });
    expect(confirm).toBeEnabled();
    expect(confirm).toHaveAttribute("title", GATE_CONFIRM_CONSEQUENCE_REJECTED);
    expect(screen.getByTestId("gate-confirm-consequence")).toHaveTextContent(
      "确认后将用当前候选继续，不再等你的修改",
    );

    fireEvent.click(confirm);
    expect(gateActions.confirm).toHaveBeenCalledOnce();
  });

  it("门不可确认（已离开门）时按钮禁用并给人话原因", () => {
    const gateActions = actions();
    render(<GatePromptEntry entry={gateEntry("terminal_stage")} actions={gateActions} />);

    const confirm = screen.getByRole("button", { name: "确认当前版本" });
    expect(confirm).toBeDisabled();
    expect(confirm).toHaveAttribute("title", "已离开人工确认门");
    expect(screen.getByTestId("gate-confirm-blocked-reason")).toHaveTextContent(
      "暂不可确认：已离开人工确认门",
    );

    fireEvent.click(confirm);
    expect(gateActions.confirm).not.toHaveBeenCalled();
  });

  it("phase_mismatch 的禁用原因是陈述事实的人话（不再出现「相位」黑话）", () => {
    render(<GatePromptEntry entry={gateEntry("phase_mismatch")} actions={actions()} />);

    const confirm = screen.getByRole("button", { name: "确认当前版本" });
    expect(confirm).toBeDisabled();
    const reason = screen.getByTestId("gate-confirm-blocked-reason");
    expect(reason).toHaveTextContent("会话已进入下一阶段，这扇门不再接收操作");
    expect(reason.textContent).not.toContain("相位");
  });
});

describe("元数据行计数人话化", () => {
  installWorkspaceStoreTestHooks();

  it("触发原因/已回复轮次/剩余修复次数都用 owner 措辞，计数保留", () => {
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 2,
        accepted_feedback_turns: 1,
        trigger: "native_human_required",
        resumable: true,
      },
    });

    render(
      <GatePromptEntry
        entry={gateEntry(null, undefined, {
          gate_trigger: "native_human_required",
          remaining_budget: 2,
        })}
        actions={actions()}
      />,
    );

    const meta = screen.getByTestId("gate-meta");
    expect(meta).toHaveTextContent("为什么找你：引擎自动评审拿不准，需要你来定");
    expect(meta).toHaveTextContent("你已回复过 1 轮");
    expect(meta).toHaveTextContent("还可自动修复 2 次");
    // 旧机器措辞退役。
    expect(meta.textContent).not.toContain("触发：");
    expect(meta.textContent).not.toContain("引擎判定");
    expect(meta.textContent).not.toContain("已反馈");
    expect(meta.textContent).not.toContain("剩余修复轮次");
  });
});
