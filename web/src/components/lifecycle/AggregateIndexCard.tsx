import { LoaderCircle, TriangleAlert } from "lucide-react";
import type {
  AggregateIndexActiveResponse,
  AggregateIndexState,
  FailureCopyDto,
} from "../../api/types";

const STATE_LABELS: Record<AggregateIndexState, string> = {
  active: "索引可用",
  stale: "索引已过期",
  degraded: "索引降级",
  rebuilding: "正在重建索引",
  missing: "尚未建立索引",
};

const STATE_BADGE_CLASSES: Record<AggregateIndexState, string> = {
  active:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  stale:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
  degraded:
    "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-danger)]",
  rebuilding:
    "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]",
  missing:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

export type AggregateIndexCardProps = {
  index: AggregateIndexActiveResponse;
  rebuilding: boolean;
  onRebuild: () => void;
};

/**
 * 失败面正文（问题标题 + 原因/怎么做/机器码 + 可选诊断行）只有这一处渲染，
 * `failure_copy` 面与 `evidence_copy` 面共用，避免两份标签各写各的、逐字漂移。
 * 前缀标签的措辞与后端 `render_detail` 逐字一致；中文正文全部取自后端 copy 字段，
 * 前端不另写中文。
 */
function FailureFaceDetail({
  copy,
  titleTestId,
  diagnostic,
}: {
  copy: FailureCopyDto;
  titleTestId: string;
  diagnostic?: string | null;
}) {
  return (
    <>
      <p data-testid={titleTestId} className="font-semibold">
        {copy.title_zh}
      </p>
      <p className="mt-1">原因：{copy.reason_zh}</p>
      <p className="mt-1">怎么做：{copy.action_zh}</p>
      <p className="mt-1 font-mono opacity-80">
        机器码：{copy.machine_line}
      </p>
      {diagnostic ? (
        <p
          data-testid="aggregate-index-warning"
          className="mt-1 break-words font-mono opacity-80"
        >
          诊断：{diagnostic}
        </p>
      ) : null}
    </>
  );
}

export function AggregateIndexCard({
  index,
  rebuilding,
  onRebuild,
}: AggregateIndexCardProps) {
  const isRebuilding = rebuilding || index.state === "rebuilding";
  // 索引不可用时的成品（问题/原因/怎么做/机器码）由后端渲染，前端不另写中文。
  const evidenceCopy = index.evidence_copy ?? null;
  // 最新一代聚合索引失败时的成品同样由后端渲染，前端不另写中文。
  const failureCopy = index.failure_copy ?? null;
  // 无源码成员告警：结构化 `{code, message_zh}`，正文由后端下发，前端只渲染一行文案。
  const sourceLessAlert = index.source_less_alert ?? null;

  // 「重建索引」按钮只有一处：索引不可用时随失败面一起给（失败面自带上动作，
  // 用户不必回到面板顶部找入口），其余状态留在标题行。
  const rebuildButton = (
    <button
      type="button"
      disabled={isRebuilding}
      onClick={onRebuild}
      className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
    >
      {isRebuilding ? (
        <LoaderCircle
          data-testid="aggregate-index-spinner"
          className="mr-1 h-4 w-4 animate-spin"
          aria-hidden="true"
        />
      ) : null}
      重建索引
    </button>
  );

  return (
    <section
      data-testid="aggregate-index-card"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--aria-ink)]">
            聚合索引
          </span>
          <span
            data-testid="aggregate-index-status"
            data-state={index.state}
            className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${STATE_BADGE_CLASSES[index.state]}`}
          >
            {index.state}
          </span>
          <span className="text-xs text-[var(--aria-ink-muted)]">
            {STATE_LABELS[index.state]}
          </span>
        </div>
        {evidenceCopy || failureCopy ? null : rebuildButton}
      </div>

      {failureCopy ? (
        <div
          data-testid="aggregate-index-failure"
          className="mt-2 rounded-md border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-xs text-[var(--aria-danger)]"
        >
          <FailureFaceDetail
            copy={failureCopy}
            titleTestId="aggregate-index-failure-title"
            diagnostic={index.warning}
          />
          <div className="mt-2">{rebuildButton}</div>
        </div>
      ) : null}

      {evidenceCopy && !failureCopy ? (
        <div
          data-testid="aggregate-index-evidence-unavailable"
          className="mt-2 rounded-md border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-xs text-[var(--aria-danger)]"
        >
          <FailureFaceDetail
            copy={evidenceCopy}
            titleTestId="aggregate-index-evidence-title"
          />
          <div className="mt-2">{rebuildButton}</div>
        </div>
      ) : null}

      {index.revision !== null && index.revision !== undefined ? (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
          成员版本：{index.revision}
          {index.indexed_at ? ` · 索引时间：${index.indexed_at}` : ""}
        </p>
      ) : null}

      {/* 无源码成员告警：不是失败面，只在索引摘要下补一行中性提示；
          正文只渲染后端下发的 `message_zh`，前端不另写中文、不加动作。 */}
      {sourceLessAlert ? (
        <p
          data-testid="aggregate-index-source-less-alert"
          className="mt-2 break-words text-xs text-[var(--aria-ink-muted)]"
        >
          {sourceLessAlert.message_zh}
        </p>
      ) : null}

      {/* 陈旧提示与失败面互斥：重建失败且无可用索引时（state=stale 且带
          failure_copy）只保留失败面，避免同时出现两套「重建」动作面。 */}
      {index.state === "stale" && !failureCopy ? (
        <div
          data-testid="aggregate-index-stale-notice"
          className="mt-2 flex items-center gap-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs text-[var(--aria-warning)]"
        >
          <TriangleAlert className="h-4 w-4 shrink-0" aria-hidden="true" />
          索引已过期，请重建后再使用聚合上下文。
        </div>
      ) : null}

      {index.warning && !failureCopy ? (
        <div
          data-testid="aggregate-index-warning"
          className="mt-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs text-[var(--aria-warning)]"
        >
          {index.warning}
        </div>
      ) : null}
    </section>
  );
}
