import { Check, Play, RotateCcw, Send, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import type {
  CodingExecutionStage,
  CodingGateRequired,
  CodingProviderRole,
} from "../api/types";
import { StageGateEntry } from "../components/coding-workspace/StageGateEntry";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import type { ChatEntry } from "../state/chat-entries";
import type { CodingPendingGate } from "../state/coding-workspace-store";

export const ACTIVE_ATTEMPT_STATUSES = new Set([
  "created",
  "running",
  "waiting_for_human",
  "blocked",
]);

const TESTING_BLOCKED_REASON_LABELS: Record<string, string> = {
  test_plan_missing_json: "Tester 未返回测试计划 JSON",
  test_plan_invalid_json: "Tester 返回的 JSON 无法解析",
  test_plan_schema_invalid: "Tester 测试计划字段不完整",
  test_plan_repair_failed: "Tester 测试计划修复失败",
  missing_required_steps: "缺少 required 测试步骤证据",
  skipped_required_steps: "required 测试步骤被阻塞（无法执行）",
  testing_blocked: "测试被阻塞",
  high_risk_test_step_requires_permission: "高风险测试步骤需要人工确认",
};

const TESTING_RESULT_REVIEW_REASON_CODE = "testing_result_review_required";

function blockedGateDisplayTitle(gate: CodingPendingGate) {
  if (gate.reason_code === TESTING_RESULT_REVIEW_REASON_CODE) {
    return gate.title;
  }
  if (gate.stage === "testing" && gate.reason_code) {
    return TESTING_BLOCKED_REASON_LABELS[gate.reason_code] ?? gate.reason_code;
  }
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
}: {
  api: ReturnType<typeof useCodingWorkspaceWs>;
  stage: CodingExecutionStage | null;
  status: string | null;
  statusText: string;
}) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const inputDisabled = status === "completed" || status === "aborted";
  const canSend = !inputDisabled && trimmedInput.length > 0;

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSend) {
      return;
    }
    api.sendContextNote(trimmedInput);
    setInput("");
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
          <ActionButtons api={api} stage={stage} status={status} compact />
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
}: {
  api: ReturnType<typeof useCodingWorkspaceWs>;
  stage: CodingExecutionStage | null;
  status: string | null;
  compact?: boolean;
}) {
  const buttonClass = compact
    ? "inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]"
    : "inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]";

  if (stage === "prepare_context") {
    return (
      <button
        type="button"
        onClick={api.startCoding}
        className={buttonClass}
        aria-label={compact ? "底部开始 Coding" : undefined}
      >
        <Play className="h-3.5 w-3.5" />
        开始 Coding
      </button>
    );
  }

  if (stage === "final_confirm" && status === "waiting_for_human") {
    return (
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={api.finalConfirm}
          className={buttonClass}
          aria-label={compact ? "底部确认完成" : undefined}
        >
          <Check className="h-3.5 w-3.5" />
          确认完成
        </button>
        <button
          type="button"
          onClick={api.abortAttempt}
          className={buttonClass}
          aria-label={compact ? "底部中止" : undefined}
        >
          <X className="h-3.5 w-3.5" />
          中止
        </button>
      </div>
    );
  }

  if (stage === "rework" && status === "waiting_for_human") {
    return (
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => api.continueRework(null)}
          className={buttonClass}
          aria-label={compact ? "底部继续返修" : undefined}
        >
          <RotateCcw className="h-3.5 w-3.5" />
          继续返修
        </button>
        <button
          type="button"
          onClick={api.abortAttempt}
          className={buttonClass}
          aria-label={compact ? "底部中止" : undefined}
        >
          <X className="h-3.5 w-3.5" />
          中止
        </button>
      </div>
    );
  }

  if (status && ACTIVE_ATTEMPT_STATUSES.has(status)) {
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
}: {
  gate: CodingPendingGate | null;
  onRespond: ReturnType<typeof useCodingWorkspaceWs>["respondGate"];
  onConfirmStage: ReturnType<typeof useCodingWorkspaceWs>["confirmStageGate"];
  onAbort: ReturnType<typeof useCodingWorkspaceWs>["abortAttempt"];
}) {
  const [reason, setReason] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);

  if (!gate) {
    return null;
  }

  if (gate.kind === "stage_gate") {
    return (
      <StageGateEntry gate={gate} onConfirmStage={onConfirmStage} onAbort={onAbort} />
    );
  }

  const activeGate = gate;
  const submitting = activeGate.submitting === true;
  const gateErrorCode = activeGate.errorCode ?? null;
  const needsReason = activeGate.available_actions.some(actionRequiresReason);
  const trimmedReason = reason.trim();
  const reasonTooLong = reason.length > 2000;
  const displayedError = reasonTooLong ? "原因不能超过 2000 字" : localError;
  const displayTitle = blockedGateDisplayTitle(activeGate);
  const testingResultReview = activeGate.reason_code === TESTING_RESULT_REVIEW_REASON_CODE;
  const testingBlocked = activeGate.stage === "testing" && !testingResultReview;
  const analystGate = activeGate.role === "analyst";
  const hasQualityBypassAction = activeGate.available_actions.some(actionRequiresReason);

  function handleAction(action: CodingGateRequired["available_actions"][number]) {
    if (action.action_type === "confirm_stage" && activeGate.stage) {
      onConfirmStage(activeGate.stage);
      return;
    }
    if (actionRequiresReason(action)) {
      if (!trimmedReason) {
        setLocalError("需要填写原因");
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
          {testingBlocked ? (
            <div className="mt-0.5 text-xs font-semibold text-amber-900">测试被阻塞</div>
          ) : null}
          {testingResultReview ? (
            <div className="mt-0.5 text-xs font-semibold text-amber-900">
              等待确认 Tester 结果
            </div>
          ) : null}
          {analystGate ? (
            <div className="mt-0.5 text-xs font-semibold text-amber-900">
              Analyst 建议人工决策
            </div>
          ) : null}
          <div className="mt-0.5 line-clamp-2 text-xs text-amber-800">
            {activeGate.description}
          </div>
          {hasQualityBypassAction ? (
            <div className="mt-1 text-xs font-semibold text-amber-900">
              人工放行会记录质量豁免；请说明跳过该门禁的原因和后续风险处理
            </div>
          ) : null}
          <GateMetadata gate={activeGate} />
          {needsReason ? (
            <div className="mt-2 grid gap-1">
              <textarea
                aria-label="门禁跳过原因"
                value={reason}
                onChange={(event) => {
                  setReason(event.target.value);
                  setLocalError(null);
                }}
                rows={2}
                maxLength={2100}
                placeholder="说明跳过该门禁的原因和后续风险处理"
                className="min-h-14 w-full resize-y rounded-md border border-amber-300 bg-white px-2 py-1.5 text-xs text-amber-950 placeholder:text-amber-700"
              />
              {displayedError ? (
                <div className="text-xs font-semibold text-[var(--aria-danger)]">
                  {displayedError}
                </div>
              ) : null}
            </div>
          ) : null}
          {gateErrorCode ? (
            <div className="mt-1 text-xs font-semibold text-[var(--aria-danger)]">
              {gateErrorCode}
            </div>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {activeGate.available_actions.map((action) => (
            <button
              key={action.action_id}
              type="button"
              disabled={submitting || (actionRequiresReason(action) && reasonTooLong)}
              onClick={() => handleAction(action)}
              className="inline-flex h-8 items-center justify-center rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-900 hover:bg-amber-100"
            >
              {submitting ? "处理中" : action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function actionRequiresReason(action: CodingGateRequired["available_actions"][number]) {
  return action.action_type === "manual_continue" || action.action_type === "accept_risk";
}

function GateMetadata({ gate }: { gate: CodingPendingGate }) {
  const rows = [
    gate.reason_code ? ["reason", gate.reason_code] : null,
    gate.raw_provider_output_ref ? ["raw", gate.raw_provider_output_ref] : null,
    gate.evidence_refs?.length ? ["evidence", gate.evidence_refs.join(", ")] : null,
  ].filter(Boolean) as [string, string][];
  if (rows.length === 0) {
    return null;
  }
  return (
    <dl className="mt-1 grid gap-0.5 text-xs text-amber-800">
      {rows.map(([label, value]) => (
        <div key={label} className="grid min-w-0 grid-cols-[4.5rem_minmax(0,1fr)] gap-2">
          <dt className="font-semibold">{label}</dt>
          <dd className="min-w-0 break-words font-mono">{value}</dd>
        </div>
      ))}
    </dl>
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
    case "testing":
      return "tester";
    case "rework":
      return "analyst";
    case "code_review":
      return "code_reviewer";
    case "internal_pr_review":
      return "internal_reviewer";
    default:
      return null;
  }
}
