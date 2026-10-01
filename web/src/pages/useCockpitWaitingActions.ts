import { useCallback } from "react";
import {
  confirmLeaseTakeover,
  getAutomationEnrollment,
  postWorkspaceHumanAction,
  retryAdvanceInitialization,
  // C2 Task 12：等待项动作的 coding REST（restart／gate-responses）。
  postCodingGateResponse,
  restartCodingAttempt,
} from "../api/client";
import { postLogicalCodebaseBootstrapAction } from "../api/logical-codebase-bootstrap";
import { postRepositoryInitializationResume } from "../api/repository-initialization";
import { notifyLifecycleInvalidated } from "../state/lifecycle-workbench-store";
import type {
  C1RecoveryActionPayload,
  LcBootstrapActionPayload,
} from "../state/cockpit-action-routing";

// 从 ChatCockpitPage.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// C1 恢复与 LC bootstrap 等待项的动作发送器（回调体原样搬移，依赖数组不变）。
export function useCockpitWaitingActions() {
  // C1 Task 9：驾驶舱 C1 恢复动作发送器——只触发对应 REST/application
  // service（recover_candidate→human-actions、retry_initialization→Task 7
  // retry 路由、confirm_takeover→Task 6 takeover 路由），expected binding
  // 从 durable enrollment 补读；rebind 走 Issue 生命周期工作台的显式表单
  //（此处只广播失效并留审计，不在驾驶舱复制换代表单）。
  const sendC1Action = useCallback(async (payload: C1RecoveryActionPayload) => {
    try {
      if (payload.kind === "recover_candidate") {
        await postWorkspaceHumanAction(payload.sessionId, {
          type: "candidate_recovery",
          command_id: payload.commandId,
          expected_gate_id: payload.gateId,
          action: "recover",
        });
      } else if (payload.kind === "retry_initialization") {
        const enrollment = await getAutomationEnrollment(payload.projectId, payload.issueId);
        const binding = enrollment?.binding_history?.current ?? null;
        if (!binding) {
          throw new Error("C1 retry requires a durable enrollment binding");
        }
        await retryAdvanceInitialization(payload.projectId, payload.issueId, payload.planId, {
          command_id: payload.commandId,
          expected_binding: binding,
          expected_attempt_id: payload.attemptId,
          expected_checkpoint: payload.checkpoint as Parameters<
            typeof retryAdvanceInitialization
          >[3]["expected_checkpoint"],
          confirm_unknown_side_effect: payload.confirmUnknownSideEffect,
        });
      } else if (payload.kind === "confirm_takeover") {
        const enrollment = await getAutomationEnrollment(payload.projectId, payload.issueId);
        const binding = enrollment?.binding_history?.current ?? null;
        if (!binding) {
          throw new Error("C1 takeover requires a durable enrollment binding");
        }
        await confirmLeaseTakeover(payload.projectId, payload.issueId, {
          command_id: payload.commandId,
          expected_binding: binding,
          expected_lease_id: payload.leaseId,
          expected_attempt_id: payload.attemptId,
        });
      } else if (payload.kind === "restart_coding") {
        // C2 Task 12：终态 attempt 显式 restart（Task 3 REST；同 command
        // 同 payload 幂等，旧版本 Rejected"请刷新"由服务端承载）。
        await restartCodingAttempt(
          {
            projectId: payload.projectId,
            issueId: payload.issueId,
            attemptId: payload.attemptId,
          },
          {
            command_id: payload.commandId,
            attempt_id: payload.attemptId,
            expected_attempt_version: payload.expectedVersion,
          },
        );
      } else if (payload.kind === "gate_response") {
        // C2 Task 12：gate response REST（与 coding WS 同一应用服务）——
        // 驾驶舱无 coding socket 也能作答。
        await postCodingGateResponse(
          {
            projectId: payload.projectId,
            issueId: payload.issueId,
            attemptId: payload.attemptId,
          },
          {
            command_id: payload.commandId,
            gate_id: payload.gateId,
            action_id: payload.actionId,
            expected_version: payload.expectedVersion,
          },
        );
      } else if (payload.kind === "resume_repository_initialization") {
        // C5 Task 6：project 级 repository 初始化失败“网关恢复后继续”——
        // 不要求 issueId，按 operation_id 调 Task 6 resume REST；成功后经
        // project invalidation 唤醒目录观察器重读 project 等待项。
        await postRepositoryInitializationResume(
          payload.projectId,
          payload.operationId,
          payload.commandId,
        );
      } else {
        // rebind：显式换代表单在 Issue 生命周期工作台（useIssueLifecycleGeneration）；
        // 驾驶舱只广播失效刷新，不复制换代表单。
        console.info(
          "[c1] explicit rebind lives in the issue lifecycle workbench",
          payload.projectId,
          payload.issueId,
        );
      }
      if (payload.kind === "resume_repository_initialization") {
        notifyLifecycleInvalidated(
          `repository_initialization:${payload.projectId}`,
        );
      } else {
        notifyLifecycleInvalidated(payload.issueId);
      }
    } catch (error) {
      // 动作失败保留 durable 等待项（下轮 lifecycle/project 刷新仍可见）；
      // 错误就地如实记录，不吞为成功、不乐观隐藏等待项。
      console.error("[c1] recovery action failed", payload, error);
    }
  }, []);
  // C4 Task 9/10：LC 冷启动统一动作发送器——驾驶舱卡片只透传 bootstrap
  // action REST（Task 6 动作面，command_id/expected 身份由卡片从 durable
  // notice 派生）；成功/重放后经 lifecycle invalidation 总线唤醒目录观察
  // （useWorkspaceSessionObservers 同源补读 bootstrap 纯投影）。失败如实
  // 记录，等待项保留下轮 GET 刷新；前端不乐观改状态。
  const sendLcBootstrapAction = useCallback(
    async (payload: LcBootstrapActionPayload) => {
      try {
        await postLogicalCodebaseBootstrapAction(
          payload.projectId,
          payload.logicalCodebaseId,
          {
            command_id: payload.commandId,
            step: payload.step as Parameters<
              typeof postLogicalCodebaseBootstrapAction
            >[2]["step"],
            action: payload.action as Parameters<
              typeof postLogicalCodebaseBootstrapAction
            >[2]["action"],
            expected_revision: payload.expectedRevision,
            expected_object_id: payload.expectedObjectId,
          },
        );
        notifyLifecycleInvalidated(
          `lc_bootstrap:${payload.projectId}:${payload.logicalCodebaseId}`,
        );
      } catch (error) {
        console.error("[lc-bootstrap] action failed", payload, error);
      }
    },
    [],
  );
  return { sendC1Action, sendLcBootstrapAction };
}
