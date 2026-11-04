// REQ-HGC-01「卡片刷新」（块1 7.8）：当前门卡重建 / 重连 SHALL 刷新同门当前事实；旧卡
// 只读历史。以下 3 例原以 +115 行追加于 GatePromptEntry.test.tsx 尾部（原行
// 1178-1292），致其 1292 行越过 1200 行硬线——按本目录既有切法（参照
// GatePromptEntry.wsc.test.tsx 头注先例）独立成文件；用例内容与原区间逐字节
// 一致（见 cadence/notes/2026-11-04_越线拆分_GatePromptEntry测试_v1.0.md）。
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
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
    timestamp: "2026-09-17T00:00:00Z",
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

// REQ-HGC-01「卡片刷新」：当前门卡的重建 / 重连 SHALL 刷新同门当前事实；旧卡
// 只读历史事实，SHALL NOT 呈现为当前授权。
//
// 修前：门卡余量只读卡片自带的创建时快照 remaining_budget——同一门受理一次
// 反馈后 durable 余量已真实递减（3→2），卡面仍停在「还可做 3 次」，与同屏
// 的「你已说明 1 轮」自相矛盾。当前卡改读同门当前投影的余量；旧卡（已留档 /
// 门身份已被当前门取代）仍只读历史事实。
describe("REQ-HGC-01 卡片刷新：当前卡读同门当前事实，旧卡只读历史", () => {
  installWorkspaceStoreTestHooks();

  function gateSessionState(manualRepairsRemaining: number) {
    return {
      session_id: "session_gate_refresh",
      workspace_type: "work_item_plan" as const,
      stage: "human_confirm",
      session_status: "waiting_for_human" as const,
      flow_kind: "single_candidate" as const,
      run_policy: "auto_if_valid" as const,
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 0,
        transitions_used: 0,
        initial_review_count: 1,
        verification_review_count: 0,
      },
      human_gate_snapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: manualRepairsRemaining,
        accepted_feedback_turns: 1,
        trigger: "native_human_required" as const,
        resumable: true,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code" as const, reviewer: null },
    };
  }

  it("同一门受理反馈后，当前卡刷新为同门当前余量，不停在卡片创建时的快照", () => {
    const store = useWorkspaceStore.getState();
    // 门开出：durable 快照余量 3，typed turn 开出（预算 3）→ 卡面创建时快照 3。
    store.setSessionState(gateSessionState(3));
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 3);
    // 同一门受理一次反馈 → durable 余量真实递减为 2（已说明 1 轮）。
    store.setSessionState(gateSessionState(2));

    render(
      <GatePromptEntry
        entry={gateEntry(null, "turn_1", {
          turn_id: "turn_1",
          gate_trigger: "native_human_required",
          remaining_budget: 3,
        })}
        actions={actions()}
      />,
    );

    const meta = screen.getByTestId("gate-meta");
    expect(meta).toHaveTextContent("人工返修还可做 2 次");
    expect(meta).not.toHaveTextContent("人工返修还可做 3 次");
    expect(meta).toHaveTextContent("你已说明 1 轮");
  });

  it("已留档的旧轮卡只读历史事实，不跟着当前门刷新", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(gateSessionState(3));
    store.applyHumanGateTurnOpen("turn_0", "cmd_0", 3);
    store.setSessionState(gateSessionState(2));
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    const archived: ChatEntry = {
      ...gateEntry(null, "turn_0", {
        turn_id: "turn_0",
        gate_trigger: "native_human_required",
        remaining_budget: 3,
      }),
      resolved: true,
      resolution: "superseded",
    };

    render(<GatePromptEntry entry={archived} actions={actions()} />);

    // 旧卡不得呈现当前门的余量，也不得把创建时快照读成当前授权。
    const meta = screen.getByTestId("gate-meta");
    expect(meta.textContent).not.toContain("人工返修还可做 2 次");
    expect(meta).toHaveTextContent("预算历史不可用");
  });

  it("门身份已被当前门取代的残留卡只读自己的创建时快照，不读当前门余量", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(gateSessionState(3));
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 3);
    store.setSessionState(gateSessionState(2));

    render(
      <GatePromptEntry
        entry={gateEntry(null, "turn_0", {
          turn_id: "turn_0",
          gate_trigger: "native_human_required",
          remaining_budget: 3,
        })}
        actions={actions()}
      />,
    );

    const meta = screen.getByTestId("gate-meta");
    expect(meta).toHaveTextContent("人工返修还可做 3 次");
    expect(meta).not.toHaveTextContent("人工返修还可做 2 次");
  });
});
