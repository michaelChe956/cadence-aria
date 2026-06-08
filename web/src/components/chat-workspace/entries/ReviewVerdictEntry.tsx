import { MessageSquareText } from "lucide-react";
import type { RevisionPath } from "../../../api/types";
import type { ChatEntry } from "../../../state/chat-entries";
import { ReviewDecisionActions } from "../ReviewDecisionActions";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ReviewVerdictEntry({
  entry,
  onSelectPath,
}: {
  entry: ChatEntry;
  onSelectPath?: (path: RevisionPath, extraContext?: string) => void;
}) {
  const verdict = verdictFromEntry(entry);
  const findings = findingsFromEntry(entry);
  const requiredFindings = findings.filter(isRequiredFinding);
  const optionalFindings = findings.filter((finding) => !isRequiredFinding(finding));

  return (
    <ChatEntryContainer
      role="reviewer"
      title={verdictLabel(verdict?.verdict ?? null, verdict?.reviewGate ?? null)}
      className="border-amber-200 bg-amber-50"
      testId="review-verdict-entry"
    >
      <div className="space-y-3">
        <div className="flex items-start gap-2">
          <MessageSquareText className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
          <div className="min-w-0">
            <div className="text-sm font-medium text-[var(--aria-ink)]">{entry.content}</div>
            {verdict?.summary && verdict.summary !== entry.content ? (
              <div className="mt-1 text-xs font-medium text-amber-900">{verdict.summary}</div>
            ) : null}
            {verdict?.comments ? (
              <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">{verdict.comments}</div>
            ) : null}
          </div>
        </div>
        {requiredFindings.length > 0 ? (
          <FindingGroup title="需要解决" tone="required" findings={requiredFindings} />
        ) : null}
        {optionalFindings.length > 0 ? (
          <FindingGroup title="可选建议" tone="optional" findings={optionalFindings} />
        ) : null}
        {onSelectPath ? <ReviewDecisionActions onSelectPath={onSelectPath} /> : null}
      </div>
    </ChatEntryContainer>
  );
}

type ReviewFinding = {
  severity: string;
  message: string;
  evidence?: string;
  impact?: string;
  required_action?: string;
};

function verdictFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const verdict = typeof metadata?.verdict === "string" ? metadata.verdict : null;
  const comments = typeof metadata?.comments === "string" ? metadata.comments : null;
  const summary = typeof metadata?.summary === "string" ? metadata.summary : null;
  const reviewGate = typeof metadata?.review_gate === "string" ? metadata.review_gate : null;
  return { verdict, comments, summary, reviewGate };
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
  return typeof finding.severity === "string" && typeof finding.message === "string";
}

function isRequiredFinding(finding: ReviewFinding) {
  return (
    finding.severity === "blocking" ||
    finding.severity === "must_fix" ||
    finding.severity === "strong_recommend_fix"
  );
}

function FindingGroup({
  title,
  tone,
  findings,
}: {
  title: string;
  tone: "required" | "optional";
  findings: ReviewFinding[];
}) {
  const titleClass = tone === "required" ? "text-amber-900" : "text-[var(--aria-ink-muted)]";
  const borderClass = tone === "required" ? "border-amber-200" : "border-[var(--aria-line)]";
  return (
    <section className={`space-y-2 rounded-md border ${borderClass} bg-white px-3 py-2`}>
      <div className={`text-xs font-semibold ${titleClass}`}>{title}</div>
      <div className="space-y-2">
        {findings.map((finding, index) => (
          <div key={`${finding.severity}-${index}`} className="space-y-1">
            <div className="text-sm font-medium text-[var(--aria-ink)]">{finding.message}</div>
            {finding.evidence ? (
              <div className="text-xs text-[var(--aria-ink-muted)]">{finding.evidence}</div>
            ) : null}
            {finding.impact ? (
              <div className="text-xs text-[var(--aria-ink-muted)]">{finding.impact}</div>
            ) : null}
            {finding.required_action ? (
              <div className="text-xs font-medium text-[var(--aria-ink)]">
                {finding.required_action}
              </div>
            ) : null}
          </div>
        ))}
      </div>
    </section>
  );
}

function verdictLabel(verdict: string | null, reviewGate: string | null) {
  if (reviewGate === "requires_revision") {
    return "需要解决后再继续";
  }
  if (reviewGate === "user_confirm_allowed") {
    return "可确认当前版本";
  }
  if (verdict === "pass") {
    return "通过";
  }
  if (verdict === "revise") {
    return "建议返修";
  }
  if (verdict === "needs_human") {
    return "需要人工确认";
  }
  return "审核结论";
}
