import { useLayoutEffect, useRef } from "react";
import { WorkbenchOverlayDrawer } from "../components/shared/WorkbenchOverlayDrawer";
import { useWorkbenchOverlayDirectory } from "../state/workbench-overlay-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CodingPlanOverviewView,
  type PlanOverviewOpenTarget,
} from "./CodingPlanOverviewPage";

/**
 * UI-1（owner 拍板，2026-10-24 冻结）：Issue 工作台为背景的宽抽屉，承载
 * 「总览 ↔ 单个 Attempt 详情」切换。定位完全由 /workbench search 驱动
 * （overlay=plan-overview + planId/attemptId/workItemId/gateId/choiceId），
 * 本组件不自持路由状态——开/切/返回/关闭全部回调宿主经
 * mergeWorkbenchSearch 合并写入。
 *
 * 硬约束（禁止双挂载）：背景是 Issue 工作台而非 Coding 页；详情复用既有
 * CodingWorkspacePage，抽屉内任一时刻只挂载一个实例（useCodingWorkspaceWs
 * 在地址切换/卸载时断连并 reset 全局 store 的语义保持不变，不建第二套
 * runtime/store）。总览视图在详情态保持挂载但隐藏且暂停轮询（保留数据与
 * 滚动位置）；抽屉关闭即整体卸载，5s 轮询与待办徽标聚合随之中停止。
 */
export function CodingPlanOverviewDrawer({
  planId,
  attemptId,
  workItemId,
  gateId,
  choiceId,
  onOpenAttempt,
  onBackToOverview,
  onClose,
}: {
  planId: string;
  attemptId?: string;
  /** 详情定位字段（search 驱动；workItemId 为 UI-3/深链保留位，详情页不消费）。 */
  workItemId?: string;
  gateId?: string;
  choiceId?: string;
  onOpenAttempt: (target: PlanOverviewOpenTarget) => void;
  onBackToOverview: () => void;
  onClose: () => void;
}) {
  const address = useWorkbenchOverlayDirectory(
    (state) => state.planAddresses[planId] ?? null,
  );
  const overviewScrollRef = useRef<HTMLDivElement | null>(null);
  const savedScrollTopRef = useRef(0);
  const previousAttemptIdRef = useRef<string | null>(null);

  // 总览 ↔ 详情切换保留总览滚动位置：总览保持挂载（仅隐藏，数据不重建）。
  // scrollTop 在可见期随滚动持续保存（display:none 后浏览器读不回真实值），
  // 返回总览（同一 commit 内容器恢复可见）时恢复。
  const handleOverviewScroll = () => {
    if (!attemptId) {
      savedScrollTopRef.current = overviewScrollRef.current?.scrollTop ?? 0;
    }
  };
  useLayoutEffect(() => {
    const previous = previousAttemptIdRef.current;
    previousAttemptIdRef.current = attemptId ?? null;
    if (previous && !attemptId && overviewScrollRef.current) {
      overviewScrollRef.current.scrollTop = savedScrollTopRef.current;
    }
  }, [attemptId]);

  return (
    <WorkbenchOverlayDrawer
      label="Coding Plan 总览"
      title="Coding Plan 总览"
      description={attemptId ? `${planId} · ${attemptId}` : planId}
      testId="plan-overview-drawer"
      onClose={onClose}
    >
      {address ? (
        <>
          {attemptId ? (
            <div
              data-testid="plan-overview-drawer-detail"
              className="flex min-h-0 flex-1 flex-col"
            >
              {/* 既有 Attempt 详情单实例：onBack/onOpenPlanOverview 都回总览
                  （GroupProgress 的 planId 即本抽屉 planId，同名互证）。 */}
              <CodingWorkspacePage
                address={{
                  projectId: address.projectId,
                  issueId: address.issueId,
                  attemptId,
                }}
                focus={{ gateId, choiceId }}
                onBack={onBackToOverview}
                onOpenPlanOverview={() => onBackToOverview()}
              />
            </div>
          ) : null}
          <div
            ref={overviewScrollRef}
            data-testid="plan-overview-drawer-overview"
            onScroll={handleOverviewScroll}
            className={
              attemptId
                ? "hidden min-h-0 flex-1"
                : "flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain"
            }
          >
            <CodingPlanOverviewView
              projectId={address.projectId}
              issueId={address.issueId}
              planId={planId}
              onOpenAttempt={onOpenAttempt}
              pollIntervalMs={attemptId ? undefined : 5_000}
            />
          </div>
        </>
      ) : (
        <div
          data-testid="plan-overview-drawer-unresolved"
          className="px-4 py-3 text-xs text-[var(--aria-ink-muted)]"
        >
          正在定位 plan 归属（planId={planId}）…若长时间无响应，请在 Issue
          工作台切换到该 plan 所属 Project 后重试。
        </div>
      )}
    </WorkbenchOverlayDrawer>
  );
}
