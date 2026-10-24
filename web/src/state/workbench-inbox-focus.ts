import { useMemo } from "react";
import { useRouter } from "@tanstack/react-router";
import { mergeWorkbenchSearch, type WorkbenchSearch } from "../workbench-search";
import type { CodingAttentionInboxProjection } from "./workspace-cockpit-projection";

/**
 * UI-3（待处理统一抽屉）：coding 待处理事项 → /workbench 待处理抽屉
 * （overlay=inbox）的聚焦定位。与 router.tsx 的 A1 统一导航（契约层，
 * UI-3 不改）互补：attempt 目标的通知/告警/收件箱「去处理」改开**同一**
 * 待处理抽屉并聚焦（设计 §3 能力 6「只提醒+定位，不执行动作」），本模块
 * 只 import 冻结契约 workbench-search.ts，不 import ./router（避免循环）。
 */
export type WorkbenchInboxFocusTarget = {
  planId?: string | null;
  attemptId?: string | null;
  workItemId?: string | null;
  gateId?: string | null;
  choiceId?: string | null;
};

/** attention 投影 → 抽屉聚焦字段；workspace_session 目标返回 null（走原会话导航）。 */
export function inboxFocusTargetFromAttention(
  info: CodingAttentionInboxProjection,
): WorkbenchInboxFocusTarget | null {
  if (info.target.type === "coding_attempt" && info.attemptId !== null) {
    return {
      planId: info.planId,
      attemptId: info.attemptId,
      workItemId: info.logicalWorkItemId,
      gateId: info.focus?.gate_id ?? null,
      choiceId: info.focus?.choice_id ?? null,
    };
  }
  // coding_plan 目标保持 UI-1 语义：开 plan-overview 抽屉（可操作面在那），
  // 不在本模块处理。
  return null;
}

/**
 * 打开共享待处理抽屉的导航实现：已在 /workbench 时合并现有 search
 * （保留 focus/其他 overlay 定位），跨路由进入时显式全新 search（防
 * from 路由同名字段串味）——与 router.tsx useCodingAttentionTargetOpener
 * 同一口径。无路由上下文（单组件测试）时返回 null，消费方回落旧导航。
 */
export function useWorkbenchInboxFocusOpener(): ((
  target: WorkbenchInboxFocusTarget,
) => void) | null {
  const router = useRouter({ warn: false });
  return useMemo(() => {
    if (!router) {
      return null;
    }
    return (target: WorkbenchInboxFocusTarget) => {
      const patch: Partial<WorkbenchSearch> = {
        overlay: "inbox",
        planId: target.planId ?? undefined,
        attemptId: target.attemptId ?? undefined,
        workItemId: target.workItemId ?? undefined,
        gateId: target.gateId ?? undefined,
        choiceId: target.choiceId ?? undefined,
      };
      const onWorkbench = router.state.location.pathname === "/workbench";
      void router.navigate(
        onWorkbench
          ? {
              to: "/workbench",
              search: (previous: Partial<WorkbenchSearch>) =>
                mergeWorkbenchSearch(previous, patch),
            }
          : { to: "/workbench", search: patch },
      );
    };
  }, [router]);
}
