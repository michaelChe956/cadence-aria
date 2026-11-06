import { useEffect, useState } from "react";
import {
  findWorkspaceSessionAutomationIdentity,
  findWorkspaceSessionIssueAddress,
  getAutomationEnrollmentControl,
  type AutomationControlStatus,
} from "../api/client";
import type { WorkspaceSessionStatus } from "../state/workspace-ws-store-types";

// 从 ChatCockpitPage.tsx 拆出（large_file_guard 行数上限，纯移动零行为变化）：
// plan 停点恢复面的只读数据链——会话所属 issue 目录解析 + P4 control GET
// 投影（stop_key/stage/allowed_actions/context_required）与复读节拍。
type PlanIssueAddress = { projectId: string; issueId: string };

type PlanContinueControlInput = {
  planContinueEligible: boolean;
  sessionId: string;
  activeNodeId: string | null;
  sessionStatus: WorkspaceSessionStatus | null;
  stage: string;
};

export function useCockpitPlanContinueControl({
  planContinueEligible,
  sessionId,
  activeNodeId,
  sessionStatus,
  stage,
}: PlanContinueControlInput) {
  const [planIssueAddress, setPlanIssueAddress] = useState<PlanIssueAddress | null>(null);
  const [planControl, setPlanControl] = useState<AutomationControlStatus | null>(null);
  const [planControlRefreshTick, setPlanControlRefreshTick] = useState(0);

  useEffect(() => {
    if (!planContinueEligible || planIssueAddress !== null) {
      return;
    }
    let alive = true;
    // 只读目录解析（projects → issues → lifecycle）；失败静默保持无恢复面。
    void findWorkspaceSessionIssueAddress(sessionId)
      .then((address) => {
        if (alive && address !== null) {
          setPlanIssueAddress(address);
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [planContinueEligible, planIssueAddress, sessionId]);

  useEffect(() => {
    if (!planContinueEligible || planIssueAddress === null) {
      if (!planContinueEligible) {
        setPlanControl(null);
      }
      return;
    }
    let alive = true;
    const { projectId, issueId } = planIssueAddress;
    void getAutomationEnrollmentControl(projectId, issueId)
      .then((status) => {
        if (alive) {
          setPlanControl(status);
        }
      })
      .catch(() => {
        if (alive) {
          setPlanControl(null);
        }
      });
    return () => {
      alive = false;
    };
    // 会话状态推进/继续动作后复读（tick 递增），停点漂移即时对账。
  }, [
    planContinueEligible,
    planIssueAddress,
    planControlRefreshTick,
    sessionId,
    activeNodeId,
    sessionStatus,
    stage,
  ]);

  return { planIssueAddress, planControl, planControlRefreshTick, setPlanControlRefreshTick };
}

type CockpitTerminationIdentity = {
  projectId: string;
  issueId: string;
  planId: string | null;
};

/**
 * 2026-11-06 陈旧回执修复（REQ-WIGA-14）：终止横幅的当前 plan 身份源。
 * 与 planControl（停点形态投影，终止 CAS 写 enabled=false 即被清空）不同，
 * 这里走 durable 会话记录（entity_id=plan_id，终止后仍在）⇒ 终止态/重挂载
 * 后仍可得，「本计划被终止 ⇒ 横幅仍渲染」与「身份不可得 ⇒ fail-closed」
 * 得以同时成立。仅在终止回执存在时解析一次（缓存），失败静默保持 null。
 * r4（复审 Important-1）：缓存以 sessionId 为键——同挂载切会话（路由无
 * key、参数变化不 remount）时旧会话身份立即失效，不得跨会话复用（否则
 * 旧代横幅可能错显到新会话/新代合法横幅被误藏）；sessionId 变化即触发
 * 重新解析（失败后的重试也随之自然发生）。
 */
export function useCockpitTerminationIdentity({
  receiptActive,
  sessionId,
}: {
  receiptActive: boolean;
  sessionId: string;
}): CockpitTerminationIdentity | null {
  const [cached, setCached] = useState<{
    sessionId: string;
    identity: CockpitTerminationIdentity;
  } | null>(null);
  const identity =
    cached !== null && cached.sessionId === sessionId ? cached.identity : null;

  useEffect(() => {
    if (!receiptActive || (cached !== null && cached.sessionId === sessionId)) {
      return;
    }
    let alive = true;
    void findWorkspaceSessionAutomationIdentity(sessionId)
      .then((resolved) => {
        if (alive && resolved !== null) {
          setCached({ sessionId, identity: resolved });
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [receiptActive, cached, sessionId]);

  return identity;
}
