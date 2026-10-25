import { DialogShell } from "../components/shared/DialogShell";
import {
  CodingPlanOverviewView,
  type PlanOverviewOpenTarget,
} from "./CodingPlanOverviewPage";

/**
 * F-B（owner 真机复验 2026-10-25）：「查看全部 WI」总览改为 Coding Attempt
 * 页内的弹窗——不跨页导航、不重连当前 attempt 的 WS、不建第二套 store。
 *
 * 复用 UI-1 已拆好的可嵌入视图 CodingPlanOverviewView（含聚合确认
 * PlanCodingFinalConfirm）与 DialogShell 行为内核（Esc / 焦点圈闭 / 滚动锁 /
 * 焦点归还），不另造浮层规范。
 *
 * 取数与轮询：弹窗按可见性取数——打开时 5 秒轮询（总览可见即可见更新），
 * 关闭即卸载 → 轮询句柄与在途请求随之停止。页面里的 GroupProgress 与
 * CodingDashboard 只做一次性取数（不轮询），因此不存在「两份 5 秒轮询」。
 *
 * 明细下钻不在弹窗内就地展开：展开会同时挂载第二个 attempt 的 WS/store
 * （全局 store 是单例，双挂载即互相覆写）。这里只回调 onOpenAttempt，由宿主
 * 导航到目标 attempt——同一时刻只有一个 attempt 实例。
 */
export type { PlanOverviewOpenTarget };

/** 总览可见时的轮询间隔（与抽屉宿主同口径）。 */
const PLAN_OVERVIEW_POLL_INTERVAL_MS = 5_000;

export function CodingPlanOverviewModal({
  projectId,
  issueId,
  planId,
  onOpenAttempt,
  onClose,
}: {
  projectId: string;
  issueId: string;
  planId: string;
  onOpenAttempt: (target: PlanOverviewOpenTarget) => void;
  onClose: () => void;
}) {
  return (
    <DialogShell
      label="Coding Plan 总览"
      title="Coding Plan 总览"
      description={planId}
      onClose={onClose}
      closeLabel="关闭全部 WI 总览"
      testId="plan-overview-modal"
      widthClass="max-w-4xl"
    >
      <CodingPlanOverviewView
        projectId={projectId}
        issueId={issueId}
        planId={planId}
        onOpenAttempt={onOpenAttempt}
        pollIntervalMs={PLAN_OVERVIEW_POLL_INTERVAL_MS}
      />
    </DialogShell>
  );
}
