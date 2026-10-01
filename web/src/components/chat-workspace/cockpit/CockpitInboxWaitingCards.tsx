import { useState } from "react";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
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
  retry_review: "重试代码审查",
  retry_internal_review: "重试内部评审",
  retry_group_review_shard: "重试组评审分片",
  retry_group_reduction: "重试组归并",
  send_to_coder: "发回 Coder",
  accept_risk: "接受风险继续",
  abort: "中止",
};

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
