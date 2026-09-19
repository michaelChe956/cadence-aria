import { GitBranch } from "lucide-react";
import type { PlanGroupProjectionDto, PlanTargetEntryDto } from "../../api/types";

// REQ-MTG-04（WP3）：plan 级 group 聚合只读投影面板。三值终态徽标 + per-target
// 行（复用 DeliveryStatusPanel 行风格）+ partial failure 未满足明细。
// 聚合区禁用一切动作元素（决策/启动/中止走各 attempt 既有入口——本面板零
// 按钮/零命令发送）。

const OVERALL_LABELS: Record<PlanGroupProjectionDto["overall"], string> = {
  all_delivered: "已全部交付",
  partial: "部分交付",
  not_started: "未启动",
};

const OVERALL_BADGE_CLASSES: Record<PlanGroupProjectionDto["overall"], string> = {
  all_delivered:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  partial:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
  not_started:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

const ATTEMPT_STATUS_LABELS: Record<string, string> = {
  created: "已创建",
  running: "执行中",
  waiting_for_human: "等待人工",
  blocked: "已阻塞",
  awaiting_manual_recovery: "等待人工恢复",
  awaiting_plan_amendment: "等待计划修订",
  applying_plan_amendment: "应用计划修订",
  amendment_apply_failed: "修订应用失败",
  completed: "已完成",
  failed: "已失败",
  aborted: "已中止",
};

const STAGE_LABELS: Record<string, string> = {
  prepare_context: "准备上下文",
  worktree_prepare: "准备 Worktree",
  coding: "编码",
  code_review: "代码评审",
  review_request: "评审请求",
  internal_pr_review: "内部 PR 评审",
  final_confirm: "最终确认",
};

const PUSH_STATUS_LABELS: Record<string, string> = {
  pushed: "已推送",
  not_pushed: "未推送",
  failed: "推送失败",
};

export function PlanGroupProjectionPanel({
  projection,
}: {
  projection: PlanGroupProjectionDto;
}) {
  return (
    <section
      data-testid="plan-group-projection-panel"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="mb-2 flex items-center gap-2">
        <span
          data-testid="plan-group-overall-badge"
          data-status={projection.overall}
          className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${OVERALL_BADGE_CLASSES[projection.overall]}`}
        >
          {OVERALL_LABELS[projection.overall]}
        </span>
        <span className="text-xs text-[var(--aria-ink-muted)]">
          Group 聚合投影（只读）
        </span>
      </div>
      {projection.entries.length > 0 ? (
        <div className="space-y-2">
          {projection.entries.map((entry) => (
            <PlanTargetEntryRow
              key={entry.target_repository_id}
              entry={entry}
              overall={projection.overall}
            />
          ))}
        </div>
      ) : (
        <p
          data-testid="plan-group-empty-hint"
          className="text-xs text-[var(--aria-ink-muted)]"
        >
          尚无 target-attempt（未启动）
        </p>
      )}
    </section>
  );
}

function PlanTargetEntryRow({
  entry,
  overall,
}: {
  entry: PlanTargetEntryDto;
  overall: PlanGroupProjectionDto["overall"];
}) {
  const delivered =
    entry.attempt_status === "completed" && entry.push_status === "pushed";
  const pushFailed = entry.push_status === "failed";
  const failed =
    entry.attempt_status === "failed" ||
    entry.attempt_status === "aborted" ||
    entry.attempt_status === "amendment_apply_failed";
  const rowFailed = failed || pushFailed;

  // 未满足原因显式（partial failure 不伪装全局成功）：blocked_reason 优先，
  // 缺席时按推送/完成事实派生。
  const unmetReason =
    entry.blocked_reason ??
    (pushFailed
      ? "推送失败"
      : entry.attempt_status === "completed" && entry.push_status === null
        ? "无评审请求"
        : entry.attempt_status === "completed"
          ? "未推送"
          : "未完成");

  return (
    <div
      data-testid="plan-target-entry-row"
      data-status={delivered ? "delivered" : rowFailed ? "failed" : "pending"}
      className={`rounded-md border px-2 py-2 text-xs ${
        rowFailed
          ? "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)]"
          : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-semibold text-[var(--aria-ink)]">
          {entry.repository_name}
        </span>
        <span
          className={
            delivered
              ? "font-semibold text-[var(--aria-success)]"
              : rowFailed
                ? "font-semibold text-[var(--aria-danger)]"
                : "text-[var(--aria-ink-muted)]"
          }
        >
          {delivered ? "已交付" : "未交付"}
        </span>
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[var(--aria-ink-muted)]">
        <span>
          {entry.attempt_status === null
            ? "无 attempt"
            : (ATTEMPT_STATUS_LABELS[entry.attempt_status] ?? entry.attempt_status)}
          {" · "}
          {entry.stage === null
            ? "—"
            : (STAGE_LABELS[entry.stage] ?? entry.stage)}
        </span>
        <span>
          {entry.push_status === null
            ? "无评审请求"
            : (PUSH_STATUS_LABELS[entry.push_status] ?? entry.push_status)}
        </span>
      </div>
      <div className="mt-1 flex items-center gap-2 text-[var(--aria-ink-muted)]">
        <GitBranch className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{entry.branch_name ?? "—"}</span>
        {entry.head_commit ? (
          <span className="shrink-0 font-mono text-[11px]">
            {entry.head_commit.slice(0, 7)}
          </span>
        ) : null}
      </div>
      {overall === "partial" && !delivered ? (
        <div
          data-testid="plan-target-unmet-reason"
          className={`mt-1 ${rowFailed ? "text-[var(--aria-danger)]" : "text-[var(--aria-ink-muted)]"}`}
        >
          未满足：{unmetReason}
        </div>
      ) : null}
      {entry.blocked_reason && overall !== "partial" ? (
        <div
          data-testid="plan-target-blocked-reason"
          className="mt-1 text-[var(--aria-danger)]"
        >
          {entry.blocked_reason}
        </div>
      ) : null}
    </div>
  );
}
