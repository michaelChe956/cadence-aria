// 失败面渲染的**唯一实现**（REQ-BOOT-08 / REQ-LAYOUT-02）。
//
// 只消费后端渲染好的结构化字段（`BootstrapFailureDto.copy` 五字段）：
// - 主行：`title_zh`（问题）——正文，不是机器码；
// - 次行：`reason_zh`（原因）与 `action_zh`（怎么做）；
// - 机器码降到**次要行**（等宽小字，`data-testid` 供断言）；
// - 原始 `detail` 收进折叠区（`<details>`）。
//
// 前端**不**另实现 describe / 码表：没有 copy 时只按既有字段降级展示
// （旧响应/损坏响应的防崩路径，不是布局失败的合法形态）。
//
// 分诊按钮化（tasks 5.6 / REQ-BOOT-08「三类分诊给出不同下一步」）：
// **环境类**失败（`copy.triage === "environment"`）额外挂一个「重新检测」
// 按钮，复用既有的 `POST /api/providers/recheck`（`api/client.ts` 的
// `recheckProviders()`），让用户修完环境后能就地复核：
// - 进行中态：按钮禁用 + 文案变「正在重新检测」，重复点击不重复发请求；
// - 成功 / 失败都有反馈（失败给原因，成功给复核结论）；
// - 复核结论里的安装指引**逐字复用**后端 `providers.rs` 的 `install_hint`
//   （随 `/api/providers/recheck` 响应下发），前端不自造安装文案。
//
// 跨块稳定扩展点：块3 在本组件之后挂 `<LayoutFailureAction/>`（同在
// `<li>` 内的兄弟表达式），因此本组件内部结构对块3 是只读契约。
import { useCallback, useRef, useState } from "react";
import { recheckProviders } from "../../api/client";
import type {
  BootstrapFailureDto,
  FailureCopyTriage,
  ProviderHealthEntry,
} from "../../api/types";

const TRIAGE_LABELS: Record<FailureCopyTriage, string> = {
  environment: "环境问题（你这边可以修）",
  product_defect: "疑似产品缺陷（可重试并反馈）",
  retry_only: "可直接重试",
};

/** 「重新检测」的反馈：结论 + 不可用 provider 的安装指引（后端原文）。 */
type RecheckFeedback = {
  ok: boolean;
  message: string;
  hints: { name: string; hint: string }[];
};

function recheckConclusion(
  providers: ProviderHealthEntry[],
): { message: string; hints: { name: string; hint: string }[] } {
  const unavailable = providers.filter((entry) => !entry.available);
  const hints = unavailable
    .filter((entry) => entry.install_hint.trim().length > 0)
    .map((entry) => ({ name: entry.display_name, hint: entry.install_hint }));
  if (providers.length === 0) {
    return { message: "重新检测完成：本次未返回 provider 检测结果。", hints };
  }
  if (unavailable.length === 0) {
    return { message: "重新检测完成：provider 全部可用，可点「重试」继续。", hints };
  }
  return {
    message: `重新检测完成：仍有 ${unavailable.length} 个 provider 不可用（${unavailable
      .map((entry) => entry.display_name)
      .join("、")}），按下面的安装指引处理后点「重试」。`,
    hints,
  };
}

export function BootstrapFailureLine({
  failure,
}: {
  failure: BootstrapFailureDto;
}) {
  const copy = failure.copy ?? null;
  const triage = copy?.triage ?? failure.triage ?? null;
  const triageLabel = triage === null ? null : (TRIAGE_LABELS[triage] ?? null);
  // 环境类（用户这边可修）才给「重新检测」；其它分诊的后端文案已写明下一步。
  const environmentFailure = triage === "environment";
  const [rechecking, setRechecking] = useState(false);
  const [feedback, setFeedback] = useState<RecheckFeedback | null>(null);
  const inFlight = useRef(false);

  const handleRecheck = useCallback(() => {
    if (inFlight.current) {
      return;
    }
    inFlight.current = true;
    setRechecking(true);
    setFeedback(null);
    void recheckProviders()
      .then((snapshot) => {
        const conclusion = recheckConclusion(snapshot.providers);
        setFeedback({ ok: true, message: conclusion.message, hints: conclusion.hints });
      })
      .catch((error: unknown) => {
        setFeedback({
          ok: false,
          message: `重新检测失败：${
            error instanceof Error
              ? error.message
              : typeof error === "string"
                ? error
                : "未知错误"
          }`,
          hints: [],
        });
      })
      .finally(() => {
        inFlight.current = false;
        setRechecking(false);
      });
  }, []);

  return (
    <div
      className="w-full text-xs leading-snug"
      data-testid="lc-bootstrap-failure"
    >
      <p
        className="font-semibold text-[var(--aria-danger)]"
        data-testid="lc-bootstrap-failure-title"
      >
        {copy ? copy.title_zh : failure.reason_code}
      </p>
      {copy ? (
        <>
          <p
            className="mt-0.5 text-[var(--aria-ink)]"
            data-testid="lc-bootstrap-failure-reason"
          >
            原因：{copy.reason_zh}
          </p>
          <p
            className="mt-0.5 text-[var(--aria-ink)]"
            data-testid="lc-bootstrap-failure-action"
          >
            怎么做：{copy.action_zh}
          </p>
        </>
      ) : null}
      <p
        className="mt-0.5 flex flex-wrap items-baseline gap-x-2 text-[var(--aria-ink-muted)]"
        data-testid="lc-bootstrap-failure-meta"
      >
        <span data-testid="lc-bootstrap-failure-machine-line">
          机器码：{copy ? copy.machine_line : failure.reason_code}
        </span>
        {triageLabel ? (
          <span data-testid="lc-bootstrap-failure-triage">{triageLabel}</span>
        ) : null}
        {failure.external_side_effect &&
        failure.external_side_effect !== "none" ? (
          <span>可能副作用：{failure.external_side_effect}</span>
        ) : null}
      </p>
      {environmentFailure ? (
        <div
          className="mt-1 flex flex-wrap items-center gap-2"
          data-testid="lc-bootstrap-failure-recheck-row"
        >
          <button
            type="button"
            disabled={rechecking}
            onClick={handleRecheck}
            className="inline-flex min-h-8 cursor-pointer items-center justify-center rounded-md bg-[var(--aria-primary)] px-2.5 py-1 text-xs font-semibold text-white transition-colors hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
            data-testid="lc-bootstrap-failure-recheck"
          >
            {rechecking ? "正在重新检测" : "重新检测"}
          </button>
          {feedback ? (
            <p
              className={
                feedback.ok
                  ? "text-[var(--aria-ink-muted)]"
                  : "text-[var(--aria-danger)]"
              }
              role={feedback.ok ? undefined : "alert"}
              data-testid="lc-bootstrap-failure-recheck-feedback"
            >
              {feedback.message}
            </p>
          ) : null}
          {feedback && feedback.hints.length > 0 ? (
            <ul
              className="w-full space-y-0.5 text-[var(--aria-ink-muted)]"
              data-testid="lc-bootstrap-failure-recheck-hints"
            >
              {feedback.hints.map((hint) => (
                <li
                  key={hint.name}
                  data-testid={`lc-bootstrap-failure-recheck-hint-${hint.name}`}
                >
                  {hint.name}：{hint.hint}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
      {failure.detail ? (
        <details
          className="mt-0.5 text-[var(--aria-ink-muted)]"
          data-testid="lc-bootstrap-failure-detail"
        >
          <summary className="cursor-pointer select-none">诊断信息</summary>
          <pre className="mt-0.5 whitespace-pre-wrap break-words font-mono text-[11px]">
            {failure.detail}
          </pre>
        </details>
      ) : null}
    </div>
  );
}
