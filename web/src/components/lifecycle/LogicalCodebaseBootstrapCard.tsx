// C4 Task 9（LC 冷启动加固）：冷启动五步等待卡——只读 bootstrap 纯投影，
// 展示每步状态/失败原因/可能外部副作用/下一步，并按 durable notice 的
// allowed_actions 调用统一 action API（准备/继续/重试/核验）。
// 按钮生成稳定 command id（notice key + action 派生），成功/重放后刷新
// 同一 bootstrap GET；repair 动作提示去身份修复诊断（mapping 裁决表单）。
//
// 2026-10-29（LC 冷启动自动推进）：步骤自身的 allowed_actions 也直接暴露
// 为可点按钮（此前只有 notice 带动作时才可见，后三步「未开始」时无任何
// 入口）；同一 (step, action) 只渲染一个按钮——优先级：规则漂移显眼提示 >
// notice > 步骤行。另提供「一键推进」（POST .../bootstrap/advance）作为自动
// 推进失败后的兼底入口——幂等端点，重复调用不重复推进。
// 规则漂移（policy/rule/reference digest 漂移）另给一条显眼提示：provider
// 启动会被 PolicyDrift 拒，必须显式「重新生成政策」。
import { useState } from "react";
import {
  postLogicalCodebaseBootstrapAction,
  postLogicalCodebaseBootstrapAdvance,
} from "../../api/logical-codebase-bootstrap";
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
  republish: "重新生成政策",
};

// 规则漂移（provider 启动会被 PolicyDrift 拒）的三种稳定 failure.reason_code
// ——后端 bootstrap_projector 的既有事实，前端只做显眼提示，不改判定。
const RULE_DRIFT_DETAILS: Record<string, string> = {
  policy_digest_drift: "政策正文变了：已发布政策的 digest 与磁盘上的当前政策正文不一致。",
  rule_digest_drift:
    "根规则入口变了：AGENTS.md / CLAUDE.md 的 digest 与政策签发时冻结的根规则不一致。",
  policy_reference_drift: "政策引用漂移：政策引用的权威来源已变更，需重新签发引用。",
};

type RuleDriftNotice = {
  step: LogicalCodebaseBootstrapProjection["steps"][number]["step"];
  reasonCode: string;
  detail: string;
  /** 幂等 command id 的来源 key：同步骤的 notice key，否则步骤稳定 object_id。 */
  actionKey: string;
};

/**
 * 规则漂移检测：rules_policy 等待人工且失败原因是政策/规则/引用漂移
 * （或该步允许 republish）→ 需要显式「重新生成政策」。
 */
function detectRuleDrift(
  projection: LogicalCodebaseBootstrapProjection,
): RuleDriftNotice | null {
  if (projection.planning_ready) {
    return null;
  }
  const step = projection.steps.find(
    (candidate) => candidate.step === "rules_policy",
  );
  if (!step || step.status !== "waiting_for_human") {
    return null;
  }
  const reasonCode = step.failure?.reason_code ?? "";
  const allowsRepublish = (step.allowed_actions ?? []).includes("republish");
  if (!RULE_DRIFT_DETAILS[reasonCode] && !allowsRepublish) {
    return null;
  }
  const notice = projection.notices.find(
    (candidate) => candidate.step === "rules_policy",
  );
  return {
    step: step.step,
    reasonCode,
    detail:
      RULE_DRIFT_DETAILS[reasonCode] ??
      "聚合政策与磁盘上的规则不一致，需按当前规则重新签发政策。",
    actionKey: notice?.key ?? step.object_id,
  };
}

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
  const [advanceBusy, setAdvanceBusy] = useState(false);
  const [advanceSummary, setAdvanceSummary] = useState<string | null>(null);

  if (projection === null || projectId === null) {
    return null;
  }
  const current = projection;

  // 同一步骤的动作去重：同一 (step, action) 只渲染一个按钮。
  // 优先级：显眼提示（规则漂移）> notice（带失败原因与副作用说明）> 步骤行。
  const ruleDrift = detectRuleDrift(current);
  const bannerClaims = new Set<string>();
  if (ruleDrift) {
    bannerClaims.add(`${ruleDrift.step}:republish`);
  }
  const noticeActionsByStep = new Map<string, Set<BootstrapActionName>>();
  for (const notice of current.notices) {
    const rendered = noticeActionsByStep.get(notice.step) ?? new Set<BootstrapActionName>();
    for (const action of notice.allowed_actions) {
      if (bannerClaims.has(`${notice.step}:${action}`)) {
        continue;
      }
      rendered.add(action);
    }
    noticeActionsByStep.set(notice.step, rendered);
  }

  // 步骤行要渲染的动作：步骤自身允许的动作减去已被显眼提示/notice 渲染的同名动作。
  function stepActions(
    step: LogicalCodebaseBootstrapProjection["steps"][number],
  ): BootstrapActionName[] {
    const rendered = noticeActionsByStep.get(step.step);
    return (step.allowed_actions ?? []).filter(
      (action) =>
        !rendered?.has(action) && !bannerClaims.has(`${step.step}:${action}`),
    );
  }

  // notice 要渲染的动作：去掉已被显眼提示占用的同名动作（不重复渲染按钮）。
  function noticeActions(
    notice: LogicalCodebaseBootstrapProjection["notices"][number],
  ): BootstrapActionName[] {
    return notice.allowed_actions.filter(
      (action) => !bannerClaims.has(`${notice.step}:${action}`),
    );
  }

  async function handleAction(
    step: string,
    action: BootstrapActionName,
    actionKey: string,
  ) {
    if (projectId === null) {
      return;
    }
    // 稳定 command id：同 key+action 重放幂等（服务端按 command key
    // 返回同一 durable 结果，不重复推进/不重复 provider turn）。notice 按钮
    // 用 notice.key，步骤行按钮用步骤的稳定 object_id。
    const commandId = `cmd-lc-bootstrap-${actionKey}-${action}`.replace(
      /[^a-zA-Z0-9_-]/g,
      "_",
    );
    setBusyAction(`${actionKey}:${action}`);
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

  // 一键推进：服务端按依赖顺序推进到 PlanningReady/失败/等待人工为止。
  // 幂等端点——重复点击不重复推进、不重复 provider turn。
  async function handleAdvance() {
    if (projectId === null) {
      return;
    }
    setAdvanceBusy(true);
    setActionError(null);
    try {
      const result = await postLogicalCodebaseBootstrapAdvance(
        projectId,
        current.logical_codebase_id,
      );
      const advanced = result?.advanced_steps ?? [];
      setAdvanceSummary(
        advanced.length > 0
          ? `已推进：${advanced.map((step) => STEP_LABELS[step] ?? step).join("、")}`
          : `没有可推进的步骤（${result?.stopped_reason ?? "已就绪或等待人工"}）`,
      );
      onChanged?.();
    } catch (error) {
      setActionError(error instanceof Error ? error.message : "推进失败");
    } finally {
      setAdvanceBusy(false);
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
      {ruleDrift ? (
        <div
          data-testid="lc-bootstrap-rule-drift"
          className="mt-1.5 rounded border-2 border-amber-500 bg-amber-100 px-2 py-1.5"
        >
          <p className="text-xs font-bold text-amber-900">
            规则已变更，需重新生成政策
          </p>
          <p className="mt-0.5 text-xs text-amber-900">
            当前已发布的聚合政策与磁盘上的规则不一致，provider
            启动会被拒绝；点「重新生成政策」按当前规则合成新版本（版本号 +1）。
          </p>
          <p
            data-testid="lc-bootstrap-rule-drift-detail"
            className="mt-0.5 text-xs text-slate-600"
          >
            {ruleDrift.detail}
          </p>
          <div className="mt-1.5 flex flex-wrap gap-2">
            <button
              type="button"
              disabled={busyAction !== null || advanceBusy}
              onClick={() =>
                void handleAction(ruleDrift.step, "republish", ruleDrift.actionKey)
              }
              className="min-h-9 rounded-md border border-amber-500 bg-white px-2.5 py-1 text-xs font-bold text-amber-800 transition-colors duration-200 hover:bg-amber-200 disabled:cursor-not-allowed disabled:opacity-60"
              data-testid="lc-bootstrap-rule-drift-republish"
            >
              {ACTION_BUTTON_LABELS.republish}
            </button>
          </div>
        </div>
      ) : null}
      <ul className="mt-1.5 space-y-0.5">
        {projection.steps.map((step) => (
          <li
            key={step.step}
            data-testid={`lc-bootstrap-step-${step.step}`}
            className="flex flex-wrap items-baseline gap-x-2 text-xs leading-snug text-[var(--aria-ink-muted)]"
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
            {stepActions(step).length > 0 ? (
              <div
                className="flex w-full flex-wrap gap-2 pt-1"
                data-testid={`lc-bootstrap-step-actions-${step.step}`}
              >
                {stepActions(step).map((action) => (
                  <button
                    key={action}
                    type="button"
                    disabled={busyAction !== null || advanceBusy}
                    onClick={() =>
                      void handleAction(step.step, action, step.object_id)
                    }
                    className="min-h-9 rounded-md border border-amber-300 bg-white px-2.5 py-1 text-xs font-semibold text-amber-700 transition-colors duration-200 hover:bg-amber-100 disabled:cursor-not-allowed disabled:opacity-60"
                    data-testid={`lc-bootstrap-step-action-${step.step}-${action}`}
                  >
                    {ACTION_BUTTON_LABELS[action] ?? action}
                  </button>
                ))}
              </div>
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
              {noticeActions(notice).length > 0 ? (
                <div className="mt-1.5 flex flex-wrap gap-2">
                  {noticeActions(notice).map((action) => (
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
      {!projection.planning_ready ? (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <button
            type="button"
            disabled={advanceBusy || busyAction !== null}
            onClick={() => void handleAdvance()}
            className="min-h-9 rounded-md border border-[var(--aria-primary)] bg-white px-2.5 py-1 text-xs font-semibold text-[var(--aria-primary)] transition-colors duration-200 hover:bg-amber-50 disabled:cursor-not-allowed disabled:opacity-60"
            data-testid="lc-bootstrap-advance"
          >
            {advanceBusy ? "推进中…" : "一键推进"}
          </button>
          {advanceSummary ? (
            <span
              data-testid="lc-bootstrap-advance-summary"
              className="text-xs text-[var(--aria-ink-muted)]"
            >
              {advanceSummary}
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
