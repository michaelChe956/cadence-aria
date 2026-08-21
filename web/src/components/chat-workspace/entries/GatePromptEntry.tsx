import { Check, RotateCcw, X } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND } from "../../../state/workspace-chat-rebuild";
import { trustedReviewComments } from "../../../state/workspace-review-trust";
import { ChatEntryContainer } from "../ChatEntryContainer";

type HumanConfirmPayload = { description: string; source: "human" | "review_findings" };
type HumanConfirmDecision = "confirm" | "request-change" | "terminate";

export function GatePromptEntry({
  entry,
  onDecision,
}: {
  entry: ChatEntry;
  onDecision?: (decision: HumanConfirmDecision, payload?: HumanConfirmPayload) => void;
}) {
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
  const isContextBlockerGate = gateKindFromEntry(entry) === WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND;
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
        ) : onDecision ? (
          <div className="flex flex-wrap justify-end gap-2">
            {isContextBlockerGate ? null : (
              <button
                type="button"
                onClick={() => onDecision("confirm")}
                className="inline-flex h-8 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50"
              >
                <Check className="h-3.5 w-3.5" />
                {confirmLabel}
              </button>
            )}
            {requestChangeLabel ? (
              <button
                type="button"
                onClick={() => onDecision("request-change", requestChangePayload(entry))}
                className="inline-flex h-8 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50"
              >
                <RotateCcw className="h-3.5 w-3.5" />
                {requestChangeLabel}
              </button>
            ) : null}
            <button
              type="button"
              onClick={() => onDecision("terminate")}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-50"
            >
              <X className="h-3.5 w-3.5" />
              终止
            </button>
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

type ReviewFinding = {
  severity?: string;
  message: string;
  evidence?: string;
  required_action?: string;
};

function requestChangePayload(entry: ChatEntry): HumanConfirmPayload {
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
