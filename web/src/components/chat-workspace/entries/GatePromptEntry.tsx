import { Check, RotateCcw, Send, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { ChatEntry } from "../../../state/chat-entries";
import { GATE_TRIGGER_LABELS } from "../../../state/workspace-cockpit-projection";
import type { WorkItemPlanHumanGateSnapshot } from "../../../api/types";
import { WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND } from "../../../state/workspace-chat-rebuild";
import type {
  CockpitActionFacade,
  CockpitRequestChangePayload,
} from "../../../state/cockpit-action-routing";
import { trustedReviewComments } from "../../../state/workspace-review-trust";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function GatePromptEntry({
  entry,
  actions,
}: {
  entry: ChatEntry;
  actions?: CockpitActionFacade;
}) {
  const [pendingTerminate, setPendingTerminate] = useState(false);
  const [feedback, setFeedback] = useState("");

  useEffect(() => {
    if (!pendingTerminate) {
      return;
    }
    const timer = window.setTimeout(() => setPendingTerminate(false), 10_000);
    return () => window.clearTimeout(timer);
  }, [pendingTerminate]);
  const summary = summaryFromEntry(entry);
  const verdict = verdictFromEntry(entry);
  const reviewGate = reviewGateFromEntry(entry);
  const findings = findingsFromEntry(entry);
  const needsHuman = verdict === "needs_human";
  const requiresTriage = reviewGate === "user_triage_required";
  const allowsCurrentVersion = reviewGate === "user_confirm_allowed";
  const canAdoptSuggestions = findings.length > 0;
  const confirmLabel =
    requiresTriage
      ? "确认当前版本"
      : allowsCurrentVersion
      ? "确认使用当前版本"
      : needsHuman
        ? "提交人工确认"
        : "确认产物";
  const requestChangeLabel = canAdoptSuggestions ? "采纳建议并返修" : null;
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
        ) : actions ? (
          <div className="space-y-2">
            {actionFacade === "typed" && !typedGateAwaitingCommand ? (
              <div className="flex flex-wrap items-center gap-2">
                <label className="sr-only" htmlFor={`gate-feedback-${entry.id}`}>
                  反馈内容
                </label>
                <input
                  id={`gate-feedback-${entry.id}`}
                  value={feedback}
                  onChange={(event) => setFeedback(event.target.value)}
                  placeholder="请输入反馈内容"
                  className="min-h-11 min-w-0 flex-1 rounded-md border border-[var(--aria-line-strong)] bg-white px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                />
                <button
                  type="button"
                  disabled={!feedback.trim()}
                  onClick={() => actions.feedback(feedback)}
                  className="inline-flex min-h-11 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  <Send className="h-3.5 w-3.5" aria-hidden="true" />
                  提交反馈
                </button>
              </div>
            ) : null}
            {typedGateAwaitingCommand ? (
              <p className="text-xs text-[var(--aria-ink-muted)]">
                等待门禁命令同步后再提交反馈
              </p>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
              {isContextBlockerGate ? null : (
                <button
                  type="button"
                  onClick={() => actions.confirm()}
                  className="inline-flex min-h-11 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  <Check className="h-3.5 w-3.5" aria-hidden="true" />
                  {confirmLabel}
                </button>
              )}
              {actionFacade !== "typed" && requestChangeLabel ? (
                <button
                  type="button"
                  onClick={() => actions.requestChange(requestChangePayload(entry))}
                  className="inline-flex min-h-11 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />
                  {requestChangeLabel}
                </button>
              ) : null}
              <button
                type="button"
                onClick={() => {
                  if (pendingTerminate) {
                    actions.terminate();
                    setPendingTerminate(false);
                    return;
                  }
                  setPendingTerminate(true);
                }}
                className="inline-flex min-h-11 items-center gap-1 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              >
                <X className="h-3.5 w-3.5" aria-hidden="true" />
                {pendingTerminate ? "确认终止" : "终止"}
              </button>
            </div>
          </div>
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

function requestChangePayload(entry: ChatEntry): CockpitRequestChangePayload {
  return { description: requestChangeDescription(entry), source: "review_findings" };
}

function requestChangeDescription(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const summary = summaryFromEntry(entry);
  const comments = trustedReviewComments(metadata);
  const findings = findingsFromEntry(entry);
  const reviewGate = reviewGateFromEntry(entry);
  if (findings.length > 0) {
    return formatFindingsForRevision(findings);
  }

  const sections: string[] = [];

  if (summary) {
    sections.push(`Review 摘要：${summary}`);
  }
  if (comments) {
    sections.push(`Review 意见：${comments}`);
  }
  if (findings.length > 0) {
    sections.push(
      [
        "Review findings：",
        ...findings.map((finding) => {
          const details = [
            finding.message,
            finding.required_action ? `处理建议：${finding.required_action}` : "",
          ].filter(Boolean);
          return `- ${details.join("；")}`;
        }),
      ].join("\n"),
    );
  }

  return sections.join("\n\n").trim() || entry.content;
}

function formatFindingsForRevision(findings: ReviewFinding[]) {
  return [
    "Review findings：",
    ...findings.map((finding) => {
      const details = [
        finding.message,
        finding.required_action ? `处理建议：${finding.required_action}` : "",
      ].filter(Boolean);
      return `- ${details.join("；")}`;
    }),
  ].join("\n");
}

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
