import { ChevronRight, MessageSquareText } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";
import {
  isRequiredFindingSeverity,
  ReviewFindingGroups,
  ReviewOwnerGuidanceBlock,
  reviewFindingsFromEntry,
  reviewOwnerGuidance,
} from "../finding-list";
import { structuredOutputDiagnosticFromUnknown } from "../../../state/structured-output-diagnostic";
import { StructuredOutputDiagnosticView } from "./StructuredOutputDiagnostic";
import {
  DISCLOSURE_CHEVRON_CLASS,
  DISCLOSURE_SUMMARY_CLASS,
  GATE_CARD_CLASS,
} from "../gate-visual-tokens";

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
  // REQ-WSC-11：人话区以「是否携带人话字段」为准，不以 verdict 字面为准——
  // 模型宣称 needs_human 却被 blocking finding 归一成 revise 时，其被强制
  // 要求写的字段仍必须可见（F2）。不携带字段的 pass/revise（即全部契约合规
  // 与存量记录）呈现逐字不变。
  const ownerGuidance = reviewOwnerGuidance(findings);

  return (
    <ChatEntryContainer
      role="reviewer"
      title={verdictLabel(verdict?.verdict ?? null, verdict?.reviewGate ?? null)}
      // F-49 A7 纪律保留：面板色由本组件显式指定，不留 Tailwind 输出顺序的权重
      // 轮盘。F-50 视觉 v2 fix round 起纳入视觉规格——琥珀不再做整卡底色，与
      // 门卡同一中性常量（白底+琥珀左线）；role=reviewer 的 green 面板照旧被覆盖。
      panelClassName={GATE_CARD_CLASS}
      testId="review-verdict-entry"
    >
      <div className="space-y-3">
        <div className="flex items-start gap-2">
          <MessageSquareText className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
          <div className="min-w-0">
            <div className="text-sm font-medium text-slate-900">{entry.content}</div>
            {verdict?.summary && verdict.summary !== entry.content ? (
              <div className="mt-1 text-xs font-medium text-slate-600">{verdict.summary}</div>
            ) : null}
            {verdict?.comments && !diagnostic ? (
              <div className="mt-1 text-xs text-slate-500">{verdict.comments}</div>
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
        {ownerGuidance ? <ReviewOwnerGuidanceBlock guidance={ownerGuidance} /> : null}
        {diagnostic ? (
          <StructuredOutputDiagnosticView
            diagnostic={diagnostic}
            comments={verdict?.comments ?? null}
          />
        ) : null}
        {ownerGuidance && findings.length > 0 ? (
          // REQ-WSC-11：技术 findings 保留但不占据主区——owner 先答人话问题，
          // 需要核对原文时再展开「技术细节」。
          <details
            className="group text-xs text-[var(--aria-ink-muted)]"
            data-testid="review-technical-details"
          >
            <summary
              className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 font-medium text-slate-700`}
            >
              <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
              技术细节（{findings.length} 条审核发现原文）
            </summary>
            <div className="mt-2">
              <ReviewFindingGroups findings={findings} />
            </div>
          </details>
        ) : null}
        {!ownerGuidance && findings.length > 0 ? (
          <ReviewFindingGroups findings={findings} />
        ) : null}
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
    // F-50 fix round 标题去重：triage 短语只保留主区门卡卡头一处，结论卡自称
    // 分诊状态，不再与门卡头同文案（复拍三处同题的漏网点）。
    return "审核结论待人工分诊";
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
