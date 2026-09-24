import { MessageSquareText } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";
import {
  isRequiredFindingSeverity,
  ReviewFindingGroups,
  reviewFindingsFromEntry,
} from "../finding-list";
import { structuredOutputDiagnosticFromUnknown } from "../../../state/structured-output-diagnostic";
import { StructuredOutputDiagnosticView } from "./StructuredOutputDiagnostic";

export function ReviewVerdictEntry({
  entry,
}: {
  entry: ChatEntry;
}) {
  const verdict = verdictFromEntry(entry);
  const findings = reviewFindingsFromEntry(entry);
  const diagnostic = structuredOutputDiagnosticFromUnknown(
    (entry.metadata as Record<string, unknown> | undefined)?.structured_output_diagnostic,
  );
  const optionalFindings = findings.filter(
    (finding) => !isRequiredFindingSeverity(finding.severity),
  );
  const round = verdict.round;

  return (
    <ChatEntryContainer
      role="reviewer"
      title={verdictLabel(verdict?.verdict ?? null, verdict?.reviewGate ?? null)}
      // F-49 A7：面板色必须显式覆盖 role 面板——同级同权重时 Tailwind 输出顺序决定
      // 胜负，dist 实测 green（role=reviewer）压掉作者的 amber（.border-amber-200
      // 早于 .border-green-200、.bg-amber-50 早于 .bg-green-50），结论卡与门卡系撞色。
      panelClassName="border-amber-200 bg-amber-50"
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
            {verdict?.comments && !diagnostic ? (
              <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">{verdict.comments}</div>
            ) : null}
          </div>
        </div>
        {/* F-39：轮次与可选建议条数此前不渲染——多轮评审的结论卡彼此不可辨（rebuild
            路径 metadata 缺 round，live 路径带）。轮次取自 metadata.round；可选建议
            条数与 selectLatestReviewAdvisoryCount / 下方「可选建议」分组同一判据。 */}
        {round !== null || optionalFindings.length > 0 ? (
          <div className="flex flex-wrap items-center gap-2">
            {round !== null ? (
              <span
                data-testid="review-round-label"
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
              >
                Review Round {round}
              </span>
            ) : null}
            {optionalFindings.length > 0 ? (
              <span
                data-testid="review-advisory-count"
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
              >
                可选建议 {optionalFindings.length} 条
              </span>
            ) : null}
          </div>
        ) : null}
        {diagnostic ? (
          <StructuredOutputDiagnosticView
            diagnostic={diagnostic}
            comments={verdict?.comments ?? null}
          />
        ) : null}
        {findings.length > 0 ? <ReviewFindingGroups findings={findings} /> : null}
        {/* 退役留档（T5/REQ-RET-02）：review_decision 路径按钮随消息族删除。 */}
      </div>
    </ChatEntryContainer>
  );
}

function verdictFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const verdict = typeof metadata?.verdict === "string" ? metadata.verdict : null;
  const comments = typeof metadata?.comments === "string" ? metadata.comments : null;
  const summary = typeof metadata?.summary === "string" ? metadata.summary : null;
  const reviewGate = typeof metadata?.review_gate === "string" ? metadata.review_gate : null;
  // F-39：轮次（live 路径 review_complete 与 rebuild 兜底卡均携带；缺失即不画标签）。
  const round = typeof metadata?.round === "number" ? metadata.round : null;
  return { verdict, comments, summary, reviewGate, round };
}

function verdictLabel(verdict: string | null, reviewGate: string | null) {
  if (reviewGate === "requires_revision") {
    return "需要解决后再继续";
  }
  if (reviewGate === "user_confirm_allowed") {
    return "可确认当前版本";
  }
  if (reviewGate === "user_triage_required") {
    return "需要判断 reviewer 意图";
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
