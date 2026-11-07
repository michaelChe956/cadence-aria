import { protocolErrorCopy } from "./protocol-error-copy";
import type { WorkspaceWsState } from "./workspace-ws-store-types";
import {
  compileRecoveryGateNode,
  isAdvanceReplayCode,
  selectGateProjection,
} from "./workspace-cockpit-gate-projection";
import type {
  CockpitInboxItem,
  TrustMissingProjection,
} from "./workspace-cockpit-inbox-types";
import { gateInboxHeadline, gateInboxTitle } from "./workspace-cockpit-inbox-items";
import { cockpitFlowState, flowElapsedMs, type CockpitFlowRow } from "./workspace-cockpit-flow";

// 从 workspace-cockpit-projection.ts 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 选择器入口（收件箱 selectCockpitInbox / 执行流 selectCockpitFlow）与 trust 缺失回落。
export function selectCockpitInbox(state: WorkspaceWsState): CockpitInboxItem[] {
  const items: CockpitInboxItem[] = [];
  const gate = selectGateProjection(state);

  if (gate && gate.closed === null) {
    const findingsCount = gate.findings.length;
    // REQ-PCG-02：recovery 门必须展示中断原因（引擎写在 recovery timeline node 的
    // summary）；原因未同步时只读诊断文案，不以默认动作猜测。
    const recoveryReason =
      gate.kind === "compile_recovery"
        ? (compileRecoveryGateNode(state)?.summary ?? null)
        : null;
    const parts = [
      gateInboxHeadline(gate),
      gate.kind === "compile_recovery"
        ? (recoveryReason ?? "中断原因未同步（只读诊断）")
        : null,
      findingsCount > 0 ? `findings ${findingsCount} 条` : null,
      gate.remaining_budget !== null ? `人工返修还可做 ${gate.remaining_budget} 次` : null,
      gate.turn?.status === "failed" && gate.turn.failure_message
        ? `失败：${gate.turn.failure_class ?? "unknown"} · ${gate.turn.failure_message}`
        : null,
      gate.turn?.status === "busy" ? "引擎在处理中" : null,
    ].filter((part): part is string => Boolean(part));

    items.push({
      id: `gate:${gate.key}`,
      kind: "gate",
      severity: gate.triage ? 2 : 1,
      title: gateInboxTitle(gate),
      summary: parts.join(" · "),
      triage: gate.triage,
      source: "gate",
      createdAt: gate.opened_at || null,
      gate,
      inlineError: gate.turn?.inlineError ?? null,
      choice: null,
    });
  }

  if (state.sessionStatus === "stopped_needs_human") {
    // 批 2 P5（C1 前端半）：durable 归属 owner=server 且 enabled 即托管态
    //（与 P4 服务端 takeover 拒绝同源）——文案指向对话窗口继续而非人工
    // 接管；非托管保留 L4 最小版接管文案。
    const automationManaged =
      state.automation?.owner === "server" && state.automation.enabled === true;
    items.push({
      id: `stopped:${state.sessionId ?? "session"}`,
      kind: "stopped",
      severity: 2,
      title: "会话停在停点",
      // L4（最小版）：停点原因从 session 状态字段取，后缀为静态接管文案；
      // 完整停点指纹（已完成原因 + 已完成进度 + 失败指纹）随 1b 的 takeover 接线（REQ-UI37-08，tasks.md 2.9）深化。
      summary: automationManaged
        ? `停点原因：${state.sessionStatus} · 自动化托管中——在对话窗口补说明继续`
        : `停点原因：${state.sessionStatus} · 等待人工接管后继续`,
      triage: false,
      source: "session_status",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
      automationManaged,
    });
  }

  if (state.protocolError) {
    items.push({
      id: `hard_error:protocol:${state.protocolError.code}`,
      kind: "hard_error",
      severity: 3,
      // F-50 裁决 6：中文主显 lead（错误码由 protocolErrorCode 走 mono 副行，
      // 英文原文留在 summary 由错误条折叠呈现）。
      title: protocolErrorCopy(state.protocolError.code).lead,
      summary: state.protocolError.message,
      triage: false,
      source: "protocol_error",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
      protocolErrorCode: state.protocolError.code,
    });
  }

  // 「登记并重试」按钮面（owner 2026-10-12 直批）：按稳定码识别 trust
  // 缺失错误 + provider wire 标签（同源生成器），回执对账切成功态。
  // 指引卡简化（owner 2026-10-12 直批）：结构化 trust_missing_guidance
  // 帧优先（wire 事实直挂 providers/canonicalRoot/commandMarkdown）；
  // 无结构化帧（旧服务端/其它启动臂）回落既有稳定码文本识别。
  const trustMissing =
    state.error === null
      ? null
      : (trustMissingFromGuidance(state) ?? trustMissingFromErrorText(state.error));
  if (state.error) {
    items.push({
      id: "hard_error:error",
      kind: "hard_error",
      severity: 3,
      title: "引擎错误",
      summary: state.error,
      triage: false,
      source: "engine_error",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
      trustMissing,
      trustRegistered:
        trustMissing !== null && state.trustRegistration?.provider === trustMissing.provider,
    });
  }

  for (const command of Object.values(state.advanceCommands)) {
    if (command.status !== "rejected" || command.code === null || isAdvanceReplayCode(command.code)) {
      continue;
    }
    items.push({
      id: `hard_error:advance:${command.command_id}`,
      kind: "hard_error",
      severity: 3,
      title: "推进被拒",
      summary: `${command.code} · ${command.reason ?? ""}`.trim(),
      triage: false,
      source: "advance",
      createdAt: null,
      gate: null,
      inlineError: command.inlineError,
      choice: null,
    });
  }

  // P0 1.3（REQ-WIGA-05）Task 11：pending choice 逐条一张卡（不可批量选择，
  // isSelectableGate 只认 human_gate）。expected_run_id 缺失（旧投影/无活跃
  // run）不猜 run——卡面提示刷新后作答，不给提交面。
  for (const request of state.pendingChoiceRequests ?? []) {
    items.push({
      id: `choice:${request.id}`,
      kind: "choice",
      severity: 2,
      title: "选择请求待作答",
      summary: request.prompt,
      triage: false,
      source: "choice",
      createdAt:
        request.created_at_ms !== null ? new Date(request.created_at_ms).toISOString() : null,
      gate: null,
      inlineError: null,
      choice: {
        sessionId: state.sessionId,
        choiceId: request.id,
        prompt: request.prompt,
        expectedRunId: request.expected_run_id,
        questions: request.questions,
        allowMultiple: request.allow_multiple,
        allowFreeText: request.allow_free_text,
        source: "workspace",
        status: "open",
      },
    });
  }

  // P2 GAP-E/G（Task 0.1）：durable Failed 的 SingleCandidate 现场投影——
  // 最新 Failed timeline 节点 + phase=failed；只读事实 + 人工显式重驱动作，
  // 绝不自动重试。非 Failed 相位/非 SC flow 不投影。
  if (
    state.flowKind === "single_candidate" &&
    state.singleCandidatePhase === "failed" &&
    state.sessionStatus === "failed"
  ) {
    const failedNode = [...state.timelineNodes]
      .filter((node) => node.status === "failed")
      .sort((left, right) => right.started_at.localeCompare(left.started_at))[0];
    if (failedNode) {
      items.push({
        id: `sc_failed:${failedNode.node_id}`,
        kind: "sc_failed",
        severity: 2,
        title: "单候选评审运行失败",
        summary: `${failedNode.summary ?? "评审运行失败"} · 等待人工显式重驱`,
        triage: false,
        source: "sc_failed",
        createdAt: failedNode.started_at ?? null,
        gate: null,
        inlineError: null,
        choice: null,
        scFailure: { failedNodeId: failedNode.node_id, phase: "failed" },
      });
    }
  }

  // C1（Task 6.1）：durable LC 恢复收件箱卡——仅 waiting/fresh_reserved/
  // fresh_running 三活跃态投卡；resolved 证据保留在 store.lcResumeRecovery
  // 供 Replay 对账，但不投卡、不阻塞 generic 后续动作（设计 §6：不得永久
  // 屏蔽合法修订）。动作资格只镜像 wire allowed_actions，不从错误文本推导。
  // 部分构造的观察态（as unknown 掩码路径）可能缺字段：undefined 视同 null。
  const lcRecovery = state.lcResumeRecovery ?? null;
  if (lcRecovery !== null && lcRecovery.state !== "resolved") {
    items.push({
      id: `lc_resume_recovery:${lcRecovery.recovery_id}`,
      kind: "lc_resume_recovery",
      severity: 2,
      title: lcRecovery.state === "waiting" ? "旧原生会话已失效" : "正在启动新原生会话…",
      summary: `${lcRecovery.reason_code} · ${lcRecovery.provider} · ${lcRecovery.run_kind}`,
      triage: false,
      source: "lc_resume_recovery",
      createdAt: lcRecovery.detected_at,
      gate: null,
      inlineError: null,
      choice: null,
      recovery: {
        recoveryId: lcRecovery.recovery_id,
        state: lcRecovery.state,
        reasonCode: lcRecovery.reason_code,
        provider: lcRecovery.provider,
        failedNodeId: lcRecovery.failed_node_id,
        runKind: lcRecovery.run_kind,
        detectedAt: lcRecovery.detected_at,
        updatedAt: lcRecovery.updated_at,
        allowedActions: lcRecovery.allowed_actions,
        lastOperation: lcRecovery.last_operation ?? null,
      },
    });
  }

  return items.sort((left, right) => {
    if (left.severity !== right.severity) {
      return right.severity - left.severity;
    }
    const leftCreated = left.createdAt ?? "";
    const rightCreated = right.createdAt ?? "";
    if (leftCreated !== rightCreated) {
      return rightCreated.localeCompare(leftCreated);
    }
    return left.id.localeCompare(right.id);
  });
}

/** trust 缺失稳定判别码（服务端 classify_launch_trust_failure_details/
 * 只读等待投影生成，出现在 error 原文）与 provider wire 标签（codex/
 * kimi_code，同源 provider_wire_label）；识别不到不投按钮面（fail-closed
 * 不猜 provider）。 */
const TRUST_MISSING_ERROR_CODES = ["provider_trust_missing", "trust_entry_missing"];
const TRUST_PROVIDER_WIRE_LABELS = ["kimi_code", "codex"] as const;

/** 旧服务端/无结构化帧回落：仅 provider 字段（识别不到不投按钮面，
 * fail-closed 不猜 provider）。 */
function trustMissingFromErrorText(message: string): TrustMissingProjection | null {
  if (!TRUST_MISSING_ERROR_CODES.some((code) => message.includes(code))) {
    return null;
  }
  const provider = TRUST_PROVIDER_WIRE_LABELS.find((label) => message.includes(label));
  return provider === undefined ? null : { provider };
}

/** 结构化 trust 指导（wire trust_missing_guidance 镜像）→ 指引卡投影；
 * 与 `trustMissingFromErrorText` 成对（调用侧 `??` 回落）：镜像字段做
 * 空串/缺席归一（canonicalRoot/retryAction 空→null），指导缺席或
 * providers 为空返回 null 交回落。 */
function trustMissingFromGuidance(state: WorkspaceWsState): TrustMissingProjection | null {
  const guidance = state.trustMissingGuidance;
  if (guidance == null || guidance.providers.length === 0) {
    return null;
  }
  return {
    provider: guidance.providers[0],
    providers: guidance.providers,
    canonicalRoot: guidance.canonicalRoot !== "" ? guidance.canonicalRoot : null,
    commandMarkdown: guidance.commandMarkdown,
    retryAction: guidance.retryAction !== "" ? guidance.retryAction : null,
  };
}

export function selectCockpitFlow(state: WorkspaceWsState, nowMs = Date.now()): CockpitFlowRow[] {
  const total = state.timelineNodes.length;
  // 与收件箱同源：awaiting_triage 是「开放 triage 门」的投影态，门禁关闭后必须回到 running。
  const gate = selectGateProjection(state);
  const awaitingTriage = gate !== null && gate.closed === null && gate.triage;
  const topology = state.timelineNodes.map((node) =>
    cockpitFlowState(node, awaitingTriage && node.node_id === state.activeNodeId),
  );

  return state.timelineNodes.map((node, index) => {
    // REQ-UI37-18：静默量取自最近一次引擎事件；节点没有 `last_event_at`
    // （老数据 / session_state 重建快照）时回退 `started_at`，与既有行为一致。
    const lastEventAt = Date.parse(node.last_event_at ?? node.started_at);
    return {
      node_id: node.node_id,
      title: node.title,
      state: topology[index] ?? "pending",
      index: index + 1,
      total,
      elapsed_ms: flowElapsedMs(node, nowMs),
      idle_ms: Number.isNaN(lastEventAt) ? 0 : Math.max(0, nowMs - lastEventAt),
      started_at: node.started_at,
      topology,
    };
  });
}
