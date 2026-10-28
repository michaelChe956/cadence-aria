import { describe, expect, it } from "vitest";
import type { WorkspaceWsState } from "./workspace-ws-store-types";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";
import {
  c1TargetLabel,
  c1WaitingItem,
  type ClassifiedC1WaitingItem,
  codingAttentionIdentity,
  codingAttentionItem,
  cockpitInboxItemSessionId,
  dedupeCodingChoiceInboxItems,
  inboxCodingChoiceIdentity,
  type CockpitInboxItem,
  isStaleDriverLeaseItem,
  logicalCodebaseBootstrapItem,
  selectCockpitInbox,
} from "./workspace-cockpit-projection";
import type { C1WaitingItem, CodingAttentionItemDto } from "../api/types";
import {
  lcFreshOperationFixture,
  lcResumeRecoveryFixture,
  reviewVerdictEntry,
} from "./workspace-cockpit-projection.test-fixtures";

// 从 workspace-cockpit-projection.test.ts 拆出（large_file_guard 1200 行上限，
// 纯移动零行为变化）：收件箱投影 / C1 与 LC 恢复卡 / trust 缺失 / 待处理事项 /
// 身份与去重的用例（断言逐字保留）。

describe("workspace cockpit inbox projection", () => {
  installWorkspaceStoreTestHooks();

  it("projects a triage gate as one severity-2 item", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.setHumanGateSnapshot({
      findings: [],
      repeated_fingerprints: [],
      attempts_used: 1,
      manual_repairs_remaining: 2,
      trigger: "repeated_fingerprint",
      resumable: true,
    });
    store.appendChatEntry(reviewVerdictEntry({ verdict: "needs_human" }));

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      id: expect.stringMatching(/^gate:snapshot:/),
      kind: "gate",
      severity: 2,
      triage: true,
      source: "gate",
    });
    // F-50 fix round 标题去重：triage 短语只保留主区门卡卡头——抽屉门条目
    // 与非 triage 门同题（gateSummaryTitle 不再分叉）。
    expect(items[0]?.title).toBe("需要人工确认");
  });

  it("projects a user_triage_required verdict gate under the unified inbox title", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.appendChatEntry(reviewVerdictEntry({ review_gate: "user_triage_required" }));

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items[0]?.triage).toBe(true);
    expect(items[0]?.title).toBe("需要人工确认");
  });

  it("projects a stopped session item", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionStatus("stopped_needs_human");

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toEqual([
      expect.objectContaining({
        id: "stopped:session",
        kind: "stopped",
        severity: 2,
        source: "session_status",
        summary: "停点原因：stopped_needs_human · 等待人工接管后继续",
      }),
    ]);
  });

  // 批 2 P5（REQ-WIGA-08 C1 前端半）：stopped 卡的同源托管管理态——durable
  // automation 归属（P0 1.2 WS session_state.automation）owner=server 且
  // enabled 的会话即「enabled 精确绑定当前 enrollment」（与 P4 服务端 takeover
  // 拒绝同源），收件箱据此隐藏接管按钮；前端隐藏只是同步呈现，不以隐藏
  // 代替服务端拒绝。
  it("projects the managed flag and recovery copy on stopped items for server-owned enabled sessions", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionStatus("stopped_needs_human");
    useWorkspaceStore.setState({
      automation: {
        owner: "server",
        enrollment_id: "enrollment_0001",
        policy_revision: 1,
        enabled: true,
      },
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toEqual([
      expect.objectContaining({
        id: "stopped:session",
        kind: "stopped",
        automationManaged: true,
      }),
    ]);
    expect(items[0]?.summary).not.toContain("等待人工接管");
    expect(items[0]?.summary).toContain("对话窗口");
  });

  it("keeps the takeover copy and no managed flag for non-managed stopped sessions", () => {
    for (const automation of [
      null,
      { owner: "client" as const, enrollment_id: "e", policy_revision: 1, enabled: false },
      { owner: "server" as const, enrollment_id: "e", policy_revision: 1, enabled: false },
    ]) {
      useWorkspaceStore.setState({ automation });
      useWorkspaceStore.getState().setSessionStatus("stopped_needs_human");

      const items = selectCockpitInbox(useWorkspaceStore.getState());

      expect(items[0]).toMatchObject({
        kind: "stopped",
        automationManaged: false,
        summary: "停点原因：stopped_needs_human · 等待人工接管后继续",
      });
    }
  });

  it("projects protocol errors as hard error items", () => {
    useWorkspaceStore.getState().setProtocolError({
      code: "INVALID_HUMAN_CONFIRM_ACTION",
      message: "confirm is rejected on this gate",
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toEqual([
      expect.objectContaining({
        id: "hard_error:protocol:INVALID_HUMAN_CONFIRM_ACTION",
        kind: "hard_error",
        severity: 3,
        summary: "confirm is rejected on this gate",
      }),
    ]);
  });

  it("exposes the protocol error code so the inbox can offer a lease retake (F-11)", () => {
    useWorkspaceStore.getState().setProtocolError({
      code: "STALE_DRIVER_LEASE",
      message: "driver connection no longer holds the lease",
    });

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items).toHaveLength(1);
    expect(items[0]?.protocolErrorCode).toBe("STALE_DRIVER_LEASE");
    expect(isStaleDriverLeaseItem(items[0]!)).toBe(true);
    expect(
      isStaleDriverLeaseItem({ ...items[0]!, protocolErrorCode: "OTHER_CODE" }),
    ).toBe(false);
    expect(isStaleDriverLeaseItem({ ...items[0]!, source: "engine_error" })).toBe(false);
  });

  it("keeps a real advance rejection and drops a replayed one", () => {
    const store = useWorkspaceStore.getState();
    store.applyAdvanceRejected("cmd_real", "ADVANCE_NOT_READY", "gate still open");
    store.applyAdvanceRejected("cmd_replay", "ADVANCE_REPLAY_NOT_READY", "durable record exists");

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items.map((item) => item.id)).toEqual(["hard_error:advance:cmd_real"]);
  });

  it("removes the gate item once the gate is closed", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(1);

    store.applyHumanGateClosed("confirm", "human_confirm");

    expect(selectCockpitInbox(useWorkspaceStore.getState())).toHaveLength(0);
  });

  // 闭环保留不得让「下一代 legacy 门」被误判为已收：
  // 进入 human_confirm 即新门开启，必须清上一轮闭环（原设计里清 closure 的逃生口）。
  it("clears the previous closure when a new legacy gate opens", () => {
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateClosed("confirm", "human_confirm");
    store.setStage("compile_plan");

    store.setStage("human_confirm");

    expect(useWorkspaceStore.getState().humanGateClosure).toBeNull();
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).filter((item) => item.kind === "gate"),
    ).toHaveLength(1);
  });

  it("sorts by severity descending then newest first", () => {
    const store = useWorkspaceStore.getState();
    store.setProtocolError({ code: "SOME_PROTOCOL_ERROR", message: "hard" });
    store.setStage("human_confirm");

    const items = selectCockpitInbox(useWorkspaceStore.getState());

    expect(items.map((item) => item.kind)).toEqual(["hard_error", "gate"]);
  });
});

describe("C5 project repository initialization waiting projection", () => {
  const projectWaiting: C1WaitingItem = {
    id: "c1:project:project_0001:repository_init:op_init_0001",
    kind: "repository_initialization_failed",
    reason:
      "repository initialization failed at pre_check (provider_unavailable); awaiting gateway recovery",
    completed_steps: ["cadence_skills"],
    target: null,
    plan_id: null,
    session_id: null,
    attempt_id: null,
    gate_id: null,
    possible_side_effect: null,
    actions: ["resume_repository_initialization"],
    next_phase: "repository_registered",
    action_context: [],
    operation_id: "op_init_0001",
    diagnostics: {
      failed_step: "pre_check",
      reason_code: "provider_unavailable",
      provider: "claude_code",
      stderr_summary: null,
      changed_paths: [],
      retryable: true,
    },
    project_id: "project_0001",
  };

  it("projects a project-level item with the backend id and no issue identity", () => {
    const item = c1WaitingItem(projectWaiting, "project_0001");
    expect(item.id).toBe("c1:project:project_0001:repository_init:op_init_0001");
    expect(item.id).not.toContain("undefined");
    expect(item.kind).toBe("c1_recovery");
    expect(item.c1Info?.issueId).toBeNull();
    expect(item.c1Info?.projectId).toBe("project_0001");
    expect(item.c1Info?.operationId).toBe("op_init_0001");
    expect(item.c1Info?.diagnostics?.failedStep).toBe("pre_check");
    expect(item.c1Info?.diagnostics?.reasonCode).toBe("provider_unavailable");
    expect(item.c1Info?.diagnostics?.provider).toBe("claude_code");
    expect(item.c1Info?.diagnostics?.retryable).toBe(true);
    expect(item.title).toBe("仓库初始化失败，等待恢复");
  });

  it("keeps the issue-level id prefix and tolerates missing diagnostics", () => {
    const item = c1WaitingItem(
      {
        ...projectWaiting,
        id: "c1:advance_retry_failed:advance_0001",
        kind: "advance_retry_failed",
        operation_id: null,
        diagnostics: null,
      },
      "project_0001",
      "issue_0001",
    );
    expect(item.id).toBe("c1:issue_0001:c1:advance_retry_failed:advance_0001");
    expect(item.c1Info?.issueId).toBe("issue_0001");
    expect(item.c1Info?.operationId).toBeNull();
    expect(item.c1Info?.diagnostics).toBeNull();
  });
});

// F-A（owner 2026-10-25 裁定，cadence/notes/2026-10-25_UI复验发现）：等待项
// 可操作性分类——lease_wait 与当前绑定健在的 generation_history 是自动等待/
// 只读信息，投 kind="info"（两个待处理抽屉均按 kind 分组：info 落「进度信息」、
// 不渲染 C1 恢复卡/动作按钮、不进待处理计数）；需要人工的维持 c1_recovery。
describe("F-A waiting item operability classification", () => {
  const leaseWaitWaiting: ClassifiedC1WaitingItem = {
    id: "c1:lease_wait:issue_0001:coding_attempt_3681bce5",
    kind: "lease_wait",
    reason: "lease active; automation keeps waiting (lease coding_attempt_3681bce5)",
    completed_steps: [],
    target: null,
    plan_id: "plan_0001",
    session_id: "wsp_0001",
    attempt_id: null,
    gate_id: null,
    possible_side_effect: "2026-10-25T08:00:00Z",
    actions: [],
    next_phase: null,
    requires_human: false,
  };
  const generationHistoryReadOnly: ClassifiedC1WaitingItem = {
    id: "c1:generation_history:issue_0001",
    kind: "generation_history",
    reason: "5 previous binding generation(s) kept read-only; current binding active",
    completed_steps: [],
    target: null,
    plan_id: "plan_0006",
    session_id: "wsp_0006",
    attempt_id: null,
    gate_id: null,
    possible_side_effect: null,
    actions: [],
    next_phase: null,
    requires_human: false,
  };
  const generationHistoryDangling: ClassifiedC1WaitingItem = {
    ...generationHistoryReadOnly,
    reason: "5 previous binding generation(s) kept read-only; current binding dangling (plan)",
    actions: ["rebind"],
    next_phase: "rebind",
    requires_human: true,
  };

  it("classifies active-lease waiting as info out of the needs-human group", () => {
    const item = c1WaitingItem(leaseWaitWaiting, "project_0001", "issue_0001");
    expect(item.kind).toBe("info");
    expect(item.severity).toBe(3);
    expect(item.title).toBe("租约活跃，自动链等待中");
    expect(item.summary).toContain("coding_attempt_3681bce5");
    expect(item.c1Info?.actions).toEqual([]);
  });

  it("downgrades generation_history to read-only info while the current binding is live", () => {
    const item = c1WaitingItem(generationHistoryReadOnly, "project_0001", "issue_0001");
    expect(item.kind).toBe("info");
    expect(item.title).toBe("存在旧代绑定（只读可查）");
    expect(item.c1Info?.actions).toEqual([]);
    expect(item.c1Info?.nextPhase).toBeNull();
  });

  it("keeps generation_history actionable when the current binding is dangling", () => {
    const item = c1WaitingItem(generationHistoryDangling, "project_0001", "issue_0001");
    expect(item.kind).toBe("c1_recovery");
    expect(item.severity).toBe(2);
    expect(item.c1Info?.actions).toEqual(["rebind"]);
    expect(item.c1Info?.nextPhase).toBe("rebind");
  });

  it("treats old responses without the flag as needing human", () => {
    const legacyWire: ClassifiedC1WaitingItem = {
      ...leaseWaitWaiting,
      requires_human: undefined,
    };
    const legacy = c1WaitingItem(legacyWire, "project_0001", "issue_0001");
    expect(legacy.kind).toBe("c1_recovery");
  });
});

describe("anchor_t19 cockpit inbox lc resume recovery card", () => {
  installWorkspaceStoreTestHooks();

  function recoverySessionState(
    extras: Record<string, unknown> = {},
  ): Record<string, unknown> {
    return {
      session_id: "session_rec",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "waiting_for_human",
      flow_kind: "legacy",
      run_policy: "interactive",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 0,
        transitions_used: 0,
        initial_review_count: 0,
        verification_review_count: 0,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: null },
      ...extras,
    };
  }

  it("anchor_t19_driver_observer_recovery_projection_and_resolved_clear", () => {
    // 三活跃态各投一张恢复卡；卡面携带 D 层渲染所需的完整恢复投影。
    for (const state of ["waiting", "fresh_reserved", "fresh_running"] as const) {
      useWorkspaceStore.getState().setSessionState(
        recoverySessionState({
          lc_resume_recovery: lcResumeRecoveryFixture({
            state,
            last_operation: lcFreshOperationFixture({ state: "failed", reason: "prepare_failed" }),
          }),
        }) as never,
      );
      const items = selectCockpitInbox(useWorkspaceStore.getState());
      const card = items.find((item) => item.kind === "lc_resume_recovery");
      expect(card, `state=${state}`).toMatchObject({
        id: "lc_resume_recovery:rec-1",
        severity: 2,
        source: "lc_resume_recovery",
      });
      expect(card?.recovery).toMatchObject({
        recoveryId: "rec-1",
        state,
        reasonCode: "resume_fingerprint_mismatch",
        provider: "codex",
        failedNodeId: "node-1",
        runKind: "legacy_draft",
        detectedAt: "2026-10-11T00:02:00Z",
        updatedAt: "2026-10-11T00:04:00Z",
        // 不推权限：allowedActions 只镜像 wire allowed_actions（B 未接齐恒空），
        // 不从 reason_code/错误文本推导动作。
        allowedActions: [],
        lastOperation: { operation_id: "op-1", state: "failed", reason: "prepare_failed" },
      });
    }

    // resolved：卡消失（不投 lc_resume_recovery 卡）……
    useWorkspaceStore.getState().setSessionState(
      recoverySessionState({
        lc_resume_recovery: lcResumeRecoveryFixture({
          state: "resolved",
          last_operation: lcFreshOperationFixture({ state: "completed", fresh_node: "node-2" }),
        }),
      }) as never,
    );
    const resolvedState = useWorkspaceStore.getState();
    expect(resolvedState.lcResumeRecovery).toMatchObject({
      recovery_id: "rec-1",
      state: "resolved",
    });
    expect(
      selectCockpitInbox(resolvedState).some((item) => item.kind === "lc_resume_recovery"),
    ).toBe(false);

    // ……且 generic 卡不被永久隐藏（resolved 不阻塞合法后续动作的收件箱事实）。
    useWorkspaceStore.getState().setSessionStatus("stopped_needs_human");
    const afterResolved = selectCockpitInbox(useWorkspaceStore.getState());
    expect(
      afterResolved.some((item) => item.kind === "lc_resume_recovery"),
    ).toBe(false);
    expect(
      afterResolved.some((item) => item.kind === "stopped"),
    ).toBe(true);

    // null：无恢复事实不投卡。
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setSessionState(recoverySessionState() as never);
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).some(
        (item) => item.kind === "lc_resume_recovery",
      ),
    ).toBe(false);
  });
});

// 「登记并重试」按钮投影（owner 2026-10-12 直批）：trust 缺失错误按稳定码
// （provider_trust_missing/trust_entry_missing）识别 + provider wire 标签
// 解析；typed 回执对账后同一错误条切 trustRegistered 态。
describe("trust missing inbox projection", () => {
  function trustErrorState(): WorkspaceWsState {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError(
      "provider_trust_missing: trust_entry_missing: canonical root 下未找到 codex 的用户级 workspace trust 条目（entry absent）",
    );
    return useWorkspaceStore.getState();
  }

  it("trust 缺失错误条挂 trustMissing 投影（稳定码+provider 解析）", () => {
    const items = selectCockpitInbox(trustErrorState());
    const errorItem = items.find((item) => item.id === "hard_error:error");
    expect(errorItem?.trustMissing).toEqual({ provider: "codex" });
    expect(errorItem?.trustRegistered).toBe(false);
  });

  it("kimi_code 标签同样识别；无稳定码的普通错误不投登记面", () => {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError(
      "provider_trust_missing: trust_entry_missing: canonical root 下未找到 kimi_code 的用户级 workspace trust 条目",
    );
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).find((item) => item.id === "hard_error:error")
        ?.trustMissing,
    ).toEqual({ provider: "kimi_code" });

    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError("provider start failed: command missing");
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).find((item) => item.id === "hard_error:error")
        ?.trustMissing,
    ).toBeNull();
  });

  it("typed 回执 provider 匹配时切 trustRegistered；不匹配保持可登记", () => {
    const before = selectCockpitInbox(trustErrorState());
    expect(before.find((item) => item.id === "hard_error:error")?.trustRegistered).toBe(false);

    useWorkspaceStore.getState().applyTrustEntryRegistered({
      provider: "codex",
      action: "registered",
      trust_key: "[projects.\"/fixture-root\"]",
      canonical_root: "/fixture-root",
      registered_at: "2026-10-12T00:00:00Z",
    });
    const after = selectCockpitInbox(useWorkspaceStore.getState());
    expect(after.find((item) => item.id === "hard_error:error")?.trustRegistered).toBe(true);

    useWorkspaceStore.getState().applyTrustEntryRegistered({
      provider: "kimi_code",
      action: "registered",
      trust_key: "wd_fixture_000000000000",
      canonical_root: "/fixture-root",
      registered_at: "2026-10-12T00:00:00Z",
    });
    const mismatched = selectCockpitInbox(useWorkspaceStore.getState());
    expect(mismatched.find((item) => item.id === "hard_error:error")?.trustRegistered).toBe(false);
  });

  // 指引卡简化（owner 2026-10-12 直批）：结构化 trust_missing_guidance
  // 帧落 store 后，投影优先用 wire 事实（providers/canonicalRoot/
  // commandMarkdown），不再解析 error 文本；error 缺席不投卡。
  it("结构化指导优先于文本解析：providers/canonicalRoot/commandMarkdown 直挂投影", () => {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().setError(
      "provider_trust_missing: trust_entry_missing: canonical root 下未找到 kimi_code 的用户级 workspace trust 条目",
    );
    useWorkspaceStore.getState().applyTrustMissingGuidance({
      providers: ["kimi_code", "codex"],
      canonicalRoot: "/aggregate/root",
      reasonCode: "trust_entry_missing",
      retryAction: "人工登记后显式重新生成",
      commandMarkdown:
        "mkdir -p '/home/u/.kimi-code/workspace-trust' && printf '{\"root\":\"/aggregate/root\",\"trustedAt\":%s}' \"$(date +%s%3N)\" > '/home/u/.kimi-code/workspace-trust/wd_root_000000000000'",
    });
    const item = selectCockpitInbox(useWorkspaceStore.getState()).find(
      (entry) => entry.id === "hard_error:error",
    );
    expect(item?.trustMissing).toEqual({
      provider: "kimi_code",
      providers: ["kimi_code", "codex"],
      canonicalRoot: "/aggregate/root",
      commandMarkdown:
        "mkdir -p '/home/u/.kimi-code/workspace-trust' && printf '{\"root\":\"/aggregate/root\",\"trustedAt\":%s}' \"$(date +%s%3N)\" > '/home/u/.kimi-code/workspace-trust/wd_root_000000000000'",
      retryAction: "人工登记后显式重新生成",
    });
  });

  it("结构化指导缺席时回落既有稳定码文本识别（旧服务端兼容）", () => {
    const items = selectCockpitInbox(trustErrorState());
    expect(items.find((item) => item.id === "hard_error:error")?.trustMissing).toEqual({
      provider: "codex",
    });
  });

  it("error 缺席时结构化指导不单独投卡（指导只配对错误条）", () => {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.getState().applyTrustMissingGuidance({
      providers: ["codex"],
      canonicalRoot: "/aggregate/root",
      reasonCode: "trust_entry_missing",
      retryAction: "人工登记后显式重新生成",
      commandMarkdown: null,
    });
    const items = selectCockpitInbox(useWorkspaceStore.getState());
    expect(items.find((item) => item.id === "hard_error:error")).toBeUndefined();
  });
});

// ① UI 语义（owner 直批 codegraph 错误语义化）：空仓/成员无可索引代码
// 文件时，bootstrap 失败卡沿既有通道呈现业务文案——专用标题 + 成员
// 清单 +「每成员至少 1 个可索引代码文件」指引；通用 CLI 失败仍走既有
// 标题，不新建卡型。
describe("lc_bootstrap 聚合索引覆盖不足文案", () => {
  const projection = {
    project_id: "project_0001",
    logical_codebase_id: "logical_codebase_0001",
    membership_revision: 5,
  };

  it("覆盖不足：专用标题 + 无可索引文件的成员清单 + 每成员至少 1 个可索引代码文件", () => {
    const item = logicalCodebaseBootstrapItem(
      {
        key: "lc:aggregate_index_active:aggregate_index_cov_0001",
        step: "aggregate_index_active",
        object_id: "aggregate_index_cov_0001",
        reason_code: "aggregate_index_member_coverage_failed",
        summary:
          "aggregate_index_failed:aggregate_index_member_coverage_failed: no indexable code file under included members [api, web]; each included member needs at least 1 indexable code file",
        external_side_effect: "aggregate_index_cli",
        allowed_actions: ["retry"],
        next_step: null,
        created_at: "2026-10-12T00:00:00Z",
      },
      projection,
    );
    expect(item.title).toBe("聚合索引失败:成员代码覆盖不足");
    expect(item.summary).toContain("无可索引代码文件的成员：api, web");
    expect(item.summary).toContain("每成员至少 1 个可索引代码文件");
    expect(item.bootstrapInfo?.allowedActions).toEqual(["retry"]);
  });

  it("清单提取不到时保留指引文案；通用失败沿用既有标题", () => {
    const noList = logicalCodebaseBootstrapItem(
      {
        key: "lc:aggregate_index_active:aggregate_index_cov_0002",
        step: "aggregate_index_active",
        object_id: "aggregate_index_cov_0002",
        reason_code: "aggregate_index_member_coverage_failed",
        summary: "aggregate_index_failed:aggregate_index_member_coverage_failed: coverage lost",
        external_side_effect: "aggregate_index_cli",
        allowed_actions: ["retry"],
        next_step: null,
        created_at: "2026-10-12T00:00:00Z",
      },
      projection,
    );
    expect(noList.title).toBe("聚合索引失败:成员代码覆盖不足");
    expect(noList.summary).toContain("每成员至少 1 个可索引代码文件");

    const generic = logicalCodebaseBootstrapItem(
      {
        key: "lc:aggregate_index_active:aggregate_index_gen_0003",
        step: "aggregate_index_active",
        object_id: "aggregate_index_gen_0003",
        reason_code: "aggregate_index_failed",
        summary: "aggregate_index_failed:codegraph_init_failed: cli exploded",
        external_side_effect: "aggregate_index_cli",
        allowed_actions: ["retry"],
        next_step: null,
        created_at: "2026-10-12T00:00:00Z",
      },
      projection,
    );
    expect(generic.title).toBe("聚合索引首建失败，等待显式重试");
    expect(generic.summary).not.toContain("可索引代码文件");
  });
});

// P4-d（REQ-CRO-12/契约 A1 · REQ-UI37-19）：durable 待处理事项 → 收件箱
// 只读投影——7 类 kind（含 plan_final_confirm 聚合条）+ 未知未来 kind 全量
// 映射为 kind=coding_action/source=coding_attention，身份 [project,issue,key]。
describe("P4-d coding attention inbox projection", () => {
  const attention = (
    overrides: Partial<CodingAttentionItemDto> = {},
  ): CodingAttentionItemDto => ({
    key: "gate:attempt_0001:gate_0001",
    kind: "blocked",
    project_id: "project_0001",
    issue_id: "issue_0001",
    plan_id: null,
    attempt_id: "attempt_0001",
    logical_work_item_id: null,
    repository_name: null,
    reason_code: "upstream_source_unavailable",
    title: "执行被阻塞，需要处理",
    summary: "上游源不可读",
    occurred_at: "2026-10-15T08:00:00Z",
    target: { type: "coding_attempt", attempt_id: "attempt_0001" },
    focus: { gate_id: "gate_0001" },
    ...overrides,
  });

  it("projects every attention kind (incl. plan_final_confirm aggregate) without dropping entries", () => {
    const kinds = [
      "choice",
      "triage",
      "blocked",
      "stalled",
      "manual_recovery",
      "final_confirm",
      "plan_final_confirm",
    ];
    for (const kind of kinds) {
      const item = codingAttentionItem(attention({ kind, key: `k:${kind}` }));
      expect(item.kind).toBe("coding_action");
      expect(item.source).toBe("coding_attention");
      expect(item.title).toBe(attention({ kind }).title);
      expect(item.summary).toContain("上游源不可读");
      expect(item.createdAt).toBe("2026-10-15T08:00:00Z");
      expect(item.severity).toBe(2);
      expect(item.gate).toBeNull();
      expect(item.choice).toBeNull();
      expect(item.attentionInfo?.attentionKind).toBe(kind);
      expect(item.attentionInfo?.identity).toBe(
        codingAttentionIdentity(attention({ kind, key: `k:${kind}` })),
      );
    }
  });

  it("keeps unknown future kinds visible instead of filtering them out", () => {
    const item = codingAttentionItem(
      attention({ kind: "some_future_kind", key: "future:1" }),
    );
    expect(item.kind).toBe("coding_action");
    expect(item.attentionInfo?.attentionKind).toBe("some_future_kind");
  });

  it("uses full-scope identity [project,issue,key] and a non-session id prefix", () => {
    const item = attention({
      key: "gate:attempt_0001:gate_0001",
      project_id: "project_0001",
      issue_id: "issue_0001",
    });
    expect(codingAttentionIdentity(item)).toBe(
      JSON.stringify(["project_0001", "issue_0001", "gate:attempt_0001:gate_0001"]),
    );
    // 同 key 跨 project/issue 不合并。
    expect(codingAttentionIdentity(attention({ project_id: "project_0002" }))).not.toBe(
      codingAttentionIdentity(item),
    );
    expect(codingAttentionIdentity(attention({ issue_id: "issue_0002" }))).not.toBe(
      codingAttentionIdentity(item),
    );
    const projected = codingAttentionItem(item);
    // 收件箱 id 前缀不得被误读为 workspace session id（P4-e 走 attention 目标导航）。
    expect(cockpitInboxItemSessionId(projected.id)).toBeNull();
  });

  it("mirrors target/focus and optional identity fields for 去处理 navigation", () => {
    const planItem = codingAttentionItem(
      attention({
        kind: "plan_final_confirm",
        key: "plan-final:plan_0001:run_1",
        plan_id: "plan_0001",
        attempt_id: null,
        repository_name: "repo-a",
        logical_work_item_id: "wi_0009",
        reason_code: null,
        target: { type: "coding_plan", plan_id: "plan_0001" },
        focus: null,
      }),
    );
    expect(planItem.attentionInfo?.projectId).toBe("project_0001");
    expect(planItem.attentionInfo?.issueId).toBe("issue_0001");
    expect(planItem.attentionInfo?.planId).toBe("plan_0001");
    expect(planItem.attentionInfo?.attemptId).toBeNull();
    expect(planItem.attentionInfo?.repositoryName).toBe("repo-a");
    expect(planItem.attentionInfo?.logicalWorkItemId).toBe("wi_0009");
    expect(planItem.attentionInfo?.reasonCode).toBeNull();
    expect(planItem.attentionInfo?.target).toEqual({
      type: "coding_plan",
      plan_id: "plan_0001",
    });
    expect(planItem.attentionInfo?.focus).toBeNull();
    expect(planItem.summary).toContain("repo-a");
    expect(planItem.summary).toContain("wi_0009");

    const choiceItem = codingAttentionItem(
      attention({
        kind: "choice",
        key: "choice:attempt_0001:choice_0001",
        target: { type: "coding_attempt", attempt_id: "attempt_0001" },
        focus: { choice_id: "choice_0001" },
      }),
    );
    expect(choiceItem.attentionInfo?.focus).toEqual({ choice_id: "choice_0001" });
  });
});

// C5：多成员逻辑代码库 target 的收件箱展示身份（第三变体不得打印 undefined）。
describe("c1TargetLabel", () => {
  it("labels a single-repository target", () => {
    expect(
      c1TargetLabel({ kind: "single_repository", repository_id: "repo_0001" }),
    ).toBe("单仓 repo_0001");
  });

  it("labels a single-member logical-codebase target", () => {
    expect(
      c1TargetLabel({
        kind: "logical_codebase",
        logical_codebase_id: "lc_0001",
        logical_repository_id: "lr_0001",
      }),
    ).toBe("逻辑代码库 lc_0001/lr_0001");
  });

  it("labels a multi-member logical-codebase target with its member count", () => {
    expect(
      c1TargetLabel({
        kind: "logical_codebase_members",
        logical_codebase_id: "lc_0001",
        logical_repository_ids: ["lr_0001", "lr_0002", "lr_0003"],
      }),
    ).toBe("逻辑代码库 lc_0001[3 个成员]");
  });

  it("labels a multi-member target without members", () => {
    expect(
      c1TargetLabel({
        kind: "logical_codebase_members",
        logical_codebase_id: "lc_0001",
        logical_repository_ids: [],
      }),
    ).toBe("逻辑代码库 lc_0001[无成员]");
  });

  it("never renders undefined for a target with missing fields", () => {
    const label = c1TargetLabel({
      kind: "logical_codebase_members",
    } as unknown as C1WaitingItem["target"]);
    expect(label).not.toContain("undefined");
    expect(label).not.toContain("null");
    expect(label).toContain("—");
  });
});

// ── UI-3（待处理统一）：coding choice 规范身份与双来源去重 ──
describe("UI-3 canonical coding choice identity & dedupe", () => {
  const attentionChoiceDto = (
    overrides: Partial<CodingAttentionItemDto> = {},
  ): CodingAttentionItemDto => ({
    key: "choice:attempt_0001:choice_9",
    kind: "choice",
    project_id: "project_0001",
    issue_id: "issue_0001",
    plan_id: null,
    attempt_id: "attempt_0001",
    logical_work_item_id: null,
    repository_name: "repo-a",
    reason_code: null,
    title: "选择请求待作答",
    summary: "是否补齐测试",
    occurred_at: "2026-10-24T00:00:00Z",
    target: { type: "coding_attempt", attempt_id: "attempt_0001" },
    focus: { choice_id: "choice_9" },
    ...overrides,
  });

  const choiceCard = (
    overrides: Partial<NonNullable<CockpitInboxItem["choice"]>> = {},
  ): CockpitInboxItem => ({
    id: "session_001:choice:choice_9",
    kind: "choice",
    severity: 2,
    title: "选择请求待作答",
    summary: "是否补齐测试",
    triage: false,
    source: "choice",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: {
      sessionId: "session_001",
      choiceId: "choice_9",
      prompt: "是否补齐测试",
      expectedRunId: "run_1",
      questions: [],
      allowMultiple: false,
      allowFreeText: false,
      source: "coding",
      attemptAddress: {
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "attempt_0001",
      },
      status: "open",
      ...overrides,
    },
  });

  it("同一 choice 双来源只显示一次：作答卡保留，同身份目录行丢弃", () => {
    const card = choiceCard();
    const catalogRow = codingAttentionItem(attentionChoiceDto());
    expect(inboxCodingChoiceIdentity(card)).toBe(inboxCodingChoiceIdentity(catalogRow));
    const deduped = dedupeCodingChoiceInboxItems([card, catalogRow]);
    expect(deduped).toHaveLength(1);
    expect(deduped[0]).toBe(card);
  });

  it("不同 attempt 的同 choiceId 不误并（完整作用域归一）", () => {
    const card = choiceCard();
    const otherAttempt = codingAttentionItem(
      attentionChoiceDto({
        attempt_id: "attempt_0002",
        target: { type: "coding_attempt", attempt_id: "attempt_0002" },
        key: "choice:attempt_0002:choice_9",
      }),
    );
    expect(inboxCodingChoiceIdentity(card)).not.toBe(inboxCodingChoiceIdentity(otherAttempt));
    expect(dedupeCodingChoiceInboxItems([card, otherAttempt])).toHaveLength(2);
  });

  it("无作答卡时目录行原样保留；workspace choice 卡不参与合并", () => {
    const catalogRow = codingAttentionItem(attentionChoiceDto());
    expect(dedupeCodingChoiceInboxItems([catalogRow])).toEqual([catalogRow]);
    const workspaceCard = choiceCard({ source: "workspace", attemptAddress: undefined });
    expect(inboxCodingChoiceIdentity(workspaceCard)).toBeNull();
    expect(dedupeCodingChoiceInboxItems([workspaceCard, catalogRow])).toHaveLength(2);
  });
});
