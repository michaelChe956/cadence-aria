// F-39/F-49 结论卡与门卡共用的 findings 呈现（单一事实源）：severity 分级、分组
// 标题、徽章与行的样式都只此一份，避免门卡按「同款样式」抄出第二套后漂移。
import type { ChatEntry } from "../../state/chat-entries";

export type ReviewFinding = {
  severity: string;
  message: string;
  evidence?: string;
  required_action?: string;
};

export function reviewFindingsFromEntry(entry: ChatEntry): ReviewFinding[] {
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

/** 需人工处理（高）：blocking / must_fix；其余（含 suggestion）视为建议级。 */
export function isRequiredFindingSeverity(severity: string) {
  return severity === "blocking" || severity === "must_fix";
}

export function ReviewFindingGroups({ findings }: { findings: ReviewFinding[] }) {
  const requiredFindings = findings.filter((finding) =>
    isRequiredFindingSeverity(finding.severity),
  );
  const optionalFindings = findings.filter(
    (finding) => !isRequiredFindingSeverity(finding.severity),
  );
  return (
    <>
      {requiredFindings.length > 0 ? (
        <FindingGroup title="需要解决" tone="required" findings={requiredFindings} />
      ) : null}
      {optionalFindings.length > 0 ? (
        <FindingGroup title="可选建议" tone="optional" findings={optionalFindings} />
      ) : null}
    </>
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
          <div
            key={`${finding.severity}-${index}`}
            className="space-y-1"
            data-testid="review-finding"
          >
            <div className="flex flex-wrap items-center gap-2">
              <SeverityBadge severity={finding.severity} />
              <div className="text-sm font-medium text-[var(--aria-ink)]">{finding.message}</div>
            </div>
            {finding.evidence ? (
              <div className="text-xs text-[var(--aria-ink-muted)]">{finding.evidence}</div>
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

function SeverityBadge({ severity }: { severity: string }) {
  const label = severityLabel(severity);
  const toneClass = isRequiredFindingSeverity(severity)
    ? "border-amber-200 bg-amber-50 text-amber-800"
    : "border-slate-200 bg-slate-50 text-slate-600";
  return (
    <span
      className={`inline-flex shrink-0 rounded border px-1.5 py-0.5 text-[11px] font-semibold ${toneClass}`}
      title={severity}
    >
      {label}
    </span>
  );
}

function severityLabel(severity: string) {
  switch (severity) {
    case "blocking":
      return "高 · 阻塞";
    case "must_fix":
      return "高 · 必须修复";
    case "suggestion":
      return "低 · 建议";
    default:
      return severity;
  }
}
