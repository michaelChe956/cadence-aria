import { Check } from "lucide-react";
import { useState } from "react";
import type { ChatEntry } from "../../../state/chat-entries";
import {
  gateActionBlockCopy,
  selectGateProjection,
  type GateActionBlockReason,
  type GateProjection,
  GATE_TRIGGER_LABELS,
} from "../../../state/workspace-cockpit-projection";
import type { WorkspaceWsState } from "../../../state/workspace-ws-store-types";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import type { WorkItemPlanHumanGateSnapshot } from "../../../api/types";
import { WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND } from "../../../state/workspace-chat-rebuild";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { ConfirmTwiceButton } from "../cockpit/ConfirmTwiceButton";
import { GateFeedbackEditor } from "../cockpit/GateFeedbackEditor";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function GatePromptEntry({
  entry,
  actions,
}: {
  entry: ChatEntry;
  actions?: CockpitActionFacade;
}) {
  const [feedback, setFeedback] = useState("");
  const summary = summaryFromEntry(entry);
  const verdict = verdictFromEntry(entry);
  const reviewGate = reviewGateFromEntry(entry);
  const findings = findingsFromEntry(entry);
  const needsHuman = verdict === "needs_human";
  const requiresTriage = reviewGate === "user_triage_required";
  const allowsCurrentVersion = reviewGate === "user_confirm_allowed";
  const confirmLabel =
    requiresTriage
      ? "确认当前版本"
      : allowsCurrentVersion
      ? "确认使用当前版本"
      : needsHuman
        ? "提交人工确认"
        : "确认产物";
  // L1（REQ-RET-02）：request-change 按钮随 legacy 决策发送面删除；findings 仅作呈现。
  const isResolved = entry.resolved === true;
  const gateTrigger = gateTriggerFromEntry(entry);
  const remainingBudget = remainingBudgetFromEntry(entry);
  const failureMessage = failureMessageFromEntry(entry);
  const inlineError = inlineErrorFromEntry(entry);
  const isContextBlockerGate =
    gateKindFromEntry(entry) === WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND;
  const actionFacade =
    (entry.metadata as Record<string, unknown> | undefined)?.action_facade;
  const typedGateAwaitingCommand =
    actionFacade === "typed" &&
    typeof (entry.metadata as Record<string, unknown> | undefined)?.command_id !== "string";
  const persistedActionBlockReason = blockReasonFromEntry(entry, "action_block_reason");
  // F-21：终止判据缺省（undefined）回退通用判据；显式 null=终止放行（?? 会把
  // null 吞成回退，必须辨 undefined）。
  const persistedTerminateBlockReason =
    (entry.metadata as Record<string, unknown> | undefined)?.terminate_block_reason !== undefined
      ? blockReasonFromEntry(entry, "terminate_block_reason")
      : persistedActionBlockReason;
  // F-21：终止与确认共用「活投影匹配 + stage 前缀离场兜底」骨架，仅判据不同
  // （terminate_block_reason 允许 plan 会话 human_confirm 非终审门放行终止）。
  const actionBlockReason = useWorkspaceStore((state) =>
    gateCardBlockReason(state, entry, persistedActionBlockReason, (projection) =>
      projection.action_block_reason ?? null,
    ),
  );
  const terminateBlockReason = useWorkspaceStore((state) =>
    gateCardBlockReason(state, entry, persistedTerminateBlockReason, (projection) =>
      projection.terminate_block_reason !== undefined
        ? projection.terminate_block_reason
        : (projection.action_block_reason ?? null),
    ),
  );
  const title = requiresTriage
    ? "需要判断 reviewer 意图"
    : allowsCurrentVersion
      ? "可确认当前版本"
      : needsHuman
        ? "需要人工确认"
        : "人工确认";

  return (
    <ChatEntryContainer
      role="system"
      title={title}
      className="border-slate-200 bg-slate-50"
      testId="gate-prompt-entry"
    >
      <div className="space-y-3">
        <div className="text-sm text-[var(--aria-ink)]">{entry.content}</div>
        {summary ? <div className="text-xs text-[var(--aria-ink-muted)]">{summary}</div> : null}
        {gateTrigger || remainingBudget !== null ? (
          <div className="flex flex-wrap items-center gap-2 text-xs">
            {gateTrigger ? (
              <span
                data-testid="gate-trigger-label"
                className="aria-chip border-[var(--aria-gate-open-border)] bg-[var(--aria-gate-open-bg)] text-[var(--aria-gate-open-fg)]"
              >
                {GATE_TRIGGER_LABELS[gateTrigger]}
              </span>
            ) : null}
            {remainingBudget !== null ? (
              <span
                data-testid="gate-budget"
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
              >
                剩余修复轮次 {remainingBudget}
              </span>
            ) : null}
          </div>
        ) : null}
        {failureMessage ? (
          <div
            data-testid="gate-failure"
            className="aria-mono text-xs text-[var(--aria-danger)]"
          >
            {failureMessage}
          </div>
        ) : null}
        {inlineError ? (
          <div
            data-testid="gate-inline-protocol-error"
            className="aria-mono text-xs text-[var(--aria-danger)]"
          >
            {inlineError.code} · {inlineError.message}
          </div>
        ) : null}
        {requiresTriage && findings.length === 0 ? (
          <div className="text-xs text-[var(--aria-ink-muted)]">
            请在下方输入人工修改说明后发送返修。
          </div>
        ) : null}
        {isContextBlockerGate && !isResolved ? (
          <div className="text-xs text-[var(--aria-ink-muted)]">
            请在下方输入补充上下文后发送（对应 provide_context），或选择终止
          </div>
        ) : null}
        {isResolved ? (
          <ResolutionBadge resolution={entry.resolution} />
        ) : terminateBlockReason === null && actions ? (
          // F-21：终止放行即渲染动作位——phase_mismatch 的 plan 门（context
          // blocker/author 失败/缺相位）此前整面消失，终止零通路；现在露出
          // 终止（二次确认惯例），confirm/反馈编辑器维持相位纪律不渲染。
          <div className="space-y-2">
            {actionBlockReason !== null ? (
              <p className="text-xs text-[var(--aria-ink-muted)]">
                {gateActionBlockCopy(actionBlockReason)}，可终止后重新发起
              </p>
            ) : null}
            {actionFacade === "typed" && actionBlockReason === null ? (
              <GateFeedbackEditor
                multiline={false}
                value={feedback}
                onChange={setFeedback}
                onSubmit={actions.feedback}
              />
            ) : null}
            {typedGateAwaitingCommand && actionBlockReason === null ? (
              <p className="text-xs text-[var(--aria-ink-muted)]">
                未同步门命令，将以新命令提交
              </p>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
              {isContextBlockerGate || actionBlockReason !== null ? null : (
                <button
                  type="button"
                  onClick={() => actions.confirm()}
                  className="inline-flex min-h-11 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  <Check className="h-3.5 w-3.5" aria-hidden="true" />
                  {confirmLabel}
                </button>
              )}
              <ConfirmTwiceButton
                label="终止"
                confirmLabel="确认终止"
                onConfirm={actions.terminate}
              />
            </div>
          </div>
        ) : actionBlockReason ? (
          <p className="text-xs text-[var(--aria-ink-muted)]">
            {gateActionBlockCopy(actionBlockReason)}
          </p>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function ResolutionBadge({ resolution }: { resolution?: string }) {
  if (resolution === "confirm") {
    return (
      <span className="inline-flex items-center rounded-md bg-emerald-50 px-2 py-1 text-xs font-semibold text-emerald-700 ring-1 ring-emerald-200">
        已确认
      </span>
    );
  }
  if (resolution === "request-change") {
    return (
      <span className="inline-flex items-center rounded-md bg-amber-50 px-2 py-1 text-xs font-semibold text-amber-700 ring-1 ring-amber-200">
        已要求修改
      </span>
    );
  }
  if (resolution === "terminate") {
    return (
      <span className="inline-flex items-center rounded-md bg-red-50 px-2 py-1 text-xs font-semibold text-red-700 ring-1 ring-red-200">
        已终止
      </span>
    );
  }
  return null;
}

function summaryFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.summary === "string" ? metadata.summary : null;
}

function verdictFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.verdict === "string" ? metadata.verdict : null;
}

function reviewGateFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.review_gate === "string" ? metadata.review_gate : null;
}

function gateKindFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.gate_kind === "string" ? metadata.gate_kind : null;
}

function blockReasonFromEntry(
  entry: ChatEntry,
  key: "action_block_reason" | "terminate_block_reason",
): Exclude<GateActionBlockReason, null> | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.[key];
  return value === "terminal_stage" || value === "phase_mismatch" || value === "closed"
    ? value
    : null;
}

// 通用 stage 前缀门离场兜底（k3 P2-1）：门卡不随 stage_change 重建（仅
// setStage），阶段离开后投影消失——按 gateIdentity 与当前 stage 不一致判
// terminal_stage，避免重渲染出可点但被静默拦截的假按钮（human_confirm 与
// F-20 story/design author_confirm 门同款纪律）。
function gateCardBlockReason(
  state: WorkspaceWsState,
  entry: ChatEntry,
  persistedReason: Exclude<GateActionBlockReason, null> | null,
  fromProjection: (projection: GateProjection) => Exclude<GateActionBlockReason, null> | null,
): Exclude<GateActionBlockReason, null> | null {
  const gateIdentity = (entry.metadata as Record<string, unknown> | undefined)?.gate_identity;
  if (typeof gateIdentity !== "string") {
    return persistedReason;
  }
  const projection = selectGateProjection(state);
  if (projection?.key === gateIdentity) {
    return projection.turn && state.stage !== "human_confirm"
      ? "terminal_stage"
      : fromProjection(projection);
  }
  return gateIdentity.startsWith("stage:") && gateIdentity !== `stage:${state.stage}`
    ? "terminal_stage"
    : persistedReason;
}
function gateTriggerFromEntry(
  entry: ChatEntry,
): WorkItemPlanHumanGateSnapshot["trigger"] | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const value = metadata?.gate_trigger;
  return value === "native_human_required" ||
    value === "repeated_fingerprint" ||
    value === "verification_new_findings" ||
    value === "repair_budget_exhausted"
    ? value
    : null;
}

function remainingBudgetFromEntry(entry: ChatEntry): number | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.remaining_budget === "number" ? metadata.remaining_budget : null;
}

function failureMessageFromEntry(entry: ChatEntry): string | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const failureClass =
    typeof metadata?.failure_class === "string" ? metadata.failure_class : null;
  const message = typeof metadata?.failure_message === "string" ? metadata.failure_message : null;
  if (!failureClass && !message) {
    return null;
  }
  return [failureClass, message].filter((part): part is string => Boolean(part)).join(" · ");
}

function inlineErrorFromEntry(entry: ChatEntry): { code: string; message: string } | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.inline_error;
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const { code, message } = value as Record<string, unknown>;
  return typeof code === "string" && typeof message === "string" ? { code, message } : null;
}

type ReviewFinding = {
  severity?: string;
  message: string;
  evidence?: string;
  required_action?: string;
};

function findingsFromEntry(entry: ChatEntry): ReviewFinding[] {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const findings = Array.isArray(metadata?.findings) ? metadata.findings : [];
  return findings.filter(isReviewFinding);
}

function isReviewFinding(value: unknown): value is ReviewFinding {
  if (!value || typeof value !== "object") {
    return false;
  }
  const finding = value as Record<string, unknown>;
  return typeof finding.message === "string";
}
