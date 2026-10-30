import { useCallback } from "react";
import { confirmWorkspaceSession, postWorkspaceHumanAction } from "../api/client";
import type { WorkspaceHumanAction } from "../api/types";
import type { WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import { notifyLifecycleInvalidated } from "../state/lifecycle-workbench-store";
import { useOperationAuditStore } from "../state/operation-audit-store";
import {
  GATE_EXCLUSION_ERROR_CODE,
  classifyGateExclusionError,
} from "../state/cockpit-action-routing";
import {
  gateActionBlockReason,
  gateKindOf,
  isStoryDesignAuthorConfirm,
  selectGateProjection,
} from "../state/workspace-cockpit-projection";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  postRevokeFindingExclusionsAction,
  type RevokeFindingExclusionsAction,
  type RevokeFindingExclusionsOutcome,
} from "../state/workspace-finding-exclusions";

/**
 * cockpit 门确认/修订反馈发送族（自 ChatCockpitPage 机械拆出，纯移动零行为变化）。
 * sendHttpConfirm/confirmHttpGate/confirmBatchGate/routeGateConfirm/
 * sendRevisionFeedback 五个回调与注释原样迁移；依赖仅 store 现读
 * （useWorkspaceStore.getState）、审计 store 与 workspaceWs 的
 * sendConfirmGate/sendRequestRevision 两方法及 sessionId（审计兜底）。
 */
export interface CockpitGateConfirmActions {
  confirmHttpGate: (withReview?: boolean) => boolean;
  confirmBatchGate: () => Promise<void>;
  routeGateConfirm: (withReview?: boolean) => boolean;
  sendRevisionFeedback: (feedback: string) => boolean;
  /**
   * P0 1.3（REQ-WIGA-05）Task 10：owner=server 会话的人工门命令 REST 发送器
   * （门面已按 automation.owner/门身份完成路由，这里只负责出站+审计+错误面）。
   */
  sendHumanActionRest: (action: WorkspaceHumanAction) => boolean;
  /**
   * REQ-WSC-13/14（k3 P0-2C）：撤销「排除此项」的 REST 发送器——sessionId
   * 与 sendHumanActionRest 同款现读（store 优先，参数兜底）。结果原样返回
   * 给门卡就地呈现（成功/失败都在卡上可见）：撤销失败必须让 owner 看到
   * 后端原因，不走全局协议错误面（那会丢掉「哪一条撤销失败」的上下文）。
   */
  sendRevokeFindingExclusions: (
    action: RevokeFindingExclusionsAction,
  ) => Promise<RevokeFindingExclusionsOutcome>;
}
/**
 * P2（REQ-WSC-15 归因扩展，REST 通道）：排除-only human action 的失败按
 * store「最近本地门动作」归因——复用 WS WsOutMessage::Error 的同一套判据
 * （classifyGateExclusionError 的 15s 窗口 + kind=exclusion，不放宽）与同一
 * 个 store 动作（applyGateExclusionFailure，消费归因线索、落同一份失败事实
 * /文案），不造第二套事实。归因不成立（窗口外/最近动作非排除）时零副作用，
 * 只走既有全局协议错误面。
 */
function attributeRestExclusionFailure(message: string): void {
  const attribution = classifyGateExclusionError(useWorkspaceStore.getState());
  if (attribution === null) {
    return;
  }
  useWorkspaceStore.getState().applyGateExclusionFailure({
    code: GATE_EXCLUSION_ERROR_CODE,
    message,
    gateKey: attribution.gateKey,
  });
}

export function useCockpitGateConfirm({
  sessionId,
  workspaceWs,
}: {
  sessionId: string;
  workspaceWs: WorkspaceWsApi;
}): CockpitGateConfirmActions {
  // F-31 纠偏：评审改为用户可选——author 门上提供「确认定稿」（缺省，不带
  // with_review）与「确认并评审」（with_review=true，服务端接管进入评审轮）两个
  // 动作；待处理抽屉经 facade confirmReview 同款对齐（v37 复验 #2），
  // 快捷键/批量 confirm 等其余入口维持定稿缺省。
  /**
   * F-20（wave2-f18-report §5）+ REQ-PCG-01（plan-compile-gate-visibility）：HTTP
   * confirm 通路。story/design AuthorConfirm 与 work_item_plan 整组 Draft
   * （batch_confirm）的 approve 设计通路同为 HTTP confirm 端点——WS confirm 帧在
   * 这些阶段被矩阵拒收。响应落定稿时乐观置 confirmed 收敛决策面；F-31 起服务端可能
   * 接管本轮进入 CrossReview（响应未定稿），此时不得乐观收敛，权威状态以服务端
   * session_state 广播为准。失败就地亮协议错误面（拒收不得零反馈）。
   */
  const sendHttpConfirm = useCallback(
    (options: { withReview: boolean; detail: string }): Promise<void> => {
      const current = useWorkspaceStore.getState();
      const targetSessionId = current.sessionId;
      if (targetSessionId === null) {
        return Promise.resolve();
      }
      const auditRecordId = useOperationAuditStore.getState().record({
        sessionId: targetSessionId,
        gateId: selectGateProjection(current)?.key ?? null,
        operation: "confirm",
        source: "chat",
        outcome: "sent",
        detail: options.detail,
      });
      return confirmWorkspaceSession(targetSessionId, "user", options.withReview)
        .then((session) => {
          useOperationAuditStore.getState().markCompleted(auditRecordId);
          if (session.status === "confirmed") {
            useWorkspaceStore.getState().setSessionStatus("confirmed");
          }
          // F-29：确认成功后通知 lifecycle invalidation——workbench 定向刷新该
          // issue 的 durable 投影（同页 notify + 跨 tab BroadcastChannel）。
          notifyLifecycleInvalidated(session.issue_id);
        })
        .catch((error: unknown) => {
          const code =
            typeof error === "object" && error !== null && "code" in error
              ? String(error.code)
              : "http_confirm_failed";
          const message =
            error instanceof Error && error.message !== ""
              ? error.message
              : "确认请求被服务端拒绝";
          useOperationAuditStore.getState().markRejected(auditRecordId, code);
          // k3 P3（F-31 纠偏复审）：拒收不得零反馈（F-28 同类）——legacy 会话
          // reviewer_enabled_at_start=None 时前端 reviewerEnabled 缺省 true，
          // 「确认并评审」会撞后端如实 4xx（workspace_session_review_not_enabled）。
          // 复用 F-28 hard-error-notice 面（ChatInputBar 在 author_confirm 渲染）
          // 就地亮出错误码+语义，决策面保持敞开供改点「确认定稿」。
          useWorkspaceStore.getState().setProtocolError({ code, message });
        });
    },
    [],
  );
  // 门面 confirm/confirmReview 的统一入口：HTTP 通路门（story/design author 门与
  // work_item_plan 整组 Draft 门）走 HTTP confirm；其余（SC typed/legacy
  // human_confirm）走既有 WS confirm 帧。withReview 仅对 author 门有意义（F-31
  // 抽屉「确认并评审」）——整组 Draft 确认不带评审参数。
  const confirmHttpGate = useCallback(
    (withReview = false): boolean => {
      const current = useWorkspaceStore.getState();
      const batch = gateKindOf(current) === "batch_confirm";
      if ((!batch && !isStoryDesignAuthorConfirm(current)) || current.sessionId === null) {
        return false;
      }
      void sendHttpConfirm({
        withReview: !batch && withReview,
        detail: batch ? "http-confirm-batch" : withReview ? "http-confirm-review" : "http-confirm",
      });
      return true;
    },
    [sendHttpConfirm],
  );
  /**
   * REQ-PCG-01：门面 confirmBatch 的发送入口——只有当前门确实是未阻断的
   * batch_confirm 时才出站（门面已做同类校验；这里再读一次 store 防陈旧闭包）。
   */
  const confirmBatchGate = useCallback((): Promise<void> => {
    const current = useWorkspaceStore.getState();
    if (
      gateKindOf(current) !== "batch_confirm" ||
      gateActionBlockReason(current) !== null ||
      current.sessionId === null
    ) {
      return Promise.resolve();
    }
    return sendHttpConfirm({ withReview: false, detail: "http-confirm-batch" });
  }, [sendHttpConfirm]);
  // 门面 confirm 统一入口（门面 confirm/confirmReview、快捷键、批量确认共用本入口）：
  // story/design author 门与 work_item_plan 整组 Draft 门走 HTTP，其余（SC typed/
  // legacy human_confirm）走既有 WS confirm 帧；withReview 仅对 author 门有意义
  // （F-31 抽屉「确认并评审」），WS 通路忽略该参。
  const routeGateConfirm = useCallback((withReview = false): boolean => {
    const current = useWorkspaceStore.getState();
    const kind = gateKindOf(current);
    if (kind === "batch_confirm") {
      return confirmHttpGate(withReview);
    }
    // REQ-PCG-02：compile recovery 门没有 confirm 语义——WS confirm 帧在 SC
    // HumanConfirm 会命中 Approval 臂并开启第二个 compile
    // （compile.rs enter_policy_valid_work_item_plan_compile），故确认入口对整个
    // recovery 门 fail-closed（recovery 自身动作走 recoverCompile 的独立通道）。
    if (kind === "compile_recovery") {
      return false;
    }
    if (isStoryDesignAuthorConfirm(current)) {
      return confirmHttpGate(withReview);
    }
    return workspaceWs.sendConfirmGate();
  }, [confirmHttpGate, workspaceWs.sendConfirmGate]);

  // v38 复验 #2/#3（恢复 C3 前原意）：story/design AuthorConfirm 门的反馈修订
  // 发送通道——「采纳 Review 意见」预填（或手输）后经「发送反馈」提交即
  // request_revision（服务端进入 Revision 并由作者按反馈重写，完成后回门）。
  // 审计与 confirm 同源（operation=feedback）；仅 story/design 会话接线，
  // WorkItemPlan 门（plan-repair 语义）不经过本回调。
  const sendRevisionFeedback = useCallback((feedback: string): boolean => {
    const current = useWorkspaceStore.getState();
    if (
      !isStoryDesignAuthorConfirm(current) ||
      gateActionBlockReason(current) !== null
    ) {
      return false;
    }
    const sent = workspaceWs.sendRequestRevision(feedback);
    if (sent) {
      useOperationAuditStore.getState().record({
        sessionId: current.sessionId ?? sessionId,
        gateId: selectGateProjection(current)?.key ?? null,
        operation: "feedback",
        source: "chat",
        outcome: "sent",
        detail: feedback,
      });
    }
    return sent;
  }, [sessionId, workspaceWs.sendRequestRevision]);

  // P0 1.3（REQ-WIGA-05）Task 10：无 driver 的人工门命令族出站——门面
  //（cockpit-action-routing）已裁决 owner=server 才路由到本发送器，且
  // expected_gate_id 由 activeNodeId/timeline 收口；此处与 sendHttpConfirm 同款
  // 纪律：sent 先落审计，回执 accepted 补 completed，busy/4xx 落 rejected 并亮
  // 协议错误面（拒收不得零反馈）。compile_recovery 与既有 WS 通路一致不进
  // 操作审计（CockpitOperation 无该语义，不臆造新类目）。
  //
  // P2（REQ-WSC-15 归因扩展，REST 通道收口）：排除-only 命令（feedback 空串 +
  // excluded_finding_ids 非空，出站形态见 cockpit-action-routing feedback 的
  // REST 分支）失败时，除全局协议错误面外还要落 store 级 gateExclusionFailure
  // ——与 WS WsOutMessage::Error 同一条归因通道（同一份失败事实、同一套门卡
  // 撤销与文案），门卡据此撤销乐观「已排除」并就地给可读原因；受理成功则与
  // WS human_gate_exclusions_accepted 回执同款对账清失败事实。非排除动作
  //（纯反馈/终止/compile recovery）的 REST 失败不归因。
  const sendHumanActionRest = useCallback(
    (action: WorkspaceHumanAction): boolean => {
      const current = useWorkspaceStore.getState();
      const targetSessionId = current.sessionId ?? sessionId;
      if (targetSessionId === null) {
        return false;
      }
      const carriesExclusions =
        action.type === "feedback" && (action.excluded_finding_ids ?? []).length > 0;
      const operation =
        action.type === "feedback" ? "feedback" : action.type === "abandon" ? "abandon_gate" : null;
      const auditRecordId =
        operation === null
          ? null
          : useOperationAuditStore.getState().record({
              sessionId: targetSessionId,
              gateId: selectGateProjection(current)?.key ?? null,
              operation,
              source: "chat",
              outcome: "sent",
              detail: `rest-human-action:${action.type}`,
            });
      void postWorkspaceHumanAction(targetSessionId, action)
        .then((status) => {
          if (status.state === "accepted") {
            if (auditRecordId !== null) {
              useOperationAuditStore.getState().markCompleted(auditRecordId);
            }
            // 受理（含幂等重放）= 排除已 durable 落盘——清失败事实与归因线索，
            // 迟到的无关错误不得误归因成本次排除失败。
            if (carriesExclusions) {
              useWorkspaceStore.getState().resolveGateExclusion();
            }
            return;
          }
          if (auditRecordId !== null) {
            useOperationAuditStore.getState().markRejected(auditRecordId, status.state);
          }
          const refusalMessage = `人工命令被服务端拒绝（${status.state}）`;
          // busy：命令未被受理——排除未落盘，乐观「已排除」必须撤销。
          if (carriesExclusions) {
            attributeRestExclusionFailure(refusalMessage);
          }
          useWorkspaceStore
            .getState()
            .setProtocolError({
              code: `human_action_${status.state}`,
              message: refusalMessage,
            });
        })
        .catch((error: unknown) => {
          const code =
            typeof error === "object" && error !== null && "code" in error
              ? String(error.code)
              : "human_action_failed";
          const message =
            error instanceof Error && error.message !== ""
              ? error.message
              : "人工命令请求失败";
          if (auditRecordId !== null) {
            useOperationAuditStore.getState().markRejected(auditRecordId, code);
          }
          // 4xx/5xx（排除落盘失败 human_action_engine_error、gate_mismatch、
          // 引擎语义拒绝）与网络异常：排除未落盘（或不可知，fail-closed）——
          // 服务端 message 原文即门卡可读原因。
          if (carriesExclusions) {
            attributeRestExclusionFailure(message);
          }
          useWorkspaceStore.getState().setProtocolError({ code, message });
        });
      return true;
    },
    [sessionId],
  );

  // REQ-WSC-13/14（k3 P0-2C）：撤销排除出站——无会话时按失败呈现（不冒充
  // 成功）；HTTP/网络层错误由 postRevokeFindingExclusionsAction 原样带回
  // code/message（never-throw），门卡据此如实上屏。
  const sendRevokeFindingExclusions = useCallback(
    (action: RevokeFindingExclusionsAction): Promise<RevokeFindingExclusionsOutcome> => {
      const targetSessionId = useWorkspaceStore.getState().sessionId ?? sessionId;
      if (targetSessionId === null) {
        return Promise.resolve({
          ok: false,
          code: "revoke_finding_exclusions_no_session",
          message: "会话不可用，撤销排除未发送",
        });
      }
      return postRevokeFindingExclusionsAction(targetSessionId, action);
    },
    [sessionId],
  );

  return {
    confirmHttpGate,
    confirmBatchGate,
    routeGateConfirm,
    sendRevisionFeedback,
    sendHumanActionRest,
    sendRevokeFindingExclusions,
  };
}
