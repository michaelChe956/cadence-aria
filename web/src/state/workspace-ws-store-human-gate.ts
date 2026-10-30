import type {
  HumanGateTurnRejectionState,
  HumanGateTurnState,
  WorkspaceSessionStatePayload,
  WorkspaceWsActions,
  WorkspaceWsState,
} from "./workspace-ws-store-types";

type GateSet = (
  partial:
    | Partial<WorkspaceWsState>
    | ((prev: WorkspaceWsState) => Partial<WorkspaceWsState>),
) => void;

/** 本模块负责的人工门动作子集（其余动作仍在 workspace-ws-store.ts）。 */
export type HumanGateActions = Pick<
  WorkspaceWsActions,
  | "applyHumanGateTurnOpen"
  | "applyHumanGateTurnCompleted"
  | "applyHumanGateTurnFailed"
  | "applyHumanGateTurnRejected"
  | "applyHumanGateBusy"
  | "applyHumanGateClosed"
  | "recordGateFeedbackSubmission"
  | "resolveGateEntry"
  | "recordGateLocalAction"
  | "applyGateExclusionFailure"
  | "resolveGateExclusion"
>;

/** RFC3339 时间戳可比性判据：两者都可解析且 a 严格早于 b（缺省/畸形=不可比）。 */
function recordedBefore(a: string | undefined, b: string | null): boolean {
  if (!a || b === null) {
    return false;
  }
  const aMs = Date.parse(a);
  const bMs = Date.parse(b);
  return !Number.isNaN(aMs) && !Number.isNaN(bMs) && aMs < bMs;
}

/**
 * REQ-WSC-14：durable「最近一次门 turn 拒收」→ 重建终态 turn（刷新后门卡仍有
 * 四要素：被拒反馈原文 / 可读原因 / 下一步动作 / 逐字诊断）。
 *
 * 只在门仍开（durable snapshot 在场）且没有更新 live turn 时重建——不覆盖
 * 已经由 `human_gate_turn_failed` / `human_gate_turn_rejected` 帧建立的 turn。
 *
 * P3-6（在途 turn 不覆盖判据，两个 guard 并用）：
 * 1. 同会话已有更新的在途 turn——活 turn（open/busy）开出晚于拒收落盘，或
 *    turn_id 已是另一回合（既有判据）——旧拒收态不得覆盖它（owner 已回复/
 *    已重新评审，卡面不得再显示「你上一条回复被拒」）。
 * 2. 拒收落盘早于当前门 episode 的开门时间（同会话正证据：快照指纹更换=门
 *    重开/复评重建）——旧 episode 的拒收不得重建进新门；刷新（跨会话）无
 *    开门正证据时维持重建（REQ-WSC-14 刷新可读不回退）。
 */
export function durableRejectionTurnFromSession(
  state: Pick<WorkspaceSessionStatePayload, "human_gate_snapshot" | "human_gate_turn_rejection">,
  prev: Pick<WorkspaceWsState, "humanGateTurn">,
  sameSession: boolean,
  gateEpisode: { changed: boolean; openedAt: string | null },
): HumanGateTurnState | null {
  const rejection = state.human_gate_turn_rejection;
  const snapshot = state.human_gate_snapshot ?? null;
  if (!rejection || !rejection.turn_id || snapshot === null) {
    return null;
  }
  // P3-6 判据 2：拒收早于本门 episode 开门（changed 时 openedAt 即本客户端
  // 发现新门的时刻）→ 旧拒收不属于这个门，不重建。
  if (sameSession && gateEpisode.changed && recordedBefore(rejection.recorded_at, gateEpisode.openedAt)) {
    return null;
  }
  const rejectionState: HumanGateTurnRejectionState = {
    rejected_feedback: rejection.rejected_feedback ?? "",
    rejection_reason: rejection.rejection_reason ?? "",
    rejection_next_actions: rejection.rejection_next_actions ?? [],
    rejection_diagnostics: rejection.rejection_diagnostics ?? [],
  };
  const existing = sameSession ? prev.humanGateTurn : null;
  if (existing) {
    // 同 turn 缺 rejection → 补齐；其它情况（更新的 turn / 已有四要素）不动。
    if (existing.turn_id !== rejection.turn_id || existing.rejection) {
      return null;
    }
    // P3-6 判据 1：同 id 但活 turn 开出晚于拒收落盘——这是更新的在途回合
    //（重试/重评已开新回合），不得用旧拒收态把它打回 failed。
    if (
      (existing.status === "open" || existing.status === "busy") &&
      recordedBefore(rejection.recorded_at, existing.opened_at)
    ) {
      return null;
    }
    return {
      ...existing,
      status: "failed",
      failure_class: "validation_reject",
      failure_message: rejectionState.rejection_reason,
      rejection: rejectionState,
    };
  }
  return {
    turn_id: rejection.turn_id,
    command_id: null,
    remaining_budget: snapshot.manual_repairs_remaining ?? 0,
    status: "failed",
    artifact_ref: null,
    failure_class: "validation_reject",
    failure_message: rejectionState.rejection_reason,
    rejection: rejectionState,
    opened_at: rejection.recorded_at ?? new Date().toISOString(),
    inlineError: null,
  };
}

/**
 * REQ-WSC-13/14：人工门动作族（turn open / completed / failed / rejected / busy /
 * closed ＋ 门卡条目记录/收口 ＋ P2 排除失败归因三动作）。
 *
 * 从 `workspace-ws-store.ts` 拆出的原因是单文件行数门禁（该文件已在祖父条款上限）；
 * turn 动作语义与原实现逐字一致：同 `turn_id` 幂等，跨 turn 一律不写。
 */
export function humanGateActions(set: GateSet): HumanGateActions {
  return {
    applyHumanGateTurnOpen: (turnId, commandId, remainingBudget) =>
      set((prev) => {
        if (prev.humanGateTurn?.turn_id === turnId) {
          // 引擎同 turn_id 重放（decisions.rs Replayed）→ 幂等消费：
          // 不新建条目、不重算预算、不刷新 opened_at。
          return {};
        }
        return {
          humanGateTurn: {
            turn_id: turnId,
            command_id: commandId,
            remaining_budget: remainingBudget,
            status: "open",
            artifact_ref: null,
            failure_class: null,
            failure_message: null,
            opened_at: new Date().toISOString(),
            inlineError: null,
            // F-49 A8：记下本 turn 开出时的门快照身份——门以新快照重建（修订成功后
            // Evaluate 重置预算）后，预算以新快照为准，见 selectGateProjection。
            opened_snapshot_identity: prev.snapshotGateIdentity,
          },
          humanGateClosure: null,
        };
      }),

    applyHumanGateTurnCompleted: (turnId, artifactRef) =>
      set((prev) => {
        if (prev.humanGateTurn?.turn_id !== turnId) {
          return {};
        }
        return {
          humanGateTurn: {
            ...prev.humanGateTurn,
            status: "awaiting_confirm",
            artifact_ref: artifactRef,
          },
        };
      }),

    applyHumanGateTurnFailed: (turnId, failureClass, message) =>
      set((prev) => {
        if (prev.humanGateTurn?.turn_id !== turnId) {
          return {};
        }
        return {
          humanGateTurn: {
            ...prev.humanGateTurn,
            status: "failed",
            failure_class: failureClass,
            failure_message: message,
          },
        };
      }),

    // REQ-WSC-14：终态拒收的可操作事实——服务端已 durable 落盘，此处只投影。
    applyHumanGateTurnRejected: (turnId, rejection) =>
      set((prev) => {
        if (prev.humanGateTurn?.turn_id !== turnId) {
          return {};
        }
        return {
          humanGateTurn: {
            ...prev.humanGateTurn,
            status: "failed",
            failure_class: "validation_reject",
            failure_message: rejection.rejection_reason,
            rejection,
          },
        };
      }),

    applyHumanGateBusy: (turnId) =>
      set((prev) => {
        if (prev.humanGateTurn?.turn_id !== turnId) {
          return {};
        }
        return { humanGateTurn: { ...prev.humanGateTurn, status: "busy" } };
      }),

    applyHumanGateClosed: (decision, stage) =>
      set({
        humanGateClosure: { decision, stage },
        // P2（REQ-WSC-15 归因扩展）：门已收口——归因线索与失败事实随之失效
        //（卡面只读留档，不再呈现排除失败）。
        lastGateLocalAction: null,
        gateExclusionFailure: null,
      }),

    // F-49 A4/B5：记下本卡提交的反馈文本——提交成功后清空输入（A4）与旧轮留档摘要
    // （B5）都以此为准，不在渲染层重读输入框。（自 workspace-ws-store.ts 迁入，
    // 行数门禁：该文件超限，人工门动作族统一归本模块。）
    recordGateFeedbackSubmission: (entryId, feedback) =>
      set((prev) => {
        const index = prev.chatEntries.findIndex((entry) => entry.id === entryId);
        if (index === -1) {
          return {};
        }
        const next = [...prev.chatEntries];
        next[index] = {
          ...next[index],
          metadata: { ...next[index].metadata, submitted_feedback: feedback },
        };
        return { chatEntries: next };
      }),

    // F-49 A3：关门决定必须收口「全部」未决门卡。此前只 resolve 倒序命中的第一张：
    // 门内轮次切换后残留的旧卡永不被收口（human_gate_closed 只在 approve/abandon
    // 时发出），关门后对话流仍留可点旧卡。（自 workspace-ws-store.ts 迁入。）
    resolveGateEntry: (resolution) =>
      set((prev) => {
        let changed = false;
        const entries = prev.chatEntries.map((entry) => {
          if (entry.type !== "gate_prompt" || entry.resolved === true) {
            return entry;
          }
          changed = true;
          return { ...entry, resolved: true, resolution };
        });
        return changed ? { chatEntries: entries } : {};
      }),

    // P2（REQ-WSC-15 归因扩展）：记录最近本地门动作（error 帧归因线索）——
    // 新动作同时清掉旧失败事实（重试排除时界面回到即时反馈形态，不残留上
    // 一次失败的红字）。
    recordGateLocalAction: (gateKey, kind) =>
      set({
        lastGateLocalAction: { kind, gateKey, at: Date.now() },
        gateExclusionFailure: null,
      }),

    // P2：归因成立的排除失败（服务端回 WsOutMessage::Error）——落失败事实供
    // 门卡撤销乐观「已排除」并就地呈现原因；动作记录一并消费（同一错误只
    // 归因一次，后续无关 error 帧不得重复撤销）。
    applyGateExclusionFailure: (failure) =>
      set((prev) => ({
        gateExclusionFailure: { ...failure, at: Date.now() },
        lastGateLocalAction: null,
      })),

    // P2：排除受理回执（human_gate_exclusions_accepted）——durable 已确认，
    // 失败事实与动作记录都清空（防回执后的无关错误被误归因成排除失败）。
    resolveGateExclusion: () =>
      set({ gateExclusionFailure: null, lastGateLocalAction: null }),
  };
}
