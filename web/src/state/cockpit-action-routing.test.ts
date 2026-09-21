import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  actionFacadeForFlowKind,
  classifyProtocolError,
  createCockpitActionFacade,
  type ProtocolErrorDisposition,
} from "./cockpit-action-routing";
import type { WorkspaceWsState } from "./workspace-ws-store-types";
import { useWorkspaceStore } from "./workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "./workspace-ws-store.test-utils";

function stateWithAdvance(commandId: string): Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands"> {
  return {
    humanGateTurn: null,
    advanceCommands: {
      [commandId]: {
        command_id: commandId,
        status: "pending",
        code: null,
        reason: null,
        attempt_id: null,
        workspace_entry: null,
        inlineError: null,
      },
    },
  };
}

function stateWithGate(turnId: string): Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands"> {
  return {
    humanGateTurn: {
      turn_id: turnId,
      command_id: "command-1",
      remaining_budget: 1,
      status: "open",
      artifact_ref: null,
      failure_class: null,
      failure_message: null,
      opened_at: "2026-09-14T00:00:00.000Z",
      inlineError: null,
    },
    advanceCommands: {},
  };
}

function emptyWorkspaceState(): Pick<WorkspaceWsState, "humanGateTurn" | "advanceCommands"> {
  return { humanGateTurn: null, advanceCommands: {} };
}

describe("classifyProtocolError", () => {
  it("routes replay rejection to its advance record instead of creating hard error", () => {
    expect(
      classifyProtocolError(
        "ADVANCE_REPLAY_NOT_READY",
        { command_id: "advance-1" },
        stateWithAdvance("advance-1"),
      ),
    ).toEqual({ kind: "advance", commandId: "advance-1" } satisfies ProtocolErrorDisposition);
  });

  it("routes an owned gate rejection to its gate record", () => {
    expect(
      classifyProtocolError(
        "INVALID_HUMAN_CONFIRM_ACTION",
        { turn_id: "turn-1" },
        stateWithGate("turn-1"),
      ),
    ).toEqual({ kind: "gate", turnId: "turn-1" } satisfies ProtocolErrorDisposition);
  });

  it("keeps an unowned protocol error as hard error", () => {
    expect(classifyProtocolError("UNEXPECTED_FRAME", {}, emptyWorkspaceState())).toEqual({
      kind: "hard_error",
    } satisfies ProtocolErrorDisposition);
  });
});

describe("cockpit gate action facade", () => {
  installWorkspaceStoreTestHooks();
  it("classifies every single-candidate gate, including a snapshot gate, as typed", () => {
    expect(actionFacadeForFlowKind("single_candidate")).toBe("typed");
    expect(actionFacadeForFlowKind("legacy")).toBe("legacy");
  });

  beforeEach(() => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
      humanGateClosure: null,
      humanGateSnapshot: null,
    });
  });
  it("dispatches typed snapshot feedback with a freshly generated command id", () => {
    const sendHumanGateFeedback = vi.fn(
      (_feedback: string, _commandId?: string) => true,
    );
    const actions = createCockpitActionFacade({
      getState: useWorkspaceStore.getState,
      flowKind: "single_candidate",
      commandId: null,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback,
      sendAdvance: vi.fn(() => true),
      adoptReview: vi.fn(),
    });

    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).toHaveBeenCalledTimes(1);
    const [feedback, commandId] = sendHumanGateFeedback.mock.calls[0];
    expect(feedback).toBe("请补齐边界");
    // 协议允许客户端自生成 command_id：断言拿到了新的 id，而不是拒发或透传空值。
    expect(commandId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
  });

  it("reuses the live turn command id instead of regenerating one", () => {
    const sendHumanGateFeedback = vi.fn(
      (_feedback: string, _commandId?: string) => true,
    );
    const actions = createCockpitActionFacade({
      flowKind: "single_candidate",
      getState: useWorkspaceStore.getState,
      commandId: "cmd_1",
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback,
      sendAdvance: vi.fn(() => true),
      adoptReview: vi.fn(),
    });
    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).toHaveBeenCalledWith("请补齐边界", "cmd_1");
  });

  it("keeps legacy flow feedback off the typed websocket helper", () => {
    const sendHumanGateFeedback = vi.fn(
      (_feedback: string, _commandId?: string) => true,
    );
    const actions = createCockpitActionFacade({
      flowKind: "legacy",
      commandId: "cmd_1",
      getState: useWorkspaceStore.getState,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback,
      sendAdvance: vi.fn(() => true),
      adoptReview: vi.fn(),
    });
    actions.feedback("请补齐边界");

    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
  });

  // F-31（v37 复验 #2）：待处理抽屉动作面与主区门卡对齐——confirmReview 复用
  // confirm 通道携带 with_review=true（HTTP confirm 端点语义）；confirm() 维持
  // 定稿缺省（不带该字段），阻断判据与 confirm 同源。
  it("forwards with_review through the shared confirm channel for the author gate (F-31)", () => {
    useWorkspaceStore.setState({
      stage: "author_confirm",
      workspaceType: "story",
      sessionStatus: "waiting_for_human",
      flowKind: "legacy",
    });
    const sendConfirm = vi.fn((_withReview?: boolean) => true);
    const facade = createCockpitActionFacade({
      flowKind: "legacy",
      commandId: null,
      getState: useWorkspaceStore.getState,
      sendConfirm,
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback: vi.fn(() => true),
      sendAdvance: vi.fn(() => true),
      adoptReview: vi.fn(),
    });

    expect(facade.confirm()).toBe(true);
    expect(facade.confirmReview()).toBe(true);
    expect(sendConfirm.mock.calls[0]).toHaveLength(0);
    expect(sendConfirm.mock.calls[1]).toEqual([true]);
  });

  // v40 复验 #3：author 门「采纳 Review 意见」经门面 adoptReview——纯客户端
  // 通道（预填修订反馈+切视图），无 WS/HTTP 帧；阻断判据与 confirm 同源
  // （门收口后不再预填修订）。
  it("hands adoptReview to the client-side channel and blocks it on a closed gate (v40 #3)", () => {
    useWorkspaceStore.setState({
      stage: "author_confirm",
      workspaceType: "story",
      sessionStatus: "waiting_for_human",
      flowKind: "legacy",
    });
    const adoptReview = vi.fn();
    const facade = createCockpitActionFacade({
      flowKind: "legacy",
      commandId: null,
      getState: useWorkspaceStore.getState,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback: vi.fn(() => true),
      sendAdvance: vi.fn(() => true),
      adoptReview,
    });

    facade.adoptReview();
    expect(adoptReview).toHaveBeenCalledTimes(1);

    // HTTP confirm 200 乐观态（sessionStatus=confirmed）即门收口：采纳预填
    // 同样阻断，不得把修订反馈预填进已定稿会话。
    useWorkspaceStore.setState({ sessionStatus: "confirmed" });
    facade.adoptReview();
    expect(adoptReview).toHaveBeenCalledTimes(1);
  });

  it("sends a manual advance exactly once through the same facade", () => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      humanGateClosure: { decision: "confirm", stage: "human_confirm" },
    });
    const sendAdvance = vi.fn<(commandId?: string) => boolean>(() => true);
    createCockpitActionFacade({
      flowKind: "legacy",
      commandId: null,
      getState: useWorkspaceStore.getState,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
      sendHumanGateFeedback: vi.fn(() => true),
      sendAdvance,
      adoptReview: vi.fn(),
    }).advance();

    expect(sendAdvance).toHaveBeenCalledTimes(1);
    expect(sendAdvance.mock.calls[0]?.[0]).toMatch(/^[0-9a-f-]{36}$/);
  });
  it("allows manual advance after the engine has confirmed a completed single-candidate gate", () => {
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "completed",
      sessionStatus: "confirmed",
      humanGateClosure: null,
    });
    const sendAdvance = vi.fn<(commandId?: string) => boolean>(() => true);

    expect(
      createCockpitActionFacade({
        flowKind: "single_candidate",
        commandId: null,
        getState: useWorkspaceStore.getState,
        sendConfirm: vi.fn(() => true),
      sendAbandonGate: vi.fn(() => true),
        sendHumanGateFeedback: vi.fn(() => true),
        sendAdvance,
        adoptReview: vi.fn(),
      }).advance(),
    ).toBe(true);

    expect(sendAdvance).toHaveBeenCalledOnce();
  });

  it("re-reads actionability before every action so a stale snapshot cannot send", () => {
    const sendConfirm = vi.fn(() => true);
    const sendAbandonGate = vi.fn(() => true);
    const sendHumanGateFeedback = vi.fn(() => true);
    const sendAdvance = vi.fn(() => true);
    const actions = createCockpitActionFacade({
      flowKind: "single_candidate",
      commandId: "cmd_1",
      getState: useWorkspaceStore.getState,
      sendConfirm,
      sendHumanGateFeedback,
      sendAbandonGate,
      sendAdvance,
      adoptReview: vi.fn(),
    });
    useWorkspaceStore.setState({
      stage: "running",
      flowKind: "single_candidate",
      singleCandidatePhase: "generate",
      humanGateSnapshot: {
        findings: [],
        repeated_fingerprints: [],
        attempts_used: 1,
        manual_repairs_remaining: 1,
        trigger: "verification_new_findings",
        resumable: true,
      },
    });

    expect(actions.confirm()).toBe(false);
    expect(actions.feedback("请补齐边界")).toBe(false);
    expect(actions.terminate()).toBe(false);
    expect(actions.advance()).toBe(false);
    expect(sendConfirm).not.toHaveBeenCalled();
    expect(sendAbandonGate).not.toHaveBeenCalled();
    expect(sendHumanGateFeedback).not.toHaveBeenCalled();
    expect(sendAdvance).not.toHaveBeenCalled();
  });
  // k3 P2-2：AuthorConfirm 矩阵只放行 Abort/AbandonHumanGate——即便 HTTP confirm
  // 乐观 confirmed 态也不发 advance（否则必回 ADVANCE_STAGE_INVALID 红条）。
  it.each(["story", "design", "work_item_plan"] as const)(
    "never sends advance from %s author_confirm even after an optimistic confirm",
    (workspaceType) => {
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType,
        flowKind: "legacy",
        sessionStatus: "confirmed",
        humanGateTurn: null,
        humanGateSnapshot: null,
        humanGateClosure: null,
      });
      const sendAdvance = vi.fn<(commandId?: string) => boolean>(() => true);

      expect(
        createCockpitActionFacade({
          flowKind: "legacy",
          commandId: null,
          getState: useWorkspaceStore.getState,
          sendConfirm: vi.fn(() => true),
          sendAbandonGate: vi.fn(() => true),
          sendHumanGateFeedback: vi.fn(() => true),
          sendAdvance,
          adoptReview: vi.fn(),
        }).advance(),
      ).toBe(false);

      expect(sendAdvance).not.toHaveBeenCalled();
    },
  );

  // F-21（v28 监控 0449）：plan 会话停在 human_confirm 的非终审门（context
  // blocker=prepare 相位/author validate 失败=generate 相位/旧会话缺相位），
  // 终止此前被 phase_mismatch 静默拦截——点击零 WS 出站、无任何可见反馈。
  // 矩阵已放行 AbandonHumanGate 到 SC human_confirm（59d59760），引擎
  // close_human_gate 只校验 stage+flow_kind——terminate 必须放行；
  // confirm/feedback 维持相位纪律不变。
  it.each(["prepare", "generate", null] as const)(
    "sends typed abandon exactly once for a plan gate parked at human_confirm with phase %s (F-21)",
    (singleCandidatePhase) => {
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        stage: "human_confirm",
        flowKind: "single_candidate",
        singleCandidatePhase,
        sessionStatus: "waiting_for_human",
        humanGateTurn: null,
        humanGateClosure: null,
      });
      const sendAbandonGate = vi.fn((_commandId: string) => true);
      const sendConfirm = vi.fn(() => true);
      const sendHumanGateFeedback = vi.fn(() => true);
      const actions = createCockpitActionFacade({
        flowKind: "single_candidate",
        commandId: null,
        getState: useWorkspaceStore.getState,
        sendConfirm,
        sendAbandonGate,
        sendHumanGateFeedback,
        sendAdvance: vi.fn(() => true),
        adoptReview: vi.fn(),
      });

      expect(actions.confirm()).toBe(false);
      expect(actions.feedback("请补充上下文")).toBe(false);
      expect(actions.terminate()).toBe(true);

      expect(sendAbandonGate).toHaveBeenCalledTimes(1);
      expect(sendAbandonGate.mock.calls[0]?.[0]).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
      );
      expect(sendConfirm).not.toHaveBeenCalled();
      expect(sendHumanGateFeedback).not.toHaveBeenCalled();
    },
  );

  it("reuses the live turn command id for a terminate-only plan gate (F-21)", () => {
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "prepare",
      humanGateClosure: null,
    });
    useWorkspaceStore.getState().applyHumanGateTurnOpen("turn_1", "cmd_live", 1);
    const sendAbandonGate = vi.fn(() => true);

    createCockpitActionFacade({
      flowKind: "single_candidate",
      commandId: "cmd_live",
      getState: useWorkspaceStore.getState,
      sendConfirm: vi.fn(() => true),
      sendAbandonGate,
      sendHumanGateFeedback: vi.fn(() => true),
      sendAdvance: vi.fn(() => true),
      adoptReview: vi.fn(),
    }).terminate();

    expect(sendAbandonGate).toHaveBeenCalledWith("cmd_live");
  });
});
