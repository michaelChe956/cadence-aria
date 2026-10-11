import { describe, expect, it, vi } from "vitest";
import type { WsServerMessage } from "./workspace-ws-message-handler";
import { handleWorkspaceWsMessage, providerName } from "./workspace-ws-message-handler";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  installWorkspaceStoreTestHooks,
  makeContextBlockerArtifactPayload,
} from "../state/workspace-ws-store.test-utils";
import { selectCockpitInbox } from "../state/workspace-cockpit-projection";
import { linkedWorkspaceAmendmentSnapshotFixture } from "../components/coding-workspace/plan-repair-test-fixtures";
import {
  lcResumeOperationResultMessage,
  lcResumeSessionStateMessage,
} from "./useWorkspaceWs.test-utils";
import {
  lcFreshOperationFixture,
  lcResumeRecoveryFixture,
} from "../state/workspace-cockpit-projection.test-fixtures";

describe("workspace websocket provider parser", () => {
  it("accepts kimi_code as a workspace provider", () => {
    expect(providerName("kimi_code")).toBe("kimi_code");
  });

  it("rejects unknown provider names", () => {
    expect(providerName("unknown_provider")).toBeNull();
  });
});

// Bug C live 路径回归：后端时序为先 artifact_update(context_blocker) 再
// stage_change -> human_confirm，且 blocker 之后没有 session_state 全量重建，
// 因此 stage_change 现场构造的 gate_prompt 也必须注入 blocker 元数据并使用
// exploration_summary 内容，行为与 rebuild 路径（workspace-chat-rebuild.ts）一致。
describe("workspace websocket live stage_change gate prompt", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const startLiveWorkItemPlanSession = () => {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_live_blocker_gate",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "running",
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
      providers: { author: "claude_code", reviewer: null },
    });
  };

  const gatePromptEntry = () =>
    useWorkspaceStore.getState().chatEntries.find((entry) => entry.type === "gate_prompt");

  it("marks the live human_confirm gate as a context blocker gate", () => {
    const contextBlocker = makeContextBlockerArtifactPayload();
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "artifact_update", version: 3, context_blocker: contextBlocker } as WsServerMessage,
      handlerOptions(),
    );
    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    const gatePrompt = gatePromptEntry();
    expect(gatePrompt).toBeDefined();
    expect(gatePrompt).toMatchObject({
      type: "gate_prompt",
      content: contextBlocker.exploration_summary,
    });
    expect(gatePrompt?.metadata).toEqual(
      expect.objectContaining({
        gate_kind: "work_item_plan_context_blocker",
        allowed_actions: ["provide_context", "abort"],
      }),
    );
  });

  it("falls back to the waiting content when the blocker exploration summary is empty", () => {
    const contextBlocker = { ...makeContextBlockerArtifactPayload(), exploration_summary: "   " };
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "artifact_update", version: 4, context_blocker: contextBlocker } as WsServerMessage,
      handlerOptions(),
    );
    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    const gatePrompt = gatePromptEntry();
    expect(gatePrompt).toMatchObject({ content: "等待人工确认" });
    expect(gatePrompt?.metadata).toEqual(
      expect.objectContaining({ gate_kind: "work_item_plan_context_blocker" }),
    );
  });

  it("does not mark a live work item plan gate without a blocker artifact", () => {
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    const gatePrompt = gatePromptEntry();
    expect(gatePrompt).toBeDefined();
    expect(gatePrompt).toMatchObject({ content: "等待人工确认" });
    expect(gatePrompt?.metadata ?? {}).not.toHaveProperty("gate_kind");
  });

  // workspace-artifact-bug-triage：表驱动覆盖 story/design/work_item 三类 live gate，
  // blocker 标记只允许出现在 work_item_plan context_blocker 场景。
  it.each([["story"], ["design"], ["work_item"]])(
    "does not mark live %s human_confirm gates as context blocker gates",
    (workspaceType) => {
      useWorkspaceStore.getState().setSessionState({
        session_id: `session_live_${workspaceType}_gate`,
        workspace_type: workspaceType,
        stage: "running",
        session_status: "running",
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
        providers: { author: "claude_code", reviewer: null },
      });

      handleWorkspaceWsMessage(
        { type: "stage_change", stage: "human_confirm" } as WsServerMessage,
        handlerOptions(),
      );

      const gatePrompt = gatePromptEntry();
      expect(gatePrompt).toBeDefined();
      expect(gatePrompt).toMatchObject({ content: "等待人工确认" });
      expect(gatePrompt?.metadata ?? {}).not.toHaveProperty("gate_kind");
    },
  );

  // F-20：story/design 的 AuthorConfirm 即人工门——live stage_change 必须落门卡
  // 条目（否则仅在 session_state 全量重建时才可见，门开瞬间无决策面）。
  it.each([["story"], ["design"]])(
    "builds the live author_confirm gate prompt for a %s session",
    (workspaceType) => {
      useWorkspaceStore.getState().setSessionState({
        session_id: `session_live_${workspaceType}_author_gate`,
        workspace_type: workspaceType,
        stage: "author_run",
        session_status: "running",
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
        providers: { author: "claude_code", reviewer: null },
      });

      handleWorkspaceWsMessage(
        { type: "stage_change", stage: "author_confirm" } as WsServerMessage,
        handlerOptions(),
      );

      const gatePrompt = gatePromptEntry();
      expect(gatePrompt).toBeDefined();
      expect(gatePrompt).toMatchObject({ type: "gate_prompt" });
      expect(gatePrompt?.metadata).toEqual(
        expect.objectContaining({
          gate_identity: "stage:author_confirm",
          action_facade: "legacy",
        }),
      );
    },
  );

  it("does not build a live author_confirm gate prompt for work_item_plan sessions", () => {
    startLiveWorkItemPlanSession();

    handleWorkspaceWsMessage(
      { type: "stage_change", stage: "author_confirm" } as WsServerMessage,
      handlerOptions(),
    );

    expect(gatePromptEntry()).toBeUndefined();
  });
});

// usage 事件按 role 关联到对应 stream 气泡（usage-transparency 契约）
describe("workspace websocket usage event", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  it("maps usage execution_event onto the node's stream entry metadata", () => {
    useWorkspaceStore.getState().setSessionState({
      session_id: "session_usage_map",
      workspace_type: "story",
      stage: "running",
      session_status: "running",
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
      providers: { author: "claude_code", reviewer: null },
    });

    handleWorkspaceWsMessage(
      { type: "stream_chunk", node_id: "timeline_node_002", role: "author", content: "段落" } as WsServerMessage,
      handlerOptions(),
    );
    useWorkspaceStore.getState().flushBufferedStream("timeline_node_002");
    handleWorkspaceWsMessage(
      {
        type: "execution_event",
        event: {
          event_id: "usage_author",
          node_id: "timeline_node_002",
          agent: "author",
          kind: "usage",
          status: "completed",
          title: "Usage",
          output:
            '{"role":"author","input_tokens":89035,"output_tokens":8896,"cache_read_tokens":230976}',
        },
      } as unknown as WsServerMessage,
      handlerOptions(),
    );

    const entry = useWorkspaceStore
      .getState()
      .chatEntries.find((e) => e.id === "timeline_node_002:stream-active");
    expect(entry?.metadata?.usage).toMatchObject({
      input_tokens: 89035,
      output_tokens: 8896,
      cache_read_tokens: 230976,
    });
  });

  it("ignores malformed usage output without touching other entries", () => {
    const before = useWorkspaceStore.getState().chatEntries.length;
    handleWorkspaceWsMessage(
      {
        type: "execution_event",
        event: {
          event_id: "usage_bad",
          node_id: "timeline_node_002",
          kind: "usage",
          status: "completed",
          title: "Usage",
          output: "not-json",
        },
      } as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(useWorkspaceStore.getState().chatEntries.length).toBe(before);
  });
});


// F-27：choice 卡此前依赖单帧 choice_request 送达；degraded 丢帧后卡永不出现。
// 治本契约：session_state 帧顶层带 pending_choice_requests，收帧后按 choice id
// 对账补挂（与 live choice_request handler 同构、幂等；pending 清空不残留）。
describe("workspace websocket session_state pending choice reconciliation", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const pendingChoiceFixture = (overrides: Record<string, unknown> = {}) => ({
    id: "choice_pending_a",
    prompt: "继续方式？",
    options: [
      { id: "opt_0", label: "继续 author", description: "沿用当前 author 会话" },
      { id: "opt_1", label: "切换 provider" },
    ],
    allow_multiple: false,
    allow_free_text: true,
    questions: [
      {
        id: "default",
        prompt: "继续方式？",
        options: [
          { id: "opt_0", label: "继续 author" },
          { id: "opt_1", label: "切换 provider" },
        ],
        allow_multiple: false,
        allow_free_text: true,
      },
    ],
    source: "text_fallback",
    ...overrides,
  });

  const sessionStateMessage = (pendingChoiceRequests: unknown) => ({
    type: "session_state",
    session_id: "session_pending_choice_reconcile",
    workspace_type: "story",
    stage: "running",
    session_status: "running",
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
    providers: { author: "claude_code", reviewer: null },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    human_presentation_revisions: [],
    ...(pendingChoiceRequests === undefined
      ? {}
      : { pending_choice_requests: pendingChoiceRequests }),
  });

  const choiceEntries = () =>
    useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.type === "choice_request");

  it("mounts a missing pending choice entry isomorphic to the live choice_request handler", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(choiceEntries()).toEqual([
      expect.objectContaining({
        id: "choice_request:choice_pending_a",
        type: "choice_request",
        role: "system",
        content: "继续方式？",
        metadata: {
          request_id: "choice_pending_a",
          prompt: "继续方式？",
          options: pendingChoiceFixture().options,
          questions: pendingChoiceFixture().questions,
          allow_multiple: false,
          allow_free_text: true,
          source: "text_fallback",
        },
      }),
    ]);
  });

  it("keeps reconciliation idempotent for repeated session_state frames", () => {
    for (let round = 0; round < 2; round += 1) {
      handleWorkspaceWsMessage(
        sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
        handlerOptions(),
      );
    }

    expect(choiceEntries()).toHaveLength(1);
    expect(choiceEntries()[0]).toMatchObject({
      id: "choice_request:choice_pending_a",
    });
  });

  it("does not duplicate an entry already mounted by the live choice_request frame", () => {
    handleWorkspaceWsMessage(
      {
        type: "choice_request",
        ...pendingChoiceFixture(),
      } as unknown as WsServerMessage,
      handlerOptions(),
    );
    handleWorkspaceWsMessage(
      sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(choiceEntries()).toHaveLength(1);
    expect(choiceEntries()[0]).toMatchObject({
      id: "choice_request:choice_pending_a",
      content: "继续方式？",
    });
  });

  it("drops the stale card once the choice leaves the pending projection", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([pendingChoiceFixture()]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(choiceEntries()).toHaveLength(1);

    handleWorkspaceWsMessage(
      sessionStateMessage([]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(choiceEntries()).toHaveLength(0);

    handleWorkspaceWsMessage(
      sessionStateMessage(undefined) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(choiceEntries()).toHaveLength(0);
  });

  it("ignores malformed pending_choice_requests payloads without crashing", () => {
    expect(() =>
      handleWorkspaceWsMessage(
        sessionStateMessage({ not: "an array" }) as unknown as WsServerMessage,
        handlerOptions(),
      ),
    ).not.toThrow();
    expect(choiceEntries()).toHaveLength(0);
  });
});

// F-59（缺陷 2）：等待提示条数据源——session_state 的 pending_choice_requests
// 归一进 store.pendingChoiceRequests：role/created_at_ms 直通（提示条标注
// 发问方 + 已等待时长/901s 倒计时锚点）；旧载荷缺 created_at_ms 时按首见
// 时刻回退且跨帧稳定；pending 清空即收敛空。
describe("workspace websocket pending choice wait state projection", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const sessionStateMessage = (pendingChoiceRequests: unknown) => ({
    type: "session_state",
    session_id: "session_pending_choice_wait",
    workspace_type: "story",
    stage: "running",
    session_status: "running",
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
    providers: { author: "claude_code", reviewer: null },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    human_presentation_revisions: [],
    pending_choice_requests: pendingChoiceRequests,
  });

  const projected = () => useWorkspaceStore.getState().pendingChoiceRequests;

  it("projects role and created_at_ms from session_state pending_choice_requests", () => {
    const created_at_ms = Date.now() - 30_000;
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_wait_a",
          prompt: "修订口径需要裁定",
          options: [],
          allow_multiple: false,
          allow_free_text: false,
          questions: [],
          source: "ask_user_question",
          role: "author",
          created_at_ms,
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(projected()).toEqual([
      {
        id: "choice_wait_a",
        prompt: "修订口径需要裁定",
        role: "author",
        created_at_ms,
        first_seen_at_ms: expect.any(Number),
        expected_run_id: null,
        options: [],
        allow_multiple: false,
        allow_free_text: false,
        questions: [],
        source: "ask_user_question",
      },
    ]);
  });

  it("falls back to first-seen time when created_at_ms is absent and keeps it stable", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_wait_legacy",
          prompt: "旧载荷无时刻",
          source: "text_fallback",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    const firstSeen = projected()[0]?.first_seen_at_ms;
    expect(firstSeen).toEqual(expect.any(Number));
    expect(projected()[0]).toMatchObject({
      id: "choice_wait_legacy",
      created_at_ms: null,
      role: "author",
    });

    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_wait_legacy",
          prompt: "旧载荷无时刻",
          source: "text_fallback",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(projected()[0]?.first_seen_at_ms).toBe(firstSeen);
  });

  it("clears once the pending projection empties", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([{ id: "choice_wait_b", prompt: "p", source: "provider_choice" }]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(projected()).toHaveLength(1);

    handleWorkspaceWsMessage(sessionStateMessage([]) as unknown as WsServerMessage, handlerOptions());
    expect(projected()).toHaveLength(0);
  });

  it("defaults role to author for malformed entries that still carry an id", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([{ id: "choice_wait_c", prompt: "p", source: "provider_choice" }]) as unknown as WsServerMessage,
      handlerOptions(),
    );
    expect(projected()[0]).toMatchObject({ id: "choice_wait_c", role: "author" });
  });
});

// P0 1.3（REQ-WIGA-05）Task 11：驾驶舱就地作答数据源——pending 投影必须保
// 完整 options/questions/source/expected_run_id（严格校验 question id/数组；
// 数据缺失不得给 REST 提交面喂数）。
describe("workspace websocket pending choice answer source projection", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const sessionStateMessage = (pendingChoiceRequests: unknown) => ({
    type: "session_state",
    session_id: "session_pending_choice_answer",
    workspace_type: "work_item_plan",
    stage: "human_confirm",
    session_status: "waiting_for_human",
    flow_kind: "single_candidate",
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
    providers: { author: "claude_code", reviewer: null },
    timeline_nodes: [],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    human_presentation_revisions: [],
    pending_choice_requests: pendingChoiceRequests,
  });

  const projected = () => useWorkspaceStore.getState().pendingChoiceRequests;

  it("keeps the full options/questions/source/expected_run_id for in-place answering", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_answer_a",
          prompt: "拆分方案确认",
          options: [{ id: "opt_0", label: "继续", description: "desc" }],
          allow_multiple: true,
          allow_free_text: false,
          questions: [
            {
              id: "q-1",
              prompt: "是否包含集成测试",
              options: [
                { id: "yes", label: "包含" },
                { id: "no", label: "不包含" },
              ],
              allow_multiple: false,
              allow_free_text: false,
            },
          ],
          source: "ask_user_question",
          role: "author",
          expected_run_id: "run-9",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(projected()[0]).toMatchObject({
      id: "choice_answer_a",
      expected_run_id: "run-9",
      source: "ask_user_question",
      allow_multiple: true,
      options: [{ id: "opt_0", label: "继续", description: "desc" }],
      questions: [
        {
          id: "q-1",
          prompt: "是否包含集成测试",
          options: [
            { id: "yes", label: "包含", description: null },
            { id: "no", label: "不包含", description: null },
          ],
          allow_multiple: false,
          allow_free_text: false,
        },
      ],
    });
  });

  it("drops malformed question structures instead of feeding the REST submit face", () => {
    handleWorkspaceWsMessage(
      sessionStateMessage([
        {
          id: "choice_answer_bad",
          prompt: "畸形题结构",
          questions: [{ prompt: "缺 id 的题" }],
          source: "ask_user_question",
        },
      ]) as unknown as WsServerMessage,
      handlerOptions(),
    );

    expect(projected()[0]).toMatchObject({ id: "choice_answer_bad" });
    expect(projected()[0].questions).toEqual([]);
  });
});

// C2（Task 6.2 / anchor_t20）：typed LcResumeRecoveryOperationResult 帧对账。
// handler 按 recovery_id 与 store 当前权威恢复对账（被新漂移替换/权威清空
// 后的迟到帧静默丢弃）；delivery=replayed 只读回放、不重复执行；running/
// resolved/prepare 失败三态以幂等 typed 回执呈现；operation_state 直取
// wire durable 事实，不从错误字符串推权限、不伪 running、不触发
// retry/advance（与既有 advance 重放/retry_interrupted_run 通道分离）。
describe("anchor_t20 fresh sender and typed operation result", () => {
  installWorkspaceStoreTestHooks();

  const handlerOptions = () => ({
    invalidatedPreStageNodeIds: new Set<string>(),
    scheduleFlush: vi.fn(),
    streamFlushTimeouts: {},
  });

  const receipts = () =>
    useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) =>
        entry.id.startsWith("lc_resume_recovery_operation_result:"),
      );

  const waitingRecovery = () =>
    lcResumeRecoveryFixture({
      state: "waiting",
      last_operation: lcFreshOperationFixture({ state: "not_started" }),
    });

  const receive = (message: unknown) =>
    handleWorkspaceWsMessage(message as unknown as WsServerMessage, handlerOptions());

  it("anchor_t20_fresh_sender_and_typed_operation_result", () => {
    // 权威首帧：waiting 恢复经真实 C1 投影路径落库。
    receive(
      lcResumeSessionStateMessage("session_rec", waitingRecovery()),
    );
    expect(useWorkspaceStore.getState().lcResumeRecovery?.recovery_id).toBe("rec-1");

    // accepted/reserved：typed 回执落幂等条目，逐字段镜像 wire。
    receive(lcResumeOperationResultMessage("rec-1", "op-1", "accepted", "reserved"));
    expect(receipts()).toHaveLength(1);
    expect(receipts()[0]).toMatchObject({
      id: "lc_resume_recovery_operation_result:rec-1:op-1:accepted:reserved",
      type: "start_generation",
      role: "system",
      metadata: {
        recovery_id: "rec-1",
        operation_id: "op-1",
        delivery: "accepted",
        operation_state: "reserved",
        fresh_node_id: null,
        reason_code: null,
      },
    });

    // 已知协议成员：不得落 UNRECOGNIZED_EVENT 诊断（default 分支不接手）。
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(0);
    // 不伪造 generic 运行事实：providerStatus 不伪 running、无 error 面。
    expect(useWorkspaceStore.getState().providerStatus).toBe("starting");
    expect(useWorkspaceStore.getState().error).toBeNull();
  });

  it("replays the same operation without duplicating execution or receipts", () => {
    receive(lcResumeSessionStateMessage("session_rec", waitingRecovery()));

    const reserved = lcResumeOperationResultMessage("rec-1", "op-1", "accepted", "reserved");
    for (let round = 0; round < 2; round += 1) {
      receive(reserved);
    }
    // 同帧重放：幂等 upsert，回执不重复、不复活执行。
    expect(receipts()).toHaveLength(1);

    // replayed/running：只读回放按状态呈现，不产生重复执行副作用。
    receive(
      lcResumeOperationResultMessage("rec-1", "op-1", "replayed", "running", {
        fresh_node_id: "node-2",
      }),
    );
    expect(receipts()).toHaveLength(2);
    expect(receipts().at(-1)?.metadata).toMatchObject({
      delivery: "replayed",
      operation_state: "running",
      fresh_node_id: "node-2",
    });
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(0);
  });

  it("presents running, resolved and prepare-failed states while only session_state moves authority", () => {
    // fresh_running 活跃态：恢复卡在场，旧 generic 出口由恢复卡替代（C6.1
    // 投影边界回归：本 handler 不复活 generic 伪事实）。
    receive(
      lcResumeSessionStateMessage(
        "session_rec",
        lcResumeRecoveryFixture({
          state: "fresh_running",
          last_operation: lcFreshOperationFixture({ state: "running" }),
        }),
      ),
    );
    receive(lcResumeOperationResultMessage("rec-1", "op-1", "replayed", "running"));
    let inbox = selectCockpitInbox(useWorkspaceStore.getState());
    expect(inbox.some((item) => item.kind === "lc_resume_recovery")).toBe(true);
    expect(receipts().at(-1)?.metadata).toMatchObject({ operation_state: "running" });

    // resolved：同 ID 回放 completed；恢复卡消失、generic stopped 卡不被
    // 永久屏蔽（后续正常动作恢复）。
    receive(
      lcResumeSessionStateMessage(
        "session_rec",
        lcResumeRecoveryFixture({
          state: "resolved",
          last_operation: lcFreshOperationFixture({
            state: "completed",
            fresh_node: "node-2",
          }),
        }),
      ),
    );
    receive(
      lcResumeOperationResultMessage("rec-1", "op-1", "replayed", "completed", {
        fresh_node_id: "node-2",
      }),
    );
    inbox = selectCockpitInbox(useWorkspaceStore.getState());
    expect(inbox.some((item) => item.kind === "lc_resume_recovery")).toBe(false);
    expect(inbox.some((item) => item.kind === "stopped")).toBe(true);
    expect(receipts().at(-1)?.metadata).toMatchObject({
      operation_state: "completed",
      fresh_node_id: "node-2",
    });

    // prepare 失败：回 waiting 不删 ID；回执呈现 failed＋reason，typed 帧
    // 不改写 store 权威恢复（仍由 session_state 全量替换承载）。
    receive(lcResumeSessionStateMessage("session_rec", waitingRecovery()));
    receive(
      lcResumeOperationResultMessage("rec-1", "op-1", "replayed", "failed", {
        reason_code: "prepare_failed",
      }),
    );
    expect(useWorkspaceStore.getState().lcResumeRecovery).toMatchObject({
      recovery_id: "rec-1",
      state: "waiting",
    });
    expect(receipts().at(-1)?.metadata).toMatchObject({
      operation_state: "failed",
      reason_code: "prepare_failed",
    });
    expect(
      selectCockpitInbox(useWorkspaceStore.getState()).some(
        (item) => item.kind === "lc_resume_recovery",
      ),
    ).toBe(true);
  });

  it("keeps unknown operation state non-optimistic without generic fabrication", () => {
    receive(lcResumeSessionStateMessage("session_rec", waitingRecovery()));
    receive(lcResumeOperationResultMessage("rec-1", "op-9", "accepted", "unknown"));

    expect(receipts()).toHaveLength(1);
    expect(receipts()[0].metadata).toMatchObject({
      operation_id: "op-9",
      operation_state: "unknown",
    });
    // unknown 不乐观标成功/失败：不伪 running、无 error/protocol_error 面。
    expect(useWorkspaceStore.getState().providerStatus).toBe("starting");
    expect(useWorkspaceStore.getState().error).toBeNull();
    expect(useWorkspaceStore.getState().protocolError).toBeNull();
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(0);
  });

  it("drops result frames for a superseded or absent recovery without side effects", () => {
    receive(lcResumeSessionStateMessage("session_rec", waitingRecovery()));
    // 旧 recovery_id（已被新漂移替换）：迟到帧静默丢弃。
    receive(lcResumeOperationResultMessage("rec-old", "op-1", "replayed", "running"));
    expect(receipts()).toHaveLength(0);
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(0);

    // store 无恢复投影（权威帧清空后）：结果帧不复活恢复事实。
    receive(lcResumeSessionStateMessage("session_rec", null));
    receive(lcResumeOperationResultMessage("rec-1", "op-1", "replayed", "completed"));
    expect(useWorkspaceStore.getState().lcResumeRecovery).toBeNull();
    expect(receipts()).toHaveLength(0);
    expect(useWorkspaceStore.getState().protocolDiagnostics).toHaveLength(0);
  });
});
