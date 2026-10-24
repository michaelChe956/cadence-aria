import { create } from "zustand";
import type { IssueLifecycleResponse } from "../api/types";

/**
 * UI-1/UI-3：/workbench 抽屉的 plan/attempt 地址簿。
 *
 * 冻结 search 契约（workbench-search.ts）不含 projectId/issueId，抽屉按
 * planId/attemptId 解析归一地址的数据源有二（合并，不互斥）：
 * 1. 入口种子（seedWorkbenchOverlayAddress）——开抽屉的调用方（Coding 页
 *    总览入口、A1 待办 plan 目标、旧 coding-plans 链接重定向）都持有完整
 *    地址，导航前先种入，保证开箱即解析；
 * 2. 工作台 lifecycle 目录（syncWorkbenchOverlayDirectory）——
 *    IssueLifecycleWorkbench 把已加载 issue 的 work_item_plans 与
 *    coding_attempts 全量归档，覆盖刷新/直开 URL（无种子）场景。
 *
 * 仅 selected project 的 lifecycle 在册：跨 project 直开 URL 且无种子时
 * 解析失败由抽屉就地提示（不新建第二套目录或全量扫描 API）。
 */
export type WorkbenchOverlayAddress = {
  projectId: string;
  issueId: string;
};

type WorkbenchOverlayDirectoryState = {
  planAddresses: Record<string, WorkbenchOverlayAddress>;
  attemptAddresses: Record<string, WorkbenchOverlayAddress>;
  seedWorkbenchOverlayAddress: (
    planId: string,
    address: WorkbenchOverlayAddress,
  ) => void;
  syncWorkbenchOverlayDirectory: (lifecycles: IssueLifecycleResponse[]) => void;
  resetWorkbenchOverlayDirectory: () => void;
};

export const useWorkbenchOverlayDirectory =
  create<WorkbenchOverlayDirectoryState>((set) => ({
    planAddresses: {},
    attemptAddresses: {},
    seedWorkbenchOverlayAddress: (planId, address) =>
      set((state) => ({
        planAddresses: { ...state.planAddresses, [planId]: address },
      })),
    syncWorkbenchOverlayDirectory: (lifecycles) =>
      set((state) => {
        const planAddresses = { ...state.planAddresses };
        const attemptAddresses = { ...state.attemptAddresses };
        for (const lifecycle of lifecycles) {
          for (const plan of lifecycle.work_item_plans ?? []) {
            planAddresses[plan.id] = {
              projectId: plan.project_id,
              issueId: plan.issue_id,
            };
          }
          for (const attempt of lifecycle.coding_attempts ?? []) {
            attemptAddresses[attempt.attempt_id] = {
              projectId: attempt.project_id,
              issueId: attempt.issue_id,
            };
          }
        }
        return { planAddresses, attemptAddresses };
      }),
    resetWorkbenchOverlayDirectory: () =>
      set({ planAddresses: {}, attemptAddresses: {} }),
  }));

/** 入口种子便捷函数（导航前调用；见文件头说明）。 */
export function seedWorkbenchOverlayAddress(
  planId: string,
  address: WorkbenchOverlayAddress,
): void {
  useWorkbenchOverlayDirectory
    .getState()
    .seedWorkbenchOverlayAddress(planId, address);
}
