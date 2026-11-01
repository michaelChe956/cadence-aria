import type {
  CodingAttempt,
  IssueDeliverySummaryDto,
  LifecycleWorkItem,
} from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { LifecycleCardDrawer } from "./LifecycleCardDrawer";
import {
  lifecycleCardKey,
  toDrawerEntity,
} from "./IssueLifecycleWorkbenchParts";

type IssueLifecycleWorkbenchDrawerProps = {
  focusedEntity: LifecycleCardData;
  workItems: LifecycleWorkItem[];
  codingAttempts: CodingAttempt[];
  deliverySummary: IssueDeliverySummaryDto | undefined;
  onClose: () => void;
  onOpenWorkspace: () => void;
  onOpenCodingWorkspace: () => void;
  onGenerateNext: () => void;
  onDelete: () => void;
  onDeleteAllCodingWorkspaces: () => void;
  groupCodingDeleteBusy: boolean;
};

/** 生命周期抽屉动作按实体归属启用；v2 单项只读，操作集中在所属计划组。 */
export function IssueLifecycleWorkbenchDrawer({
  focusedEntity,
  workItems,
  codingAttempts,
  deliverySummary,
  onClose,
  onOpenWorkspace,
  onOpenCodingWorkspace,
  onGenerateNext,
  onDelete,
  onDeleteAllCodingWorkspaces,
  groupCodingDeleteBusy,
}: IssueLifecycleWorkbenchDrawerProps) {
  const isPlanWorkItem = focusedEntity.kind === "work_item" &&
    Boolean(focusedEntity.raw.source_work_item_plan_id);
  return (
    <div className="fixed right-0 top-0 z-50 h-full w-[min(480px,100vw)] shadow-xl">
      <LifecycleCardDrawer
        key={lifecycleCardKey(focusedEntity)}
        entity={toDrawerEntity(focusedEntity, workItems, codingAttempts)}
        deliverySummary={deliverySummary}
        onClose={onClose}
        onOpenWorkspace={isPlanWorkItem ? undefined : onOpenWorkspace}
        onOpenCodingWorkspace={
          !isPlanWorkItem &&
          ((focusedEntity.kind === "work_item" &&
            focusedEntity.raw.plan_status === "confirmed") ||
          (focusedEntity.kind === "work_item_group" &&
            focusedEntity.raw.status === "confirmed"))
            ? onOpenCodingWorkspace
            : undefined
        }
        onDeleteAllCodingWorkspaces={
          // H5（设计 C）：辅入口不要求 confirmed 状态——残留清理对失效 plan
          // 同样可达（无法进入 attempt 页的用户仍能清剩余）。
          focusedEntity.kind === "work_item_group"
            ? onDeleteAllCodingWorkspaces
            : undefined
        }
        groupCodingDeleteBusy={groupCodingDeleteBusy}
        onGenerateNext={
          focusedEntity.status === "confirmed" &&
          (focusedEntity.kind === "story_spec" ||
            focusedEntity.kind === "design_spec")
            ? onGenerateNext
            : undefined
        }
        onDelete={
          !isPlanWorkItem &&
          (focusedEntity.kind === "work_item" ||
          focusedEntity.kind === "work_item_group")
            ? onDelete
            : undefined
        }
      />
    </div>
  );
}
