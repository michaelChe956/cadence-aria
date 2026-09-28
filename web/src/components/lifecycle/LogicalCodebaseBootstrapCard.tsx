// C4 Task 9（LC 冷启动加固）：冷启动五步等待卡——只读 bootstrap 纯投影，
// 展示每步状态/失败原因/可能外部副作用/下一步，并按 durable notice 的
// allowed_actions 调用统一 action API（准备/继续/重试/核验）。
// 按钮生成稳定 command id（notice key + action 派生），成功/重放后刷新
// 同一 bootstrap GET；repair 动作提示去身份修复诊断（mapping 裁决表单）。
import { useState } from "react";
import { postLogicalCodebaseBootstrapAction } from "../../api/logical-codebase-bootstrap";
import type {
  BootstrapActionName,
  LogicalCodebaseBootstrapProjection,
} from "../../api/types";

const STEP_LABELS: Record<string, string> = {
  identity: "身份",
  manifest_checkout: "清单/检出",
  rules_policy: "规则/政策",
  member_index: "成员索引",
  aggregate_index_active: "聚合索引激活",
};

const STATUS_LABELS: Record<string, string> = {
  not_started: "未开始",
  running: "进行中",
  completed: "已完成",
  failed: "失败",
  waiting_for_human: "等待人工",
};

const ACTION_BUTTON_LABELS: Record<BootstrapActionName, string> = {
  prepare: "准备",
  continue: "继续",
  retry: "重试",
  revalidate: "核验",
  repair: "去修复",
};

type LogicalCodebaseBootstrapCardProps = {
  projectId: string | null;
  projection: LogicalCodebaseBootstrapProjection | null;
  onChanged?: () => void;
};

export function LogicalCodebaseBootstrapCard({
  projectId,
  projection,
  onChanged,
}: LogicalCodebaseBootstrapCardProps) {
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  if (projection === null || projectId === null) {
    return null;
  }
  const current = projection;

  async function handleAction(step: string, action: BootstrapActionName, noticeKey: string) {
    if (projectId === null) {
      return;
    }
    // 稳定 command id：同 notice+action 重放幂等（服务端按 command key
    // 返回同一 durable 结果，不重复推进/不重复 provider turn）。
    const commandId = `cmd-lc-bootstrap-${noticeKey}-${action}`.replace(
      /[^a-zA-Z0-9_-]/g,
      "_",
    );
    setBusyAction(`${noticeKey}:${action}`);
    setActionError(null);
    try {
      await postLogicalCodebaseBootstrapAction(
        projectId,
        current.logical_codebase_id,
        {
          command_id: commandId,
          step: step as LogicalCodebaseBootstrapProjection["steps"][number]["step"],
          action,
          expected_revision: current.membership_revision ?? null,
          expected_object_id:
            current.steps.find((candidate) => candidate.step === step)?.object_id ??
            current.logical_codebase_id,
        },
      );
      onChanged?.();
    } catch (error) {
      // 失败如实展示，等待事实保留（下轮 GET 刷新仍可见）。
      setActionError(error instanceof Error ? error.message : "动作失败");
    } finally {
      setBusyAction(null);
    }
  }

  return (
    <div
      data-testid="lc-bootstrap-card"
      className="border-b border-[var(--aria-line)] px-3 py-2"
    >
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold text-[var(--aria-ink)]">冷启动状态</h3>
        <span
          data-testid="lc-bootstrap-planning-ready"
          className={
            projection.planning_ready
              ? "rounded bg-emerald-100 px-2 py-0.5 text-xs font-semibold text-emerald-700"
              : "rounded bg-amber-100 px-2 py-0.5 text-xs font-semibold text-amber-700"
          }
        >
          {projection.planning_ready ? "PlanningReady" : "冷启动未完成"}
        </span>
      </div>
      <ul className="mt-2 space-y-1">
        {projection.steps.map((step) => (
          <li
            key={step.step}
            data-testid={`lc-bootstrap-step-${step.step}`}
            className="flex flex-wrap items-baseline gap-x-2 text-xs text-[var(--aria-ink-muted)]"
          >
            <span className="font-semibold text-[var(--aria-ink)]">
              {STEP_LABELS[step.step] ?? step.step}
            </span>
            <span data-testid={`lc-bootstrap-status-${step.step}`}>
              {STATUS_LABELS[step.status] ?? step.status}
            </span>
            {step.failure ? (
              <span className="text-[var(--aria-danger)]">
                原因 {step.failure.reason_code}：{step.failure.detail}
              </span>
            ) : null}
            {step.status === "failed" || step.status === "waiting_for_human" ? (
              <span>
                {step.failure?.external_side_effect &&
                step.failure.external_side_effect !== "none"
                  ? `可能副作用：${step.failure.external_side_effect}`
                  : null}
              </span>
            ) : null}
          </li>
        ))}
      </ul>
      {projection.notices.length > 0 ? (
        <div className="mt-2 space-y-2">
          {projection.notices.map((notice) => (
            <div
              key={notice.key}
              data-testid={`lc-bootstrap-notice-${notice.step}`}
              className="rounded border border-amber-200 bg-amber-50 px-2 py-1.5"
            >
              <p className="text-xs font-semibold text-amber-800">
                {notice.reason_code}
              </p>
              <p className="mt-0.5 text-xs text-slate-600">{notice.summary}</p>
              {notice.external_side_effect &&
              notice.external_side_effect !== "none" ? (
                <p className="mt-0.5 text-xs text-[var(--aria-danger)]">
                  可能外部副作用：{notice.external_side_effect}
                </p>
              ) : null}
              {notice.next_step ? (
                <p className="mt-0.5 text-xs text-slate-500">
                  下一步：{STEP_LABELS[notice.next_step] ?? notice.next_step}
                </p>
              ) : null}
              {notice.allowed_actions.length > 0 ? (
                <div className="mt-1.5 flex flex-wrap gap-2">
                  {notice.allowed_actions.map((action) => (
                    <button
                      key={action}
                      type="button"
                      disabled={busyAction !== null}
                      onClick={() => void handleAction(notice.step, action, notice.key)}
                      className="min-h-9 rounded-md border border-amber-300 bg-white px-2.5 py-1 text-xs font-semibold text-amber-700 transition-colors duration-200 hover:bg-amber-100 disabled:cursor-not-allowed disabled:opacity-60"
                      data-testid={`lc-bootstrap-card-action-${action}`}
                    >
                      {ACTION_BUTTON_LABELS[action] ?? action}
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          ))}
        </div>
      ) : null}
      {actionError ? (
        <p data-testid="lc-bootstrap-action-error" className="mt-1 text-xs text-[var(--aria-danger)]">
          {actionError}
        </p>
      ) : null}
    </div>
  );
}
