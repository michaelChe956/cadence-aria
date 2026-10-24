import { CodingPlanOverviewDrawer } from "../../pages/CodingPlanOverviewDrawer";
import {
  closeWorkbenchOverlayPatch,
  type WorkbenchSearch,
} from "../../workbench-search";
import { WorkbenchInboxOverlay } from "./WorkbenchInboxOverlay";

/**
 * UI-1：/workbench 的 overlay 抽屉宿主——按冻结 search 契约
 * （workbench-search.ts）渲染抽屉层。定位写入全部经 onSearchChange 回调
 * （宿主 router 以 mergeWorkbenchSearch 合并写入，本组件不直接碰路由）。
 *
 * UI-3 接入：overlay=inbox 分支已注册，内容挂载点见 WorkbenchInboxOverlay。
 */
export function WorkbenchOverlayHost({
  search,
  onSearchChange,
}: {
  search: WorkbenchSearch;
  onSearchChange: (patch: Partial<WorkbenchSearch>) => void;
}) {
  if (search.overlay === "plan-overview" && search.planId) {
    return (
      <CodingPlanOverviewDrawer
        planId={search.planId}
        attemptId={search.attemptId}
        workItemId={search.workItemId}
        gateId={search.gateId}
        choiceId={search.choiceId}
        onOpenAttempt={(target) =>
          onSearchChange({
            attemptId: target.attemptId,
            workItemId: target.workItemId,
            gateId: target.gateId,
            choiceId: target.choiceId,
          })
        }
        onBackToOverview={() =>
          onSearchChange({
            attemptId: undefined,
            workItemId: undefined,
            gateId: undefined,
            choiceId: undefined,
          })
        }
        onClose={() => onSearchChange(closeWorkbenchOverlayPatch())}
      />
    );
  }
  if (search.overlay === "inbox") {
    return (
      <WorkbenchInboxOverlay
        onClose={() => onSearchChange(closeWorkbenchOverlayPatch())}
      />
    );
  }
  return null;
}
