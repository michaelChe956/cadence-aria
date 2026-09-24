// C2 human-gate-convergence（REQ-HGC-02）：批次门区分 + 距通过清单 + 相位提示行删除。
// 现场依据：F-54（两道连续确认门卡片同构不可区分）+ F-52 §三（门卡无「距通过差什么」
// 收敛视图）。全部断言从 durable 投影事实派生，零新协议。
import { describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";
import { buildGatePromptEntry } from "./workspace-chat-rebuild";
import {
  gateBatchConfirmTitle,
  gateBatchConfirmWhyCopy,
  gateDistanceToPass,
} from "./gate-prompt-copy";

function baseGateState() {
  return {
    sessionId: "session_gate_conv",
    stage: "author_confirm",
    workspaceType: "work_item_plan" as const,
    flowKind: "single_candidate" as const,
    sessionStatus: "waiting_for_human" as const,
    humanGateTurn: null,
    humanGateSnapshot: null,
    humanGateClosure: null,
    chatEntries: [],
    timelineNodes: [],
  };
}

function batchConfirmTimelineNode() {
  return {
    node_id: "node_batch_gate",
    node_type: "work_item_batch_confirm" as const,
    agent: null,
    stage: "author_confirm",
    round: null,
    status: "active" as const,
    title: "work_item_batch_confirm",
    summary: null,
    started_at: "2026-09-25T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: { author: "claude_code" as const, reviewer: null, review_rounds: 1 },
    retry: null,
  };
}

describe("REQ-HGC-02 批次确认门专属文案", () => {
  it("emits a dedicated publication card for the batch confirm gate", () => {
    useWorkspaceStore.setState({
      ...baseGateState(),
      timelineNodes: [batchConfirmTimelineNode()],
    });

    const card = buildGatePromptEntry(useWorkspaceStore.getState());
    expect(card).not.toBeNull();
    expect(card?.type).toBe("gate_prompt");
    const metadata = (card?.metadata ?? {}) as Record<string, unknown>;
    expect(metadata.gate_kind).toBe("batch_confirm");
    // 批次门无反馈通路：不得铸成 typed 决策卡（不露 feedback 编辑器）。
    expect(metadata.action_facade).not.toBe("typed");
  });

  it("keeps the candidate revision gate card on the revision copy", () => {
    useWorkspaceStore.setState({
      ...baseGateState(),
      stage: "human_confirm",
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 0,
        manual_repairs_remaining: 2,
        accepted_feedback_turns: 1,
        trigger: "native_human_required",
        resumable: false,
      },
      singleCandidatePhase: "approval",
    });

    const card = buildGatePromptEntry(useWorkspaceStore.getState());
    expect(card).not.toBeNull();
    const metadata = (card?.metadata ?? {}) as Record<string, unknown>;
    expect(metadata.gate_kind ?? "human_gate").not.toBe("batch_confirm");
    expect(metadata.action_facade).toBe("typed");
    // 候选修订门标题不受批次门文案影响（F-50 单标题制维持）。
    expect(gateBatchConfirmTitle()).not.toBe("需要人工确认");
  });

  it("uses publication semantics in the batch title and why copy", () => {
    expect(gateBatchConfirmTitle()).toBe("确认发布整组 Work Items");
    const why = gateBatchConfirmWhyCopy();
    expect(why).toContain("发布");
    expect(why).toContain("确认");
    // 发布含义一句：确认后进入执行（区别于候选修订门的「反馈后再修订」语境）。
    expect(why).not.toContain("反馈后再修订");
  });
});

describe("REQ-HGC-02 距通过清单", () => {
  const finding = (overrides: Record<string, unknown>) => ({
    severity: "suggestion",
    message: "m",
    ...overrides,
  });

  it("lists must_fix/error blockers as pending", () => {
    const items = gateDistanceToPass({
      findings: [
        finding({ severity: "must_fix", message: "CT-001 缺能力" }),
        finding({ severity: "error", class: "mechanical_error", message: "x" }),
        finding({ severity: "suggestion", message: "advisory" }),
      ],
      trigger: "native_human_required",
      approveAvailable: false,
    });
    const blockers = items.find((item) => item.key === "blockers");
    expect(blockers?.status).toBe("pending");
    expect(blockers?.label).toContain("2");
  });

  it("marks options preflight gaps from mechanical_error findings", () => {
    const items = gateDistanceToPass({
      findings: [finding({ severity: "error", class: "mechanical_error" })],
      trigger: "native_human_required",
      approveAvailable: false,
    });
    expect(items.find((item) => item.key === "preflight")?.status).toBe("pending");
  });

  it("marks verification reopen as pending and clean review as ok", () => {
    const verification = gateDistanceToPass({
      findings: [],
      trigger: "verification_new_findings",
      approveAvailable: true,
    });
    expect(verification.find((item) => item.key === "verification")?.status).toBe("pending");

    const clean = gateDistanceToPass({
      findings: [],
      trigger: "native_human_required",
      approveAvailable: true,
    });
    expect(clean.find((item) => item.key === "blockers")?.status).toBe("ok");
    expect(clean.find((item) => item.key === "verification")?.status).toBe("ok");
    expect(clean.find((item) => item.key === "approve")?.status).toBe("ok");
  });

  it("marks approve unavailability and unknown preflight honestly", () => {
    const items = gateDistanceToPass({
      findings: [finding({ severity: "must_fix" })],
      trigger: null,
      approveAvailable: false,
    });
    expect(items.find((item) => item.key === "approve")?.status).toBe("pending");
    // findings 不带 class（review 元数据形态）时预检结果 unknown，不猜。
    expect(items.find((item) => item.key === "preflight")?.status).toBe("unknown");
    expect(items.find((item) => item.key === "verification")?.status).toBe("unknown");
  });
});
