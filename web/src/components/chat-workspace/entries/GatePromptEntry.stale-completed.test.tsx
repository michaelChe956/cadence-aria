// owner 2026-10-31 真机（会话 workspace_session_auto_111eafb6…）：计划已确认完成
// （durable status=confirmed、相位 completed、4 个 Work Item 已建），对话流里第一轮
// 评审的过期门卡仍写「会话已进入下一阶段，这扇门不再接收操作…此时只能终止此门后
// 重新发起」+「终止此门」按钮——owner 读成「我卡死了，只能重来」。这张卡是历史
// 记录、不是待办门：文案必须直说「无需处理」，且不出现指向动作的话。
//
// 本文件钉住过期门卡文案的两条分流：
// 1. 会话已成功完成 → 「已确认完成/历史记录/无需处理」，无「终止此门」「重新发起」
//    「不再接收操作」，「确认当前版本」仍禁用（不得把已完成误判成可操作）。
// 2. 其余过期情形（阶段已推进/会话失败）→ 既有措辞一字不改。
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import { GatePromptEntry } from "./GatePromptEntry";

/**
 * 第一轮评审的过期门卡：开门时刻 action_block_reason=phase_mismatch（F-21 终止
 * 放行），卡上仍带相位阻断措辞——真机现场就是这张卡把 owner 骗成「卡死」。
 */
function staleRoundOneGateEntry(): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-10-31T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: "phase_mismatch",
      terminate_block_reason: null,
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

describe("过期门卡文案：会话已成功完成 → 直说无需处理", () => {
  installWorkspaceStoreTestHooks();

  it("计划已确认完成时，过期门卡显示「已确认完成/历史记录/无需处理」，不再指向终止或重新发起", () => {
    const gateActions = actions();
    // 真机终态：durable session_status=confirmed、相位 completed、门已不存在。
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      flowKind: "single_candidate",
      stage: "human_confirm",
      singleCandidatePhase: "completed",
      sessionStatus: "confirmed",
      humanGateSnapshot: null,
      humanGateTurn: null,
      humanGateClosure: null,
    });

    render(<GatePromptEntry entry={staleRoundOneGateEntry()} actions={gateActions} />);

    const card = screen.getByTestId("gate-prompt-entry");
    expect(card.textContent).toContain("已确认完成");
    expect(card.textContent).toContain("历史记录");
    expect(card.textContent).toContain("无需处理");
    // 误导措辞全部退役：不再是「这扇门不再接收操作」，也不再让 owner 去
    // 「终止此门后重新发起」。
    expect(card.textContent).not.toContain("不再接收操作");
    expect(card.textContent).not.toContain("重新发起");
    expect(card.textContent).not.toContain("终止此门");
    // 不得把「已完成」误判成「可操作」：确认按钮仍是禁用的。
    expect(screen.getByRole("button", { name: "确认当前版本" })).toBeDisabled();
    // 历史记录卡不是待办门：终止按钮不挂（终态会话终止无意义，服务端也会拒）。
    expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.terminate).not.toHaveBeenCalled();
  });

  it("会话未完成（仍在等人工）时，过期门卡沿用既有措辞", () => {
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      flowKind: "single_candidate",
      stage: "human_confirm",
      singleCandidatePhase: "prepare",
      sessionStatus: "waiting_for_human",
      humanGateSnapshot: null,
      humanGateTurn: null,
      humanGateClosure: null,
    });

    render(<GatePromptEntry entry={staleRoundOneGateEntry()} actions={actions()} />);

    expect(screen.getByTestId("gate-confirm-blocked-reason")).toHaveTextContent(
      "会话已进入下一阶段，这扇门不再接收操作；可终止此门后重新发起",
    );
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "会话已进入下一阶段，这扇门不再接收操作；此时只能终止此门后重新发起",
    );
    expect(screen.getByRole("button", { name: "终止此门" })).toBeVisible();
  });

  it("会话失败时，过期门卡沿用既有措辞", () => {
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      flowKind: "single_candidate",
      stage: "human_confirm",
      singleCandidatePhase: "failed",
      sessionStatus: "failed",
      humanGateSnapshot: null,
      humanGateTurn: null,
      humanGateClosure: null,
    });

    render(<GatePromptEntry entry={staleRoundOneGateEntry()} actions={actions()} />);

    expect(screen.getByTestId("gate-confirm-blocked-reason")).toHaveTextContent(
      "会话已进入下一阶段，这扇门不再接收操作；可终止此门后重新发起",
    );
    expect(screen.getByTestId("gate-next-step")).toHaveTextContent(
      "此时只能终止此门后重新发起",
    );
  });
});
