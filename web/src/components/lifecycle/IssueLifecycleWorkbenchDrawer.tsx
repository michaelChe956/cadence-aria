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
};

/**
 * 生命周期详情抽屉（纯搬运自 IssueLifecycleWorkbench，无行为改动）：
 * 搬运原 drawer 块的包装层与各回调的条件启用规则（confirmed 状态、kind 白名单）。
 */
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
}: IssueLifecycleWorkbenchDrawerProps) {
  return (
    <div className="fixed right-0 top-0 z-50 h-full w-[min(480px,100vw)] shadow-xl">
      <LifecycleCardDrawer
        key={lifecycleCardKey(focusedEntity)}
        entity={toDrawerEntity(focusedEntity, workItems, codingAttempts)}
        deliverySummary={deliverySummary}
        onClose={onClose}
        onOpenWorkspace={onOpenWorkspace}
        onOpenCodingWorkspace={
          (focusedEntity.kind === "work_item" &&
            focusedEntity.raw.plan_status === "confirmed") ||
          (focusedEntity.kind === "work_item_group" &&
            focusedEntity.raw.status === "confirmed")
            ? onOpenCodingWorkspace
            : undefined
        }
        onGenerateNext={
          focusedEntity.status === "confirmed" &&
          (focusedEntity.kind === "story_spec" ||
            focusedEntity.kind === "design_spec")
            ? onGenerateNext
            : undefined
        }
        onDelete={
          focusedEntity.kind === "work_item" ||
          focusedEntity.kind === "work_item_group"
            ? onDelete
            : undefined
        }
      />
    </div>
  );
}
