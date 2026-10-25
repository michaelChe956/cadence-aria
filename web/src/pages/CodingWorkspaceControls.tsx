import { Check, Play, Send, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import type {
  CodingExecutionStage,
  CodingGateRequired,
  GroupFinalReadinessDiagnostic,
  GroupFinalReadinessStatus,
  CodingProviderRole,
  VerificationCommandEvidence,
  VerificationTriageRecord,
} from "../api/types";
import { StageGateEntry } from "../components/coding-workspace/StageGateEntry";
import { ConfirmTwiceButton } from "../components/chat-workspace/cockpit/ConfirmTwiceButton";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import type { ChatEntry } from "../state/chat-entries";
import { codingStartupRejectionCopy } from "../state/cockpit-operation-semantics";
import { CoderTriageCard, GateMetadata, VerificationTriageSection } from "./CoderTriageCard";
import type { CodingPendingGate } from "../state/coding-workspace-store";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";

export const ACTIVE_ATTEMPT_STATUSES = new Set([
  "created",
  "running",
  "waiting_for_human",
  "blocked",
  "awaiting_manual_recovery",
]);


function blockedGateDisplayTitle(gate: CodingPendingGate) {
  return gate.title;
}


export function CodingPanelTabs({
  activePanel,
  onSelectPanel,
}: {
  activePanel: "chat" | "results";
  onSelectPanel: (panel: "chat" | "results") => void;
}) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2">
      <div className="flex min-w-0 items-center gap-1">
        <button
          type="button"
          onClick={() => onSelectPanel("chat")}
          className={codingPanelTabClass(activePanel === "chat")}
        >
          运行对话
        </button>
        <button
          type="button"
          onClick={() => onSelectPanel("results")}
          className={codingPanelTabClass(activePanel === "results")}
        >
          运行结果
        </button>
      </div>
    </div>
  );
}

function codingPanelTabClass(active: boolean) {
  return [
    "inline-flex h-8 items-center rounded-md px-3 text-xs font-semibold transition-colors",
    active
      ? "bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
      : "text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
  ].join(" ");
}

export function errorMessage(reason: unknown, fallback: string) {
  return reason instanceof Error ? reason.message : fallback;
}

export function requestIdFromEntry(entry: ChatEntry) {
  const requestId = entry.metadata?.request_id;
  return typeof requestId === "string" ? requestId : null;
}

export function CodingComposer({
  api,
  stage,
  status,
  statusText,
  pendingGate,
  groupFinalReadinessStatus,
  groupFinalReadinessDiagnostics = [],
  startupErrorCode = null,
  automationManaged = false,
}: {
  api: ReturnType<typeof useCodingWorkspaceWs>;
  stage: CodingExecutionStage | null;
  status: string | null;
  statusText: string;
  pendingGate?: CodingPendingGate | null;
  groupFinalReadinessStatus?: GroupFinalReadinessStatus | null;
  groupFinalReadinessDiagnostics?: GroupFinalReadinessDiagnostic[];
  startupErrorCode?: string | null;
  /** 批 2 T4（REQ-WIGA-09）：托管下隐藏单 attempt 中止（终止走统一入口）。 */
  automationManaged?: boolean;
}) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const inputDisabled = status === "completed" || status === "aborted";
  const canSend = !inputDisabled && trimmedInput.length > 0;
  const blockedByGate = pendingGate?.kind === "blocked";
  // F-E：底部这行提示与上方门卡/字段同一套词汇——人工放行门写的是放行说明
  //（进质量豁免审计），发回 Coder 门才是修复意见；两者并存时说"人工处理意见"。
  const blockedGateHasBypass =
    pendingGate?.available_actions.some(actionIsQualityBypass) ?? false;
  const blockedGateHasCoderFeedback =
    pendingGate?.available_actions.some((action) => action.action_type === "send_to_coder") ??
    false;
  const blockedComposerHint =
    blockedGateHasBypass && blockedGateHasCoderFeedback
      ? "请使用上方门禁操作提交人工处理意见（人工放行会记入质量豁免审计）"
      : blockedGateHasBypass
        ? "请使用上方门禁操作提交人工放行说明（会记入质量豁免审计）"
        : "请使用上方门禁操作提交人工修复意见";

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSend) {
      return;
    }
    api.sendContextNote(trimmedInput);
    setInput("");
  }

  if (blockedByGate) {
    return (
      <div className="grid gap-2 border-t border-[var(--aria-line)] bg-white px-3 py-2">
        <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
          {blockedComposerHint}
        </div>
        <div className="flex min-w-0 items-center justify-between gap-2">
          <div className="truncate text-xs text-[var(--aria-ink-muted)]">{statusText}</div>
          {status && ACTIVE_ATTEMPT_STATUSES.has(status) && !automationManaged ? (
            <button
              type="button"
              onClick={api.abortAttempt}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]"
              aria-label="底部中止"
            >
              <X className="h-3.5 w-3.5" />
              中止
            </button>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="grid gap-2 border-t border-[var(--aria-line)] bg-white px-3 py-2"
    >
      <textarea
        aria-label="补充 Coding 上下文"
        value={input}
        onChange={(event) => setInput(event.target.value)}
        disabled={inputDisabled}
        rows={2}
        placeholder="补充上下文"
        className="min-h-16 w-full resize-y rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
      />
      <div className="flex min-w-0 items-center justify-between gap-2">
        <div className="truncate text-xs text-[var(--aria-ink-muted)]">{statusText}</div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="submit"
            disabled={!canSend}
            className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
          >
            <Send className="h-3.5 w-3.5" />
            发送上下文
          </button>
          <ActionButtons
            api={api}
            stage={stage}
            status={status}
            startupErrorCode={startupErrorCode}
            compact
            groupFinalReadinessStatus={groupFinalReadinessStatus}
            groupFinalReadinessDiagnostics={groupFinalReadinessDiagnostics}
            automationManaged={automationManaged}
          />
        </div>
      </div>
    </form>
  );
}

export function ActionButtons({
  api,
  stage,
  status,
  compact = false,
  groupFinalReadinessStatus,
  groupFinalReadinessDiagnostics = [],
  startupErrorCode = null,
  automationManaged = false,
  automationTerminated = false,
}: {
  api: ReturnType<typeof useCodingWorkspaceWs>;
  stage: CodingExecutionStage | null;
  status: string | null;
  compact?: boolean;
  groupFinalReadinessStatus?: GroupFinalReadinessStatus | null;
  groupFinalReadinessDiagnostics?: GroupFinalReadinessDiagnostic[];
  startupErrorCode?: string | null;
  /** 批 2 T4：托管下隐藏全部单 attempt 中止按钮（managed 判据=T3 同源投影）。 */
  automationManaged?: boolean;
  /** 批 2 T4：终止后不再提供重新开始（无接管/局部 abort/restart）。 */
  automationTerminated?: boolean;
}) {
  const buttonClass = compact
    ? "inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]"
    : "inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]";
  const finalConfirmReady =
    groupFinalReadinessStatus === "complete" && groupFinalReadinessDiagnostics.length === 0;
  const finalConfirmDiagnostic = groupFinalReadinessDiagnostics[0]?.message;
  const startupCopy = codingStartupRejectionCopy(startupErrorCode);

  // F-44：终态（用户已中止/失败）优先于阶段分支——此时阶段按钮（开始/继续/
  // 确认完成）都不再可达，只提供「重新开始」显式入口（两步确认防误触，与
  // GatePromptEntry 的终止按钮同款 ConfirmTwiceButton 先例）；运行中/已完成
  // 不渲染（completed 不提供重开，进行中的 attempt 走阶段按钮）。
  if (status === "aborted" || status === "failed") {
    // 批 2 T4（REQ-WIGA-09）：终止后不再提供重新开始（无接管/局部
    // abort/restart；重启由 automation 链路或服务端拒绝收口）。
    if (automationTerminated) {
      return null;
    }
    return (
      <div className="flex items-center gap-2">
        <ConfirmTwiceButton
          label="重新开始"
          confirmLabel="确认重新开始"
          ariaLabel={compact ? "底部重新开始" : undefined}
          confirmAriaLabel={compact ? "底部确认重新开始" : undefined}
          onConfirm={api.restartCoding}
        />
      </div>
    );
  }

  if (stage === "prepare_context") {
    return (
      <div className="flex items-center gap-2">
        <button
          type="button"
          data-testid={
            compact
              ? "onboarding-anchor-coding-start-compact"
              : "onboarding-anchor-coding-start"
          }
          onClick={api.startCoding}
          disabled={startupCopy !== null}
          title={startupCopy ?? undefined}
          className={`${buttonClass} disabled:cursor-not-allowed disabled:opacity-50`}
          aria-label={compact ? "底部开始 Coding" : undefined}
        >
          <Play className="h-3.5 w-3.5" />
          开始 Coding
        </button>
        {!compact && startupCopy ? (
          <span role="status" className="text-xs text-[var(--aria-danger)]">
            {startupCopy}
          </span>
        ) : null}
      </div>
    );
  }

  if (stage === "review_request" && status && ACTIVE_ATTEMPT_STATUSES.has(status)) {
    return (
      <div className="flex items-center gap-2">
        <div className="flex items-center gap-2">
          <button
            type="button"
            data-testid={
              compact
                ? "onboarding-anchor-coding-start-compact"
                : "onboarding-anchor-coding-start"
            }
            onClick={api.startCoding}
            disabled={startupCopy !== null}
            title={startupCopy ?? undefined}
            className={`${buttonClass} disabled:cursor-not-allowed disabled:opacity-50`}
            aria-label={compact ? "底部继续 Coding" : undefined}
          >
            <Play className="h-3.5 w-3.5" />
            继续 Coding
          </button>
          {!compact && startupCopy ? (
            <span role="status" className="text-xs text-[var(--aria-danger)]">
              {startupCopy}
            </span>
          ) : null}
        </div>
        {automationManaged ? null : (
          <button
            type="button"
            onClick={api.abortAttempt}
            className={buttonClass}
            aria-label={compact ? "底部中止" : undefined}
          >
            <X className="h-3.5 w-3.5" />
            中止
          </button>
        )}
      </div>
    );
  }

  if (stage === "final_confirm" && status === "waiting_for_human") {
    return (
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={api.finalConfirm}
          disabled={!finalConfirmReady}
          title={finalConfirmReady ? undefined : finalConfirmDiagnostic ?? "最终确认证据尚未完整"}
          className={`${buttonClass} disabled:cursor-not-allowed disabled:opacity-50`}
          aria-label={compact ? "底部确认完成" : undefined}
        >
          <Check className="h-3.5 w-3.5" />
          确认完成
        </button>
        {!compact && !finalConfirmReady && finalConfirmDiagnostic ? (
          <span className="max-w-64 text-xs text-[var(--aria-danger)]" role="status">
            {finalConfirmDiagnostic}
          </span>
        ) : null}
        {automationManaged ? null : (
          <button
            type="button"
            onClick={api.abortAttempt}
            className={buttonClass}
            aria-label={compact ? "底部中止" : undefined}
          >
            <X className="h-3.5 w-3.5" />
            中止
          </button>
        )}
      </div>
    );
  }

  if (status && ACTIVE_ATTEMPT_STATUSES.has(status) && !automationManaged) {
    return (
      <button
        type="button"
        onClick={api.abortAttempt}
        className={buttonClass}
        aria-label={compact ? "底部中止" : undefined}
      >
        <X className="h-3.5 w-3.5" />
        中止
      </button>
    );
  }

  return null;
}

export function GatePanel({
  gate,
  onRespond,
  onConfirmStage,
  onAbort,
  automationManaged = false,
  onOpenTermination,
  verificationTriage = null,
  onEnterVerificationTriage,
  commandEvidence = null,
  onRerunPlannedCommand,
}: {
  gate: CodingPendingGate | null;
  onRespond: ReturnType<typeof useCodingWorkspaceWs>["respondGate"];
  onConfirmStage: ReturnType<typeof useCodingWorkspaceWs>["confirmStageGate"];
  onAbort: ReturnType<typeof useCodingWorkspaceWs>["abortAttempt"];
  /** 批 2 T4（REQ-WIGA-09）：托管判据（T3 同源投影 owner=server）——门动作
   * 集合不变，但 Abort 执行语义改整 plan 统一终止。 */
  automationManaged?: boolean;
  /** 批 2 T4：打开统一终止二次确认面（页面接线，零请求副作用）。 */
  onOpenTermination?: () => void;
  /** C2 Task 8：验证处理旁路面板（不进 action 枚举与 available_actions）。 */
  verificationTriage?: VerificationTriageRecord | null;
  onEnterVerificationTriage?: (gateId: string) => void;
  /** C2 Task 9：计划命令与实际命令并列证据＋重跑入口。 */
  commandEvidence?: VerificationCommandEvidence | null;
  onRerunPlannedCommand?: (checkId: string) => void;
}) {
  const [reason, setReason] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  // F-D：最后一个协议错误的原文——门动作被服务端拒绝时，gate.errorCode 只有
  // `coding_gate_response_failed` 这个粗码，具体原因（如 send_to_coder 的前置
  // 不满足）在 message 里。两者配对只用于识别「这一实例的哪次失败」。
  const protocolErrorMessage = useCodingWorkspaceStore(
    (state) => state.protocolError?.message ?? null,
  );

  if (!gate) {
    return null;
  }

  if (gate.kind === "stage_gate") {
    return (
      <StageGateEntry gate={gate} onConfirmStage={onConfirmStage} onAbort={onAbort} />
    );
  }

  // P2-d（REQ-CRO-09）：coder 分诊门走 CoderTriageCard（三段大白话＋三按钮＋
  // 技术详情/高级验证处理折叠）；其余门走原渲染，零改动。
  if (isTriageCardGate(gate)) {
    return (
      <CoderTriageCard
        gate={gate}
        onRespond={onRespond}
        automationManaged={automationManaged}
        onOpenTermination={onOpenTermination}
        verificationTriage={verificationTriage}
        onEnterVerificationTriage={onEnterVerificationTriage}
        commandEvidence={commandEvidence}
        onRerunPlannedCommand={onRerunPlannedCommand}
      />
    );
  }

  const activeGate = gate;
  const submitting = activeGate.submitting === true;
  const gateErrorCode = activeGate.errorCode ?? null;
  const gateReasonCode = activeGate.reason_code ?? null;
  const contextActions = activeGate.available_actions.filter(actionRequiresContext);
  const needsReason = contextActions.length > 0;
  const needsCoderFeedbackContext = activeGate.available_actions.some(
    (action) => action.action_type === "send_to_coder",
  );
  const trimmedReason = reason.trim();
  const reasonMissing = needsReason && trimmedReason === "";
  const reasonTooLong = reason.length > 2000;
  const displayedError = reasonTooLong ? "原因不能超过 2000 字" : localError;
  const displayTitle = blockedGateDisplayTitle(activeGate);
  const hasQualityBypassAction = activeGate.available_actions.some(actionIsQualityBypass);
  // F-E：输入框的标签/占位/必填文案按"这个门到底让人做什么"分派——人工放行门写的是
  // 放行说明（进质量豁免审计），发回 Coder 门写的是修复意见；两者同门并存时分开说。
  const reasonField = reasonFieldCopy(needsCoderFeedbackContext, hasQualityBypassAction);
  // F-D：服务端已判定「最近一次审查结论不能作为返修指令」（send_to_coder 的前置是
  // verdict ∈ {request_changes, blocked}）。判定只作用于**当前门实例**：gate.errorCode
  // 由 store 在每次提交前清空，门被解决或换成新实例即自然复位；动作集合仍由服务端
  // available_actions 权威（spec 要求恰好四项），这里只是把这一实例的该动作置为禁用，
  // 不从清单里删除。
  const sendToCoderRejected =
    gateErrorCode === GATE_RESPONSE_FAILED_CODE &&
    (protocolErrorMessage ?? "").includes(SEND_TO_CODER_NOT_ACTIONABLE_CODE);
  const gateErrorCopy = gateErrorGuidance(gateErrorCode, protocolErrorMessage);
  // 指引里的替代动作必须引用本门真实的动作文案（门的 label 有 "重试代码审查"/"重试审查" 两族）。
  const retryActionLabel =
    activeGate.available_actions.find((action) => action.action_type === "retry_review")?.label ??
    null;
  const qualityBypassLabel =
    activeGate.available_actions.find(actionIsQualityBypass)?.label ?? null;

  function actionDisabled(action: CodingGateRequired["available_actions"][number]) {
    if (submitting) {
      return true;
    }
    if (action.action_type === "send_to_coder" && sendToCoderRejected) {
      return true;
    }
    // F-E：必填校验前置——空说明时直接把需要上下文的动作禁用，并就地说明要填什么，
    // 而不是让用户点一次才知道。
    return actionRequiresContext(action) && (reasonMissing || reasonTooLong);
  }

  function handleAction(action: CodingGateRequired["available_actions"][number]) {
    // 批 2 T4（REQ-WIGA-09）：托管下门 Abort 动作改走整 plan 统一终止
    //（执行语义改变，动作集合/文案保持原样）；非 Abort 动作不变。
    if (action.action_type === "abort" && automationManaged && onOpenTermination) {
      onOpenTermination();
      return;
    }
    if (action.action_type === "confirm_stage" && activeGate.stage) {
      onConfirmStage(activeGate.stage);
      return;
    }
    if (actionRequiresContext(action)) {
      if (!trimmedReason) {
        // 防御：按钮已按 reasonMissing 置灰，键盘/脚本路径仍然给出同一句可执行提示。
        setLocalError(missingContextMessage(action, gateReasonCode));
        return;
      }
      if (reasonTooLong) {
        return;
      }
      setLocalError(null);
      onRespond(activeGate.gate_id, action.action_id, trimmedReason);
      return;
    }
    setLocalError(null);
    onRespond(activeGate.gate_id, action.action_id, undefined);
  }

  return (
    <div
      data-testid="coding-pending-gate"
      className="border-t border-amber-200 bg-amber-50 px-3 py-2"
    >
      <div className="flex min-w-0 flex-col gap-2 md:flex-row md:items-center md:justify-between">
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-amber-900">{displayTitle}</div>
          <div className="mt-0.5 line-clamp-2 text-xs text-amber-800">
            {activeGate.description}
          </div>
          {hasQualityBypassAction ? (
            <div className="mt-1 text-xs font-semibold text-amber-900">
              人工放行会记录质量豁免；请说明跳过该门禁的原因和后续风险处理
            </div>
          ) : null}
          {/* F-D：必败动作不再静默失败——服务端已判定的原因改成可执行指引，
              并把该实例的「提交给 Coder 修复」置灰（动作集合本身仍由服务端
              available_actions 权威，未从清单移除）。 */}
          {sendToCoderRejected ? (
            <div
              data-testid="coding-gate-send-to-coder-unavailable"
              role="status"
              className="mt-1 rounded-md border border-amber-300 bg-white px-2 py-1.5 text-xs leading-4 text-amber-900"
            >
              「提交给 Coder 修复」在当前门不可用：最近一次 Code Review 结论无法作为返修指令
              （结论不是 request_changes／blocked）。
              {retryActionLabel ? `请改用「${retryActionLabel}」；` : null}
              {qualityBypassLabel
                ? `或填写人工放行说明后点「${qualityBypassLabel}」（会记入质量豁免审计）。`
                : null}
              重试审查或本门解决后该动作会自动恢复可用。
            </div>
          ) : null}
          {isVerificationTriageEligibleGate(activeGate) ? (
            <VerificationTriageSection
              gate={activeGate}
              verificationTriage={verificationTriage}
              onEnterVerificationTriage={onEnterVerificationTriage}
              commandEvidence={commandEvidence}
              onRerunPlannedCommand={onRerunPlannedCommand}
            />
          ) : null}
          <GateMetadata gate={activeGate} />
          {needsReason ? (
            <div className="mt-2 grid gap-1">
              <textarea
                aria-label={reasonField.label}
                value={reason}
                onChange={(event) => {
                  setReason(event.target.value);
                  setLocalError(null);
                }}
                rows={2}
                maxLength={2100}
                placeholder={reasonField.placeholder}
                aria-describedby={reasonMissing ? "coding-gate-context-required" : undefined}
                className="min-h-14 w-full resize-y rounded-md border border-amber-300 bg-white px-2 py-1.5 text-xs text-amber-950 placeholder:text-amber-700"
              />
              {/* F-E：必填校验前置——空说明时先就地写清「哪个动作、要填什么、会有什么后果」，
                  对应动作同时置灰；不是点一次才知道要填。 */}
              {reasonMissing ? (
                <div
                  id="coding-gate-context-required"
                  data-testid="coding-gate-context-required"
                  role="status"
                  className="text-xs leading-4 text-amber-800"
                >
                  {contextActions.map((action) => missingContextMessage(action, gateReasonCode)).join("；")}
                </div>
              ) : null}
              {displayedError ? (
                <div className="text-xs font-semibold text-[var(--aria-danger)]">
                  {displayedError}
                </div>
              ) : null}
            </div>
          ) : null}
          {gateErrorCode ? (
            <div className="mt-1 flex flex-col gap-0.5">
              {gateErrorCopy ? (
                <div role="alert" className="text-xs font-semibold text-[var(--aria-danger)]">
                  {gateErrorCopy}
                </div>
              ) : null}
              {/* 原始错误码保留（排障/支持用），但不再作为唯一信息。 */}
              <div className="aria-mono text-[10px] text-[var(--aria-ink-muted)]">
                {gateErrorCode}
              </div>
            </div>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {activeGate.available_actions.map((action) => (
            <button
              key={action.action_id}
              type="button"
              disabled={actionDisabled(action)}
              title={
                action.action_type === "send_to_coder" && sendToCoderRejected
                  ? "最近一次 Code Review 结论无法作为返修指令；请重试代码审查或人工继续"
                  : undefined
              }
              onClick={() => handleAction(action)}
              className="inline-flex h-8 items-center justify-center rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-900 hover:bg-amber-100 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {submitting ? "处理中" : action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function actionRequiresContext(action: CodingGateRequired["available_actions"][number]) {
  return (
    action.action_type === "manual_continue" ||
    action.action_type === "accept_risk" ||
    action.action_type === "send_to_coder" ||
    // P2-d（REQ-CRO-09/A4）：说明情况重试需非空 extra_context（前端与后端双重校验）。
    action.action_type === "retry_coding_with_context"
  );
}

function actionIsQualityBypass(action: CodingGateRequired["available_actions"][number]) {
  return action.action_type === "manual_continue" || action.action_type === "accept_risk";
}

/** 门动作被服务端拒绝时 WS 帧里的粗码（`coding_ws_handler/socket.rs`）。 */
const GATE_RESPONSE_FAILED_CODE = "coding_gate_response_failed";
/** `send_code_review_feedback_to_coder` 的前置不满足码（`rework.rs`）。 */
const SEND_TO_CODER_NOT_ACTIONABLE_CODE = "send_to_coder_latest_review_not_actionable";

/** F-E：必填文案必须带动作名与后果，用户才知道"填这段字会换来什么"——
 * 字段名按"这个动作会拿这段文字做什么"分派，不把「人工放行」说成「修复意见」。 */
function missingContextMessage(
  action: CodingGateRequired["available_actions"][number],
  reasonCode: string | null,
): string {
  switch (action.action_type) {
    case "manual_continue":
    case "accept_risk":
      return `「${action.label}」需要填写人工放行说明（将写入 context note 并记入质量豁免审计${reasonCode ? `：reason_code=${reasonCode}` : ""}）`;
    case "send_to_coder":
      return `「${action.label}」需要填写人工修复意见（将作为本轮修复意见发给 Coder）`;
    case "retry_coding_with_context":
      return `「${action.label}」需要填写情况说明（将作为情况说明发给 Coder 并重新开跑）`;
    default:
      return `「${action.label}」需要填写原因（将写入 context note）`;
  }
}

/** F-E：输入框标签/占位按门实际提供的动作分派（放行说明 vs 发回 Coder 的修复意见）。 */
function reasonFieldCopy(
  hasCoderFeedback: boolean,
  hasQualityBypass: boolean,
): { label: string; placeholder: string } {
  if (hasQualityBypass && hasCoderFeedback) {
    return {
      label: "人工处理说明（人工继续会记入质量豁免审计）",
      placeholder:
        "说明人工放行的原因和后续风险处理（会记入质量豁免审计），或写发回 Coder 的修复意见",
    };
  }
  if (hasQualityBypass) {
    return {
      label: "人工放行说明（记入质量豁免审计）",
      placeholder: "说明人工放行的原因和后续风险处理；该说明会写入 context note 并记入质量豁免审计",
    };
  }
  if (hasCoderFeedback) {
    return {
      label: "人工修复意见",
      placeholder: "补充本轮人工修复意见；该意见会优先于 Code Reviewer findings",
    };
  }
  return {
    label: "门禁跳过原因",
    placeholder: "说明跳过该门禁的原因和后续风险处理",
  };
}

/** F-D：把已经发生的门动作失败翻译成可执行指引（原始码另以小字保留）。 */
function gateErrorGuidance(
  errorCode: string | null,
  protocolErrorMessage: string | null,
): string | null {
  if (errorCode === null) {
    return null;
  }
  if ((protocolErrorMessage ?? "").includes(SEND_TO_CODER_NOT_ACTIONABLE_CODE)) {
    return "「提交给 Coder 修复」被服务端拒绝：最近一次 Code Review 结论不是 request_changes／blocked，无法作为返修指令。请重试代码审查，或填写人工放行说明后点「人工继续」。";
  }
  if (errorCode === "coding_gate_extra_context_required") {
    return "该动作需要非空说明文字（人工放行/发回 Coder/说明情况重试都要求填写意见）。";
  }
  return null;
}

const VERIFICATION_TRIAGE_ELIGIBLE_REASON_CODES = [
  "code_review_output_human_triage",
  "code_review_verification_incomplete",
  "code_review_operational_blocker",
  "coding_output_human_triage",
  // P2-a/P2-d（REQ-CVT-06）：运行环境阻塞门同样可转入验证处理（与后端白名单同步）。
  "coding_operational_blocker",
] as const;

/** P2-d（REQ-CRO-09，C5）：走 CoderTriageCard 的分诊门 reason_code——集中定义；
 * P4-e（REQ-CRO-09/PTR-IDLE）：`coder_provider_stalled` 纳入——coder 挂起门
 * 三动作（retry_coding/retry_coding_with_context/abort）复用同一张卡，
 * 说明情况提交即重新开跑（块 4 §3.2）；reviewer 挂起门按 stage 回原门渲染。 */
const TRIAGE_CARD_REASON_CODES = [
  "coding_output_human_triage",
  "coding_operational_blocker",
  "coder_provider_stalled",
] as const;

export function isTriageCardGate(gate: Pick<CodingGateRequired, "reason_code">): boolean {
  return (
    gate.reason_code != null &&
    (TRIAGE_CARD_REASON_CODES as readonly string[]).includes(gate.reason_code)
  );
}

/** C2 Task 8（#19）：可转入独立验证处理的门（coder 输出门＋CR 三门）。 */
export function isVerificationTriageEligibleGate(gate: CodingGateRequired): boolean {
  return (
    gate.reason_code != null &&
    (VERIFICATION_TRIAGE_ELIGIBLE_REASON_CODES as readonly string[]).includes(gate.reason_code)
  );
}

export function lockedProviderRole(
  stage: CodingExecutionStage | null,
  status: string | null,
  pendingGates: CodingGateRequired[],
): CodingProviderRole | null {
  if (pendingGates.some((gate) => gate.kind === "stage_gate")) {
    return null;
  }
  if (status !== "running" || !stage) {
    return null;
  }
  return providerRoleForStage(stage);
}

function providerRoleForStage(stage: CodingExecutionStage): CodingProviderRole | null {
  switch (stage) {
    case "coding":
      return "coder";
    case "code_review":
      return "code_reviewer";
    case "internal_pr_review":
      return "internal_reviewer";
    default:
      return null;
  }
}
