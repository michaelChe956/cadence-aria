import { WorkbenchOverlayDrawer } from "../shared/WorkbenchOverlayDrawer";

/**
 * UI-3（待处理统一，owner 拍板）：overlay=inbox 待处理抽屉的**骨架**。
 * 导航/开合/关闭管线由 UI-1 的 overlay 契约统一承载（/workbench search：
 * overlay=inbox + planId/attemptId/workItemId/gateId/choiceId 定位字段，
 * 写入一律经 mergeWorkbenchSearch 合并）；内容由 UI-3 在
 * `workbench-inbox-overlay-mount` 挂载点接入（Cockpit 待处理投影 +
 * 分诊/choice 卡，复用既有协议与组件，不新建第二套待办模型）。
 */
export function WorkbenchInboxOverlay({ onClose }: { onClose: () => void }) {
  return (
    <WorkbenchOverlayDrawer
      label="待处理事项"
      title="待处理事项"
      testId="workbench-inbox-overlay"
      onClose={onClose}
    >
      {/* UI-3 内容挂载点：替换本占位为 CockpitInbox 投影/分诊卡宿主。 */}
      <div
        data-testid="workbench-inbox-overlay-mount"
        className="px-4 py-3 text-xs text-[var(--aria-ink-muted)]"
      >
        待处理抽屉（UI-3 接入）
      </div>
    </WorkbenchOverlayDrawer>
  );
}
