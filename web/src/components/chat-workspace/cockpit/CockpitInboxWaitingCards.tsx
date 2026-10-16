import { X } from "lucide-react";
import { useState } from "react";
import {
  ApiRequestError,
  AUTOMATION_PLAN_TERMINATION_REQUIRED,
  getAutomationEnrollment,
  parseAutomationTerminationIdentity,
  postAutomationEnrollmentTerminate,
  postCodingGateResponse,
} from "../../../api/client";
 import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
 import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
import { notifyLifecycleInvalidated } from "../../../state/lifecycle-workbench-store";
import { useCodingWorkspaceStore } from "../../../state/coding-workspace-store";
import { ConfirmTwiceButton } from "./ConfirmTwiceButton";
import { BTN_SECONDARY_CLASS } from "../gate-visual-tokens";

// 从 CockpitInbox.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// durable 等待项卡片（C1/C2 恢复、LC 冷启动、repository 初始化恢复）与动作文案常量。
/**
 * C1 Task 9：C1 恢复动作按钮文案（按 actions 名固定；点击只经 facade
 * 触发对应 REST，前端不判定业务成功）。
 */
const C1_ACTION_BUTTON_LABELS: Record<string, string> = {
  recover_candidate: "恢复候选门",
  retry_initialization: "重试初始化",
  confirm_takeover: "确认接管",
  rebind: "去换代",
  // C2 Task 12：coding 链等待项动作（restart／gate 动作经 REST 作答）。
  restart_coding: "重启 Coding",
  // C5 Task 6：project 级 repository 初始化失败“网关恢复后继续”。
  resume_repository_initialization: "网关恢复后继续",
  manual_continue: "人工继续",
  retry_coding: "重试编码",
  // P2-d（REQ-CRO-09）：分诊卡「说明情况重试」动作，避免驾驶舱显示 raw id。
  retry_coding_with_context: "我来说明情况，让 AI 继续",
  retry_review: "重试代码审查",
  retry_internal_review: "重试内部评审",
  retry_group_review_shard: "重试组评审分片",
  retry_group_reduction: "重试组归并",
  send_to_coder: "发回 Coder",
  accept_risk: "接受风险继续",
  abort: "中止",
};

// ── 批 2 T4（REQ-WIGA-09/08）：统一「终止整个自动化」——页面/收件箱共用
// 同一二次确认面（块 3 §2/§4）：任意入口→同一确认→confirmed:true 提交
// terminate；未确认零请求副作用；REST/WS 428 automation_plan_termination_
// required 只弹本确认面，不渲染为错误弹窗。──

export const PLAN_TERMINATION_ENTRY_LABEL = "终止整个自动化";
export const PLAN_TERMINATION_CONFIRM_LABEL = "确认终止整个自动化";

const PLAN_TERMINATION_EXPLANATION =
  "本计划全部仓库、全部 Work Item 将停止，不可继续；已产出文件保留。";

/** 统一终止入口按钮（危险描边；点击只开确认面，零请求）。 */
export function AutomationTerminationEntryButton({
  onOpen,
}: {
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      data-testid="automation-termination-entry"
      onClick={onOpen}
      className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--aria-danger)] bg-white px-2 text-xs font-semibold text-[var(--aria-danger)] transition-colors hover:bg-red-50 disabled:opacity-50"
    >
      <X className="h-3.5 w-3.5" aria-hidden="true" />
      {PLAN_TERMINATION_ENTRY_LABEL}
    </button>
  );
}

/** 终止态呈现（terminated_at＋不可逆说明）；视觉与 P5 托管卡同源
 *（琥珀指引底，`stopped-managed-hint` 同族）。 */
export function AutomationTerminatedBanner({
  terminatedAt,
  outcome = null,
}: {
  terminatedAt: string;
  outcome?: string | null;
}) {
  return (
    <div
      data-testid="automation-terminated-banner"
      role="status"
      className="flex shrink-0 flex-wrap items-center gap-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-4 text-amber-900"
    >
      <span className="font-semibold">自动化已终止</span>
      <span className="font-mono">{terminatedAt}</span>
      <span>本计划全部仓库已停止，不可继续；已产出文件保留。</span>
      {outcome === "cleanup_required" ? (
        // k3 波5 F2（P3）：如实说明需管理员介入——前端无重试入口，且
        // terminate 每次重读 expected_policy_revision（终止后已 +1），真
        // 重试会被命令账本判异 payload Conflict，不承诺可重试。
        <span className="font-semibold">
          清理未完成（cleanup_required），需管理员介入。
        </span>
      ) : null}
    </div>
  );
}

/**
 * 统一二次确认面：Coding Workspace 页、总览驾驶舱与收件箱共用。确认前零
 * 请求；确认（两步）后按需 GET enrollment 补读身份 → POST terminate
 * `confirmed:true`（command_id 按 enrollment 派生，重放幂等）。服务端若再
 * 回 428（版本漂移），用 details 身份刷新确认面而不是报错。
 */
export function AutomationTerminationConfirmSurface() {
  const termination = useCodingWorkspaceStore((state) => state.planTermination);
  const openConfirm = useCodingWorkspaceStore((state) => state.openTerminationConfirm);
  const closeConfirm = useCodingWorkspaceStore((state) => state.closeTerminationConfirm);
  const setSubmitting = useCodingWorkspaceStore((state) => state.setTerminationSubmitting);
  const setError = useCodingWorkspaceStore((state) => state.setTerminationError);
  const setResult = useCodingWorkspaceStore((state) => state.setTerminationResult);
  const [identityNotice, setIdentityNotice] = useState<string | null>(null);
  const request = termination.pending;

  if (request === null) {
    return null;
  }

  const submit = async () => {
    setSubmitting(true);
    setIdentityNotice(null);
    try {
      let enrollmentId = request.enrollmentId;
      let policyRevision = request.policyRevision;
      if (enrollmentId === null || policyRevision === null) {
        // 身份未知（如总览目录解析路径）：提交前只读补读，不猜。
        const enrollment = await getAutomationEnrollment(
          request.projectId,
          request.issueId,
        );
        if (enrollment === null) {
          throw new ApiRequestError({
            code: "automation_enrollment_not_found",
            message: "未找到自动化 enrollment，无法终止",
            details: {},
          });
        }
        enrollmentId = enrollment.enrollment_id;
        policyRevision = enrollment.policy_revision;
      }
      const response = await postAutomationEnrollmentTerminate(
        request.projectId,
        request.issueId,
        {
          command_id: `cmd-fe-terminate-${enrollmentId}`,
          expected_enrollment_id: enrollmentId,
          expected_policy_revision: policyRevision,
          confirmed: true,
        },
      );
      setIdentityNotice(
        `已终止 enrollment ${response.enrollment_id}${response.plan_id ? ` · plan ${response.plan_id}` : ""}`,
      );
      setResult({
        projectId: request.projectId,
        issueId: request.issueId,
        terminatedAt: response.terminated_at,
        outcome: response.outcome,
      });
    } catch (error) {
      if (
        error instanceof ApiRequestError &&
        error.code === AUTOMATION_PLAN_TERMINATION_REQUIRED
      ) {
        const identity = parseAutomationTerminationIdentity(error.details);
        if (identity !== null) {
          // 确认后仍要求确认（身份漂移）：以服务端身份刷新确认面重试。
          openConfirm({
            projectId: request.projectId,
            issueId: request.issueId,
            enrollmentId: identity.enrollment_id,
            policyRevision: identity.policy_revision,
          });
          return;
        }
      }
      setError(error instanceof Error && error.message !== "" ? error.message : "终止请求失败");
    }
  };

  return (
    <div
      data-testid="automation-termination-confirm"
      role="alertdialog"
      aria-label="终止整个自动化确认"
      className="flex shrink-0 flex-wrap items-center gap-2 border-b border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-4 text-amber-900"
    >
      <span className="font-semibold">{PLAN_TERMINATION_ENTRY_LABEL}</span>
      <span>{PLAN_TERMINATION_EXPLANATION}</span>
      {identityNotice ? (
        <span role="status" className="font-semibold">
          {identityNotice}
        </span>
      ) : null}
      {termination.submitting ? (
        <span role="status" className="font-semibold">
          终止中…
        </span>
      ) : (
        <>
          <ConfirmTwiceButton
            variant="ghost"
            label={PLAN_TERMINATION_ENTRY_LABEL}
            confirmLabel={PLAN_TERMINATION_CONFIRM_LABEL}
            onConfirm={() => void submit()}
          />
          <button
            type="button"
            onClick={closeConfirm}
            className="inline-flex h-8 items-center rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-900 hover:bg-amber-100"
          >
            取消
          </button>
        </>
      )}
      {termination.error ? (
        <span role="alert" className="font-semibold text-[var(--aria-danger)]">
          {termination.error}
        </span>
      ) : null}
    </div>
  );
}

/**
 * 收件箱 Abort 统一路由（T4）：先走原 gate-responses REST（手动单 attempt
 * 原语义）；托管 plan 时服务端 T3 返回 428 `automation_plan_termination_
 * required`（零副作用、details 携带身份）——前端只弹统一二次确认面。
 */
async function requestInboxGateAbortOrTerminate(payload: {
  projectId: string;
  issueId: string;
  attemptId: string;
  gateId: string;
  commandId: string;
  expectedVersion: number;
}) {
  try {
    await postCodingGateResponse(
      {
        projectId: payload.projectId,
        issueId: payload.issueId,
        attemptId: payload.attemptId,
      },
      {
        command_id: payload.commandId,
        gate_id: payload.gateId,
        action_id: "abort",
        expected_version: payload.expectedVersion,
      },
    );
    notifyLifecycleInvalidated(payload.issueId);
  } catch (error) {
    if (
      error instanceof ApiRequestError &&
      error.code === AUTOMATION_PLAN_TERMINATION_REQUIRED
    ) {
      const identity = parseAutomationTerminationIdentity(error.details);
      useCodingWorkspaceStore.getState().openTerminationConfirm({
        projectId: payload.projectId,
        issueId: payload.issueId,
        enrollmentId: identity?.enrollment_id ?? null,
        policyRevision: identity?.policy_revision ?? null,
      });
      return;
    }
    // 非 428 失败如实记录（等待项保留下轮刷新，不吞成成功）。
    console.error("[c1] abort action failed", payload, error);
  }
}

/**
 * C4 Task 9：LC 冷启动统一动作按钮文案（准备/继续/重试/核验；repair 的
 * mapping 裁决表单在生命周期工作台，驾驶舱只导航不复制表单）。
 */
const LC_BOOTSTRAP_ACTION_BUTTON_LABELS: Record<string, string> = {
  prepare: "准备",
  continue: "继续",
  retry: "重试",
  revalidate: "核验",
  repair: "去修复",
};

/**
 * C1 Task 9（enrollment-recovery-surface）：durable 恢复等待项卡片——展示
 * reason/已完成步骤/target/身份/可能副作用/下一阶段，并按服务端 durable
 * 派生的 actions 渲染动作按钮（稳定 command_id 从 item id 派生，同命令重试
 * 幂等）。按钮只经 facade 透传页面接线的 C1 REST 发送器；无 facade 时只读。
 */
export function C1RecoveryCard({
  item,
  actions,
}: {
  item: CockpitInboxItem;
  actions?: CockpitActionFacade;
}) {
  const info = item.c1Info;
  if (!info) {
    return null;
  }
  const handleAction = (action: string) => {
    if (!actions) {
      return;
    }
    if (action === "abort") {
      // 批 2 T4（REQ-WIGA-09）：收件箱 Abort 统一路由——手动走原
      // gate-responses REST；托管 plan 服务端回 428 统一终止要求（零副作用），
      // 前端只弹与页面共用的二次确认面（不渲染为错误）。
      const context = info.actionContext.find((entry) => entry.action === action);
      if (!context || !info.attemptId || !info.gateId) {
        return;
      }
      void requestInboxGateAbortOrTerminate({
        projectId: info.projectId,
        issueId: info.issueId ?? "",
        attemptId: info.attemptId,
        gateId: info.gateId,
        commandId: context.commandId,
        expectedVersion: context.expectedVersion,
      });
      return;
    }
    if (action === "recover_candidate") {
      actions.recoverCandidate({
        kind: "recover_candidate",
        projectId: info.projectId,
        issueId: info.issueId ?? "",
        sessionId: info.sessionId ?? "",
        gateId: info.gateId ?? "",
        commandId: `cmd-c1-recover-${info.itemId}`,
      });
      return;
    }
    if (action === "retry_initialization") {
      void actions.retryInitialization({
        kind: "retry_initialization",
        projectId: info.projectId,
        issueId: info.issueId ?? "",
        planId: info.planId ?? "",
        commandId: `cmd-c1-retry-${info.itemId}`,
        attemptId: info.attemptId ?? "",
        // next_phase 携带服务端 durable journal checkpoint；未知副作用
        // 首试不确认（Task 7 NeedsHuman 语义），确认按钮单独发送。
        checkpoint: info.nextPhase ?? "record_persisted",
        confirmUnknownSideEffect: false,
      });
      return;
    }
    if (action === "confirm_takeover") {
      void actions.confirmTakeover({
        kind: "confirm_takeover",
        projectId: info.projectId,
        issueId: info.issueId ?? "",
        commandId: `cmd-c1-takeover-${info.itemId}`,
        // lease id 是服务端 item id 的尾段（c1:lease_takeover:{issue}:{lease}）。
        leaseId: info.itemId.split(":").pop() ?? "",
        attemptId: info.attemptId ?? "",
      });
      return;
    }
    if (action === "restart_coding") {
      // C2 Task 12：action_context 携带服务端派生的稳定 command_id＋
      // expected 版本；前端不自行生成、不判定成功。
      const context = info.actionContext.find((entry) => entry.action === action);
      if (!context || !info.attemptId) {
        return;
      }
      void actions.restartCoding?.({
        kind: "restart_coding",
        projectId: info.projectId,
        issueId: info.issueId ?? "",
        attemptId: info.attemptId,
        commandId: context.commandId,
        expectedVersion: context.expectedVersion,
      });
      return;
    }
    if (info.gateId && info.actionContext.some((entry) => entry.action === action)) {
      // C2 Task 12：gate 动作经 gate-responses REST 作答（与 coding WS
      // 同一应用服务）；仅 action_context 内携带版本的动作可出站。
      const context = info.actionContext.find((entry) => entry.action === action);
      if (!context || !info.attemptId) {
        return;
      }
      void actions.respondGate?.({
        kind: "gate_response",
        projectId: info.projectId,
        issueId: info.issueId ?? "",
        attemptId: info.attemptId,
        gateId: info.gateId,
        actionId: action,
        commandId: context.commandId,
        expectedVersion: context.expectedVersion,
      });
      return;
    }
    if (action === "resume_repository_initialization") {
      // C5 Task 6：resume 按 operation_id 出站，command id 稳定派生
      //（同命令重放幂等）；不要求 issueId（project 等待项无 issue 归属）。
      if (!info.operationId) {
        return;
      }
      void actions.resumeRepositoryInitialization?.({
        kind: "resume_repository_initialization",
        projectId: info.projectId,
        operationId: info.operationId,
        commandId: `cmd-repo-init-resume-${info.operationId}`,
      });
      return;
    }
    if (action === "rebind") {
      actions.rebind({
        kind: "rebind",
        projectId: info.projectId,
        issueId: info.issueId ?? "",
      });
    }
  };
  return (
    <div className="mt-2" data-testid={`c1-waiting-${info.kind}`}>
      {info.completedSteps.length > 0 ? (
        <p className="text-xs text-slate-600">
          已完成步骤：{info.completedSteps.join("、")}
        </p>
      ) : null}
      <p className="mt-1 text-xs text-slate-500">target：{info.targetLabel}</p>
      {info.possibleSideEffect ? (
        <p className="mt-1 text-xs text-[var(--aria-danger)]">
          可能副作用：{info.possibleSideEffect}
        </p>
      ) : null}
      {info.nextPhase ? (
        <p className="mt-1 text-xs text-slate-500">下一阶段：{info.nextPhase}</p>
      ) : null}
      {info.diagnostics ? (
        // C5 Task 6：repository 初始化失败的结构化诊断（原因/步骤/路径）。
        <div className="mt-1" data-testid="repo-init-failure-diagnostics">
          <p className="aria-mono text-xs text-[var(--aria-danger)]">
            失败步骤 {info.diagnostics.failedStep} · 原因{" "}
            {info.diagnostics.reasonCode}
            {info.diagnostics.provider ? ` · provider ${info.diagnostics.provider}` : ""}
            {info.diagnostics.retryable ? " · 可重试" : ""}
          </p>
          {info.diagnostics.stderrSummary ? (
            <p className="mt-1 break-words text-xs text-slate-500">
              {info.diagnostics.stderrSummary}
            </p>
          ) : null}
          {info.diagnostics.changedPaths.length > 0 ? (
            <p className="mt-1 break-words text-xs text-slate-500">
              已改动路径：{info.diagnostics.changedPaths.join("、")}
            </p>
          ) : null}
        </div>
      ) : null}
      {info.actions.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-2">
          {info.actions.map((action) => (
            <button
              key={action}
              type="button"
              disabled={!actions}
              onClick={() => handleAction(action)}
              className={`${BTN_SECONDARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
              data-testid={
                action === "resume_repository_initialization"
                  ? "repo-init-resume-action"
                  : `c1-action-${action}`
              }
            >
              {C1_ACTION_BUTTON_LABELS[action] ?? action}
            </button>
          ))}
          {info.actions.includes("retry_initialization") && info.possibleSideEffect ? (
            <button
              type="button"
              disabled={!actions}
              onClick={() =>
                actions?.retryInitialization({
                  kind: "retry_initialization",
                  projectId: info.projectId,
                  issueId: info.issueId ?? "",
                  planId: info.planId ?? "",
                  // 未知副作用确认是独立命令（同 command 异 payload fail-closed）。
                  commandId: `cmd-c1-retry-confirm-${info.itemId}`,
                  attemptId: info.attemptId ?? "",
                  checkpoint: info.nextPhase ?? "record_persisted",
                  confirmUnknownSideEffect: true,
                })
              }
              className={`${BTN_SECONDARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
              data-testid="c1-action-retry_initialization_confirm"
            >
              确认副作用并重试
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * C4 Task 9（LC 冷启动加固）：durable bootstrap 等待/失败通知卡片——展示
 * step/object/原因/可能外部副作用/下一步，并按服务端 notice 的
 * allowed_actions 渲染统一按钮。稳定 command_id 从 notice key 派生并复用
 * （同命令重放幂等，不重复计数）；按钮只经 facade 透传页面接线的
 * bootstrap action REST，前端不乐观改状态。repair 动作导航到生命周期
 * 工作台（mapping 裁决表单在那边）。
 */
export function LcBootstrapCard({
  item,
  actions,
}: {
  item: CockpitInboxItem;
  actions?: CockpitActionFacade;
}) {
  const info = item.bootstrapInfo;
  const [submitted, setSubmitted] = useState<string | null>(null);
  if (!info) {
    return null;
  }
  const handleAction = (action: string) => {
    if (!actions) {
      return;
    }
    // 同一 notice+action 的重复点击复用同一 command id（服务端 replay 幂等）。
    const commandId = `cmd-lc-bootstrap-${info.noticeKey}-${action}`
      .replace(/[^a-zA-Z0-9_-]/g, "_");
    setSubmitted(action);
    void actions
      .sendBootstrapAction({
        kind: "lc_bootstrap",
        projectId: info.projectId,
        logicalCodebaseId: info.logicalCodebaseId,
        step: info.step,
        action,
        commandId,
        expectedRevision: info.membershipRevision,
        expectedObjectId: info.objectId,
      })
      .catch(() => {
        // 动作失败保留 durable 等待项（下轮 bootstrap GET 刷新仍可见）；
        // 就地恢复按钮，不吞成成功。
        setSubmitted(null);
      });
  };
  return (
    <div className="mt-2" data-testid={`lc-bootstrap-${info.step}`}>
      <p className="text-xs text-slate-600">
        逻辑代码库 {info.logicalCodebaseId} · 步骤 {info.step} · 对象 {info.objectId}
      </p>
      <p className="aria-mono mt-1 text-xs text-[var(--aria-danger)]">{info.reasonCode}</p>
      {info.externalSideEffect && info.externalSideEffect !== "none" ? (
        <p className="mt-1 text-xs text-[var(--aria-danger)]">
          可能外部副作用：{info.externalSideEffect}
        </p>
      ) : null}
      {info.nextStep ? (
        <p className="mt-1 text-xs text-slate-500">下一步：{info.nextStep}</p>
      ) : null}
      {info.allowedActions.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-2">
          {info.allowedActions.map((action) => (
            <button
              key={action}
              type="button"
              disabled={!actions || submitted === action}
              onClick={() => handleAction(action)}
              className={`${BTN_SECONDARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
              data-testid={`lc-bootstrap-action-${action}`}
            >
              {LC_BOOTSTRAP_ACTION_BUTTON_LABELS[action] ?? action}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
