import { useState } from "react";
import { confirmPlanCoding } from "../api/client";
import type { PlanCodingFinalConfirmResultDto } from "../api/types";
import type { PlanCodingFinalConfirmView } from "./plan-coding-progress";

// POST 回执逐仓状态文案（失败仓 error_code/message 原样透传，不吞原因）。
const RESULT_STATUS_LABELS: Record<string, string> = {
  confirmed: "已确认",
  already_completed: "早已完成",
  failed: "失败",
  not_attempted: "未执行",
};

/**
 * P1-b（REQ-MTG-11 / 契约 A2）：plan 级「确认完成」聚合按钮——GroupProgress
 * 聚合分支与 plan 总览标题共用。final_confirm.state ∈ {ready, partial} 且
 * targets ≥2 才出现；一次点击只发一个无 body POST，逐仓结果可见（失败仓
 * error_code/message）；完成后经 onConfirmed 通知调用方 GET 刷新真实状态
 * （不依赖新事件协议）。done 只读成功；not_ready 不渲染（blocking 原因在
 * GET 投影中，逐 attempt 原确认按钮仍保留在各自工作台，不受影响）。
 */
export function PlanCodingFinalConfirm({
  projectId,
  issueId,
  planId,
  finalConfirm,
  targetCount,
  unitCount,
  onConfirmed,
}: {
  projectId: string;
  issueId: string;
  planId: string;
  finalConfirm: PlanCodingFinalConfirmView;
  targetCount: number;
  unitCount: number;
  /** POST 落定（含部分失败）后通知调用方重拉聚合进度。 */
  onConfirmed?: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<PlanCodingFinalConfirmResultDto | null>(
    null,
  );
  const [requestError, setRequestError] = useState<string | null>(null);

  if (finalConfirm.state === "done") {
    return (
      <span
        data-testid="plan-final-confirm-done"
        className="text-[11px] font-semibold text-[var(--aria-ink-muted)]"
      >
        全部人工确认
      </span>
    );
  }

  const aggregateReady =
    finalConfirm.state === "ready" || finalConfirm.state === "partial";
  // REQ-MTG-11：聚合按钮只在 ≥2 冻结 targets 时呈现（单仓走逐 attempt 原按钮）。
  if (!aggregateReady || targetCount < 2) {
    return null;
  }

  const readyRepoCount =
    finalConfirm.state === "ready"
      ? targetCount
      : finalConfirm.readyAttemptIds.length;
  const label = `确认本计划全部完成（${readyRepoCount} 仓/${unitCount} 项）`;

  const handleConfirm = () => {
    if (busy) {
      return;
    }
    setBusy(true);
    setRequestError(null);
    confirmPlanCoding(projectId, issueId, planId)
      .then((dto) => {
        setResult(dto);
        onConfirmed?.();
      })
      .catch((reason: unknown) => {
        setRequestError(
          reason instanceof Error && reason.message
            ? reason.message
            : "聚合确认请求失败",
        );
      })
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <div className="flex min-w-0 flex-col items-start gap-1">
      <button
        type="button"
        data-testid="plan-final-confirm-button"
        disabled={busy}
        onClick={handleConfirm}
        className="inline-flex h-7 shrink-0 items-center rounded-md border border-[var(--aria-primary)] bg-white px-2 text-[11px] font-semibold text-[var(--aria-primary)] hover:bg-slate-50 disabled:opacity-50"
      >
        {busy ? "确认中…" : label}
      </button>
      {requestError ? (
        <div
          data-testid="plan-final-confirm-error"
          className="text-[11px] font-semibold text-[var(--aria-danger)]"
        >
          {requestError}
        </div>
      ) : null}
      {result ? (
        <div
          data-testid="plan-final-confirm-result"
          className="grid min-w-0 gap-0.5 text-[11px] text-[var(--aria-ink-muted)]"
        >
          <div className="font-semibold">
            {result.outcome === "all_confirmed"
              ? "全部仓确认成功"
              : result.outcome === "partially_confirmed"
                ? "部分仓确认成功（首失败即停，可重试）"
                : "未开始确认（存在未就绪仓）"}
          </div>
          <ul className="grid gap-0.5 font-mono">
            {result.results.map((item) => (
              <li
                key={item.attempt_id}
                data-testid={`plan-final-confirm-result-${item.target_repository_id}`}
                className={
                  item.status === "failed"
                    ? "text-[var(--aria-danger)]"
                    : undefined
                }
              >
                {item.target_repository_id}：
                {RESULT_STATUS_LABELS[item.status] ?? item.status}
                {item.error_code ? `（${item.error_code}）` : null}
                {item.message ? `——${item.message}` : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
