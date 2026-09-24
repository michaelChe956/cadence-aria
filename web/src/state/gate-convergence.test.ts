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
  gateFindingsCrossRoundDelta,
  gateFindingsDeltaCopy,
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

// —— REQ-HGC-02 场景 3（跨轮 delta）：本轮 × 前轮 findings 的结构化 identity 对比 ——
//
// 数据锚定 C1 golden（src/product/work_item_plan_policy/fixtures/
// f52-findings-golden.json，Rust tests_fingerprint.rs 已锁定 node_007 与
// node_012 对同一 CT-001 供需缺口「同题异措辞同指纹」）：前端以同一 fingerprint
// 常量驱动，锁定集合差语义——新增=current−prev、已解决=prev−current、
// 复现=prev∩current；任一侧 unstable/前轮缺席/任一侧空 → 整体 unknown
//（findings 空≠历史已解决，历史不全显式 unknown，不猜）。
describe("REQ-HGC-02 跨轮 delta", () => {
  const FP_CT001 = "11".repeat(32);
  const FP_TASK_MAPPING = "22".repeat(32);
  const FP_UNSTABLE = "33".repeat(32);

  const goldenFinding = (overrides: Record<string, unknown>) => ({
    severity: "must_fix",
    message: "WI-001 的输出契约 CT-001 只声明了静态托管文件",
    fingerprint: FP_CT001,
    ...overrides,
  });

  // 前轮（node_007 口径）：CT-001 缺口 + TASK-001 done_when_refs 映射不足。
  const previousRound = [
    goldenFinding({
      message: "WI-001 的输出契约 CT-001 未声明 WI-002 消费时要求的能力",
    }),
    goldenFinding({
      severity: "suggestion",
      class_hint: "advisory",
      message: "TASK-001 的 done_when_refs 仅指向 AC-005",
      fingerprint: FP_TASK_MAPPING,
    }),
  ];
  // 本轮（node_012 口径）：同一 CT-001 缺口换措辞复现 + 一条新缺口。
  const currentRound = [
    goldenFinding({
      message: "edge WI-001 -> WI-002 的 CT-001 capability 覆盖不闭合",
    }),
    goldenFinding({
      severity: "must_fix",
      class_hint: "repairable",
      message: "WI-003 的 CHECK-002 引用了基线外路径",
      fingerprint: "44".repeat(32),
    }),
  ];

  it("counts added/resolved/recurring across rounds by fingerprint identity", () => {
    const delta = gateFindingsCrossRoundDelta(currentRound, previousRound);
    expect(delta).toEqual({
      kind: "counts",
      added: 1,
      resolved: 1,
      recurring: 1,
    });
    expect(gateFindingsDeltaCopy(delta)).toContain("新增 1");
    expect(gateFindingsDeltaCopy(delta)).toContain("已解决 1");
    expect(gateFindingsDeltaCopy(delta)).toContain("复现 1");
  });

  it("treats empty current findings as unknown instead of all-resolved", () => {
    // findings 空 ≠ 历史问题已解决（Review Focus 2）：本轮空集不可推断。
    const delta = gateFindingsCrossRoundDelta([], previousRound);
    expect(delta.kind).toBe("unknown");
    expect(gateFindingsDeltaCopy(delta)).not.toContain("已解决");
  });

  it("reports unknown when the previous round is missing (history incomplete)", () => {
    // 刷新/重连后前轮快照不可得（历史不全）：显式 unknown，不猜。
    const delta = gateFindingsCrossRoundDelta(currentRound, null);
    expect(delta.kind).toBe("unknown");
    expect(gateFindingsDeltaCopy(delta)).not.toContain("新增");
  });

  it("refuses to infer when either round carries an unstable identity", () => {
    const unstableCurrent = [
      goldenFinding({ fingerprint: FP_UNSTABLE, identity_unstable: true }),
    ];
    const delta = gateFindingsCrossRoundDelta(unstableCurrent, previousRound);
    expect(delta.kind).toBe("unknown");

    const unstablePrevious = [
      goldenFinding({ fingerprint: FP_UNSTABLE, identity_unstable: true }),
    ];
    expect(
      gateFindingsCrossRoundDelta(currentRound, unstablePrevious).kind,
    ).toBe("unknown");
  });

  it("keeps the prior snapshot findings when the same gate is re-reviewed", () => {
    useWorkspaceStore.setState({ ...baseGateState(), previousGateFindings: null });
    const frame = (remaining: number, message: string) => ({
      session_id: "session_gate_conv",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      timeline_nodes: [],
      providers: { author: "claude_code", reviewer: null },
      messages: [],
      checkpoints: [],
      human_gate_snapshot: {
        findings: [goldenFinding({ message })],
        repeated_fingerprints: [],
        attempts_used: 0,
        manual_repairs_remaining: remaining,
        trigger: "native_human_required",
        resumable: true,
      },
    });

    useWorkspaceStore.getState().setSessionState(frame(3, "round one") as never);

    // 首次开门：无前轮可比对。
    expect(useWorkspaceStore.getState().previousGateFindings).toBeNull();

    useWorkspaceStore.getState().setSessionState(frame(2, "round two") as never);

    // 同一 logical gate 复评重建：旧快照 findings 成为前轮。
    expect(useWorkspaceStore.getState().previousGateFindings).toEqual([
      goldenFinding({ message: "round one" }),
    ]);
    const delta = gateFindingsCrossRoundDelta(
      useWorkspaceStore.getState().humanGateSnapshot?.findings ?? [],
      useWorkspaceStore.getState().previousGateFindings,
    );
    expect(delta).toEqual({ kind: "counts", added: 0, resolved: 0, recurring: 1 });
  });

  it("drops the previous findings across sessions or gate closure", () => {
    useWorkspaceStore.setState({ ...baseGateState(), previousGateFindings: null });
    const frame = (sessionId: string, withSnapshot: boolean) => ({
      session_id: sessionId,
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      timeline_nodes: [],
      providers: { author: "claude_code", reviewer: null },
      messages: [],
      checkpoints: [],
      ...(withSnapshot
        ? {
            human_gate_snapshot: {
              findings: [goldenFinding({})],
              repeated_fingerprints: [],
              attempts_used: 0,
              manual_repairs_remaining: 3,
              trigger: "native_human_required",
              resumable: true,
            },
          }
        : {}),
    });

    useWorkspaceStore.getState().setSessionState(frame("session_a", true) as never);

    useWorkspaceStore.getState().setSessionState(frame("session_a", false) as never);

    // 关门（快照缺席）：前轮事实作废。
    expect(useWorkspaceStore.getState().previousGateFindings).toBeNull();

    useWorkspaceStore.getState().setSessionState(frame("session_a", true) as never);

    useWorkspaceStore.getState().setSessionState(frame("session_b", true) as never);

    // 跨会话新门：不得拿他门 findings 当前轮。
    expect(useWorkspaceStore.getState().previousGateFindings).toBeNull();
  });
});
