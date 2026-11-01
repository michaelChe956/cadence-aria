import { AlertTriangle, CheckCircle2, ChevronRight } from "lucide-react";
import type { StructuredOutputDiagnostic } from "../../../api/types";
import {
  DISCLOSURE_CHEVRON_CLASS,
  DISCLOSURE_SUMMARY_CLASS,
  GATE_ALERT_SUBCARD_CLASS,
} from "../gate-visual-tokens";

export function StructuredOutputDiagnosticView({
  diagnostic,
  comments,
}: {
  diagnostic: StructuredOutputDiagnostic;
  comments: string | null;
}) {
  if (diagnostic.repair_succeeded) {
    return (
      <div className="space-y-2 rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2">
        <div className="flex items-center gap-2 text-xs font-medium text-emerald-800">
          <CheckCircle2 className="h-4 w-4 shrink-0" />
          <span>结构化输出已自动修复</span>
        </div>
        {comments ? <ReviewerComments comments={comments} /> : null}
      </div>
    );
  }

  // F-50 视觉 v2 §2：卡内告警子卡改左线式——琥珀只上 4px 左线与图标，正文回
  // slate 系，不再琥珀底叠琥珀底；与正式门卡视觉区分（告警=信息，门卡=决策）。
  return (
    <div
      className={`space-y-2 rounded-md ${GATE_ALERT_SUBCARD_CLASS}`}
      role="alert"
    >
      <div className="flex items-start gap-2">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        <div className="space-y-1">
          <div className="text-sm font-semibold text-slate-900">结构化审核结果解析失败</div>
          <div className="text-xs text-slate-600">{diagnostic.message}</div>
          <div className="text-xs text-slate-600">
            {diagnostic.repair_attempted
              ? "系统已自动修复 1 次，仍未成功。"
              : "系统未尝试自动修复，请人工检查原始审核输出。"}
          </div>
        </div>
      </div>
      {comments ? <ReviewerComments comments={comments} /> : null}
      {typeof diagnostic.raw_output_preview === "string" ? (
        <details className="group text-xs text-slate-500">
          <summary
            className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 font-medium text-slate-700`}
          >
            <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
            查看原始输出片段
          </summary>
          <pre
            className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-gray-50 p-2 font-mono text-xs"
            data-testid="structured-output-raw-preview"
          >
            {diagnostic.raw_output_preview}
          </pre>
        </details>
      ) : null}
    </div>
  );
}

function ReviewerComments({ comments }: { comments: string }) {
  return (
    <details className="group text-xs text-slate-500">
      <summary
        className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 font-medium text-slate-700`}
      >
        <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
        查看 Reviewer comments
      </summary>
      <div className="mt-2 whitespace-pre-wrap text-slate-600">{comments}</div>
    </details>
  );
}
