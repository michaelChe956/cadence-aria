import { useEffect, useMemo, useState } from "react";
import type { Dispatch, RefObject, SetStateAction } from "react";
import { COCKPIT_INBOX_DRAWER_ID } from "../components/cockpit/CockpitInboxDrawer";
import type { WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import { useCockpitHotkeys } from "../hooks/useCockpitHotkeys";
import { createCockpitActionFacade } from "../state/cockpit-action-routing";
import type { ConfirmTwiceButtonHandle } from "../state/cockpit-operation-semantics";
import { useWorkspaceStore } from "../state/workspace-ws-store";

/**
 * cockpit 快捷键动作族（自 ChatCockpitPage 机械拆出，纯移动零行为变化）：
 * 收件箱抽屉聚焦挂起 state/effect + confirm/feedback/takeover/advance 四组
 * hotkeyHandlers（内嵌两处 createCockpitActionFacade 现读门面）+ useCockpitHotkeys
 * 接线，依赖数组原样迁移。
 */
export interface CockpitHotkeyActionsOptions {
  inboxDrawerOpen: boolean;
  setInboxDrawerOpen: Dispatch<SetStateAction<boolean>>;
  takeoverTargetSessionId: string | null;
  takeoverButtonRef: RefObject<ConfirmTwiceButtonHandle | null>;
  routeGateConfirm: (withReview?: boolean) => boolean;
  adoptLatestReview: () => void;
  confirmBatchGate: () => Promise<void>;
  workspaceWs: WorkspaceWsApi;
}

export function useCockpitHotkeyActions({
  inboxDrawerOpen,
  setInboxDrawerOpen,
  takeoverTargetSessionId,
  takeoverButtonRef,
  routeGateConfirm,
  adoptLatestReview,
  confirmBatchGate,
  workspaceWs,
}: CockpitHotkeyActionsOptions): void {
  // F-31：待处理收件箱已移入默认收起的抽屉（收起为常驻 DOM + visibility 过渡
  // 隐藏、子树不卸载），于是抽屉内的接管按钮/反馈框在收起态既不可见也不可聚焦。
  // 热键要先展开抽屉；对 visibility:hidden 子树 focus() 会静默失效，所以收起时把
  // 目标挂起，等抽屉真正提交/可见后再聚焦。
  const [pendingInboxFocus, setPendingInboxFocus] = useState<HTMLElement | null>(
    null,
  );
  useEffect(() => {
    if (!inboxDrawerOpen || pendingInboxFocus === null) {
      return;
    }
    pendingInboxFocus.focus();
    setPendingInboxFocus(null);
  }, [inboxDrawerOpen, pendingInboxFocus]);
  const hotkeyHandlers = useMemo(
    () => ({
      confirm: () => {
        const current = useWorkspaceStore.getState();
        createCockpitActionFacade({
          flowKind: current.flowKind,
          commandId:
            typeof current.humanGateTurn?.command_id === "string"
              ? current.humanGateTurn.command_id
              : null,
          getState: useWorkspaceStore.getState,
          sendConfirm: routeGateConfirm,
          sendAbandonGate: workspaceWs.sendAbandonGate,
          sendHumanGateFeedback: workspaceWs.sendHumanGateFeedback,
          sendAdvance: workspaceWs.sendAdvance,
          adoptReview: adoptLatestReview,
          sendBatchConfirm: confirmBatchGate,
          sendCompileRecovery: workspaceWs.sendWorkItemPlanCompileRecoveryAction,
        }).confirm();
      },
      feedback: () => {
        const editor = document.querySelector<HTMLElement>(
          '[data-testid="gate-feedback-editor"] [aria-label="门禁反馈"]',
        );
        if (editor === null) {
          return;
        }
        if (editor.closest(`#${COCKPIT_INBOX_DRAWER_ID}`) === null) {
          // 主区门卡的反馈框本就可见，照旧直接聚焦。
          editor.focus();
          return;
        }
        setPendingInboxFocus(editor);
        setInboxDrawerOpen(true);
      },
      takeover: () => {
        if (takeoverTargetSessionId === null) {
          return;
        }
        // 接管按钮随收件箱入抽屉：先展开抽屉再 arm()，否则「确认接管」态落在
        // 不可见子树里，用户还没看到确认态就要再按一次直接执行接管。
        setInboxDrawerOpen(true);
        takeoverButtonRef.current?.arm();
      },
      advance: () => {
        const current = useWorkspaceStore.getState();
        createCockpitActionFacade({
          flowKind: current.flowKind,
          commandId:
            typeof current.humanGateTurn?.command_id === "string"
              ? current.humanGateTurn.command_id
              : null,
          getState: useWorkspaceStore.getState,
          sendConfirm: routeGateConfirm,
          sendAbandonGate: workspaceWs.sendAbandonGate,
          sendHumanGateFeedback: workspaceWs.sendHumanGateFeedback,
          sendAdvance: workspaceWs.sendAdvance,
          adoptReview: adoptLatestReview,
          sendBatchConfirm: confirmBatchGate,
          sendCompileRecovery: workspaceWs.sendWorkItemPlanCompileRecoveryAction,
        }).advance();
      },
    }),
    [
      takeoverTargetSessionId,
      workspaceWs.sendAdvance,
      workspaceWs.sendAbandonGate,
      routeGateConfirm,
      adoptLatestReview,
      workspaceWs.sendHumanGateFeedback,
      confirmBatchGate,
      workspaceWs.sendWorkItemPlanCompileRecoveryAction,
    ],
  );
  useCockpitHotkeys(hotkeyHandlers);
}
