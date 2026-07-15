import { AlertTriangle, CheckCircle2 } from "lucide-react";
import type { StructuredOutputDiagnostic } from "../../../api/types";

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

  return (
    <div
      className="space-y-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2"
      role="alert"
    >
      <div className="flex items-start gap-2">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-700" />
        <div className="space-y-1">
          <div className="text-sm font-semibold text-amber-950">结构化审核结果解析失败</div>
          <div className="text-xs text-amber-900">{diagnostic.message}</div>
          <div className="text-xs text-amber-800">
            {diagnostic.repair_attempted
              ? "系统已自动修复 1 次，仍未成功。"
              : "系统未尝试自动修复，请人工检查原始审核输出。"}
          </div>
        </div>
      </div>
      {comments ? <ReviewerComments comments={comments} /> : null}
      {typeof diagnostic.raw_output_preview === "string" ? (
        <details className="text-xs text-[var(--aria-ink-muted)]">
          <summary className="cursor-pointer font-medium text-[var(--aria-ink)]">
            查看原始输出片段
          </summary>
          <pre
            className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-white p-2 font-mono text-xs"
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
    <details className="text-xs text-[var(--aria-ink-muted)]">
      <summary className="cursor-pointer font-medium text-[var(--aria-ink)]">
        查看 Reviewer comments
      </summary>
      <div className="mt-2 whitespace-pre-wrap">{comments}</div>
    </details>
  );
}
