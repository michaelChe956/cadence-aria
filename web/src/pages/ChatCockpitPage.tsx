import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, ClipboardCopy } from "lucide-react";
import type { AuthorDecisionChoice } from "../api/types";
import { takeoverWorkspaceSession } from "../api/client";
import { fetchWorkspaceArtifactVersion } from "../api/workspace-content";
import {
  ChatEntryList,
  type ChatEntryListHandle,
} from "../components/chat-workspace/ChatEntryList";
import { ArtifactReviewPanel } from "../components/chat-workspace/ArtifactReviewPanel";
import { PendingChoiceNotice, pendingChoiceEntries } from "../components/chat-workspace/PendingChoiceNotice";
import {
  ChatInputBar,
  type ChatInputBarHandle,
} from "../components/chat-workspace/ChatInputBar";
import { TimelineNodeList } from "../components/chat-workspace/TimelineNodeList";
import { DisconnectBanner } from "../components/workspace/DisconnectBanner";
import { BulkConfirmReport } from "../components/chat-workspace/cockpit/BulkConfirmReport";
import { CockpitInbox } from "../components/chat-workspace/cockpit/CockpitInbox";
import { ConfirmTwiceButton } from "../components/chat-workspace/cockpit/ConfirmTwiceButton";
import { PlanApprovalPanel } from "../components/chat-workspace/cockpit/PlanApprovalPanel";
import {
  useCockpitObservedRecords,
  useCockpitSessionWatch,
  useCockpitSettings,
  useCockpitShellInbox,
} from "../components/cockpit/CockpitShell";
import { CockpitInboxDrawer } from "../components/cockpit/CockpitInboxDrawer";
import { HARD_ERROR_DRAWER_REFERENCE_NOTE } from "../state/protocol-error-copy";
import { STALE_DRIVER_LEASE_CODE } from "../state/workspace-cockpit-projection";
import {
  GATE_TERMINATE_BUTTON_LABEL,
  GATE_TERMINATE_CONFIRM_LABEL,
} from "../state/gate-prompt-copy";
import { CockpitPageHeader } from "../components/cockpit/CockpitPageHeader";
import { useWorkspaceContentLoaders } from "../hooks/useWorkspaceContentLoaders";
import { useLeaseDiagnostics } from "../hooks/useLeaseDiagnostics";
import { useCockpitAutopilot } from "../hooks/useCockpitAutopilot";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import type { WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import type { ChatEntry, ChoiceResponsePayload } from "../state/chat-entries";
import { createCockpitActionFacade } from "../state/cockpit-action-routing";
import {
  cockpitInboxItemSessionId,
  gateActionBlockReason,
  gateKindOf,
  isStoryDesignAuthorConfirm,
  selectGateProjection,
  selectCockpitFlow,
  type CockpitInboxItem,
} from "../state/workspace-cockpit-projection";
import { useBulkConfirmStore } from "../state/bulk-confirm-store";
import type { BulkConfirmTarget } from "../state/bulk-confirm-runner";
import { workspaceContentCacheValues } from "../state/workspace-content-cache";
import { watchWindowCopy } from "../state/workspace-observer-store";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { selectLatestReviewReport } from "../state/workspace-ws-selectors";
import {
  COCKPIT_HOTKEYS,
  type ConfirmTwiceButtonHandle,
} from "../state/cockpit-operation-semantics";
import { useOperationAuditStore } from "../state/operation-audit-store";
import {
  selectOperationAuditRows,
  type OperationAuditTarget,
} from "../state/operation-audit-projection";
import { OperationAuditView } from "../components/cockpit/OperationAuditView";
import { parentSessionIdFor } from "../state/parent-session-navigation";
import { useStageUI } from "../hooks/useStageUI";
import { useProviderDefaultsApplication } from "../hooks/useProviderDefaultsApplication";
import { writeWorkspaceProviderDefaults } from "../state/workspace-provider-defaults";
import {
  clampReviewRounds,
  latestUnacknowledgedAbortedNode,
  numericContentCacheValues,
  optionalWorkItemPlanReviewDecisionOptions,
  providerConfigFor,
  ProviderConfigDialogButton,
  requestIdFromEntry,
  scrollTargetEntryIdForNode,
  UNLOAD_GUARDED_STAGES,
  UNLOAD_GUARD_MESSAGE,
} from "./ChatWorkspacePageParts";
import {
  generationStatusText,
  RETAKABLE_LEASE_CODES,
  runningProviderName,
  StreamingConversationBlock,
  TERMINAL_NODE_STATUSES,
  terminalElapsedMs,
  type TerminalNodeContext,
  useNowTicker,
} from "./ChatCockpitPageParts";
import { useCockpitGateConfirm } from "./useCockpitGateConfirm";
import { useCockpitHotkeyActions } from "./useCockpitHotkeyActions";
import { useCockpitNodeDetailHydration } from "./useCockpitNodeDetailHydration";

export function ChatCockpitPage({
  sessionId,
  onBack,
  onOpenSession,
  workspaceWs,
}: {
  sessionId: string;
  onBack: () => void;
  onOpenSession: (sessionId: string) => void;
  workspaceWs: WorkspaceWsApi;
}) {
  const state = useWorkspaceStore();
  const now = useNowTicker(1000);
  const cockpitSettings = useCockpitSettings();
  useCockpitAutopilot({
    sessionId,
    state,
    settings: cockpitSettings,
    sendAdvance: workspaceWs.sendAdvance,
  });
  const [takeoverSessionId, setTakeoverSessionId] = useState<string | null>(null);
  const [inboxDrawerOpen, setInboxDrawerOpen] = useState(false);
  const [auditOpen, setAuditOpen] = useState(false);
  const [auditTarget, setAuditTarget] = useState<OperationAuditTarget | null>(null);
  const auditRecords = useOperationAuditStore((audit) => audit.records);
  const parentSessionId = parentSessionIdFor(
    state.planRepair,
    useOperationAuditStore.getState().takeoverLinks,
    sessionId,
  );
  const observedInbox = useCockpitShellInbox();
  const observedRecords = useCockpitObservedRecords();
  const watchSession = useCockpitSessionWatch();
  const watchWindow = watchWindowCopy(cockpitSettings.watchLimit);
  const selectedState =
    takeoverSessionId === null
      ? state
      : observedRecords.find((record) => record.sessionId === takeoverSessionId)?.state ?? null;
  const selectedSessionId = takeoverSessionId ?? sessionId;
  const pendingChoices = pendingChoiceEntries(selectedState?.chatEntries ?? []);
  const statusState = selectedState ?? state;
  const activeTimelineNode =
    statusState.timelineNodes.find((node) => node.node_id === statusState.activeNodeId) ??
    statusState.timelineNodes.at(-1) ??
    null;
  const activeStartedAtMs = activeTimelineNode
    ? Date.parse(activeTimelineNode.started_at)
    : NaN;
  const activeNodeTerminalStatus =
    activeTimelineNode !== null && TERMINAL_NODE_STATUSES[activeTimelineNode.status] === true
      ? (activeTimelineNode.status as TerminalNodeContext["status"])
      : null;
  // F-01：终态节点不挂活动计时——elapsed 冻结在节点结束时刻，不随墙钟增长。
  const terminalContext: TerminalNodeContext | null =
    activeTimelineNode !== null && activeNodeTerminalStatus !== null
      ? {
          status: activeNodeTerminalStatus,
          elapsedMs: terminalElapsedMs(activeTimelineNode),
        }
      : null;
  const runningContext =
    activeNodeTerminalStatus === null &&
    (statusState.providerStatus === "running" || statusState.providerStatus === "starting")
      ? {
          provider: runningProviderName(statusState.stage, statusState.providers ?? null),
          elapsedMs: Number.isNaN(activeStartedAtMs)
            ? null
            : Math.max(0, now - activeStartedAtMs),
        }
      : null;
  const isEmptyUnstarted =
    statusState.stage === "prepare_context" && statusState.timelineNodes.length === 0;
  const flowRows = useMemo(
    () => (selectedState ? selectCockpitFlow(selectedState, now) : []),
    [now, selectedState],
  );
  const contentCacheValues = useMemo(
    () => workspaceContentCacheValues(selectedState?.contentCache ?? state.contentCache),
    [selectedState?.contentCache, state.contentCache],
  );
  const { loadContent, cacheContent: cacheCurrentContent } = useWorkspaceContentLoaders(selectedSessionId);
  const cacheContent = takeoverSessionId === null ? cacheCurrentContent : undefined;
  const [drilldownNodeId, setDrilldownNodeId] = useState<string | null>(null);
  const [drilldownView, setDrilldownView] = useState<
    "conversation" | "plan" | "artifact"
  >("conversation");
  const [jumpEntryId, setJumpEntryId] = useState<string | null>(null);
  const [defaultsSavedAt, setDefaultsSavedAt] = useState<number | null>(null);
  const isCurrentSession = takeoverSessionId === null;
  const stageConfig = useStageUI(state.stage);
  const canConfigureProviders =
    isCurrentSession &&
    stageConfig.providerEditable &&
    workspaceWs.connectionStatus === "connected";
  const inboxEmptyHint =
    isCurrentSession && state.stage === "prepare_context" && (state.timelineNodes?.length ?? 0) === 0
      ? "会话尚未开始——在右侧选择 Provider 并点击「开始生成」"
      : null;
  const providerSummary = [
    `Author：${selectedState?.providers?.author ?? "claude_code"}`,
    selectedState?.reviewerEnabled
      ? `Reviewer：${selectedState.providers?.reviewer ?? "codex"}`
      : "未启用交叉审核",
  ].join(" · ");
  const abortedByDisconnectNode = useMemo(
    () =>
      latestUnacknowledgedAbortedNode(
        state.timelineNodes,
        state.acknowledgedAbortedNodes,
      ),
    [state.acknowledgedAbortedNodes, state.timelineNodes],
  );
  useUnloadGuard({
    enabled: UNLOAD_GUARDED_STAGES.has(state.stage),
    message: UNLOAD_GUARD_MESSAGE,
  });
  const handleStartGeneration = useCallback(() => {
    const { providers, reviewerEnabled, reviewRounds, permissionModes } =
      useWorkspaceStore.getState();
    workspaceWs.sendStartGeneration(
      providerConfigFor(
        providers,
        reviewerEnabled,
        reviewRounds,
        permissionModes,
      ),
      reviewerEnabled,
    );
  }, [workspaceWs.sendStartGeneration]);
  const handleSaveProviderDefaults = useCallback(() => {
    const { providers, reviewerEnabled } = useWorkspaceStore.getState();
    if (!providers) {
      return;
    }

    writeWorkspaceProviderDefaults({
      author: providers.author,
      reviewer: providers.reviewer ?? "codex",
      reviewerEnabled,
    });
    setDefaultsSavedAt(Date.now());
  }, []);
  const handlePermissionResponse = useCallback(
    (entry: ChatEntry, approved: boolean) => {
      const requestId = requestIdFromEntry(entry);
      if (!requestId) {
        return;
      }
      workspaceWs.respondPermission(requestId, approved, undefined);
    },
    [workspaceWs.respondPermission],
  );
  const handleChoiceResponse = useCallback(
    (entry: ChatEntry, response: ChoiceResponsePayload) => {
      const requestId = requestIdFromEntry(entry);
      if (!requestId) {
        return;
      }
      workspaceWs.sendChoiceResponse(
        requestId,
        response.selected_option_ids,
        response.free_text,
        response.answers,
      );
    },
    [workspaceWs.sendChoiceResponse],
  );
  const isPlanApprovalSession = selectedState?.workspaceType === "work_item_plan";
  // F-38：plan 会话此前被排除在「产物审核」之外——Work Item Plan 产物（artifact_versions
  // 单串 markdown）在 cockpit 里不可达。产物面板与页签扩到 work_item_plan；面板内的
  // story/design 定稿动作位仍按 author 门判据（isStoryDesignAuthorConfirm）渲染，不随
  // 页签可见性外溢（plan 门的相位不是 author_confirm）。
  const isArtifactReviewSession =
    selectedState?.workspaceType === "story" ||
    selectedState?.workspaceType === "design" ||
    selectedState?.workspaceType === "work_item_plan";
  const chatInputRef = useRef<ChatInputBarHandle | null>(null);
  const activeNode = useMemo(
    () => state.timelineNodes.find((node) => node.node_id === state.activeNodeId) ?? null,
    [state.activeNodeId, state.timelineNodes],
  );
  const latestReviewReport = useWorkspaceStore(selectLatestReviewReport);
  const changelogSummary = useMemo(() => {
    const lastCompletedRevision = (selectedState?.timelineNodes ?? [])
      .filter((node) => node.node_type === "revision" && node.status === "completed")
      .at(-1);
    const summary = lastCompletedRevision?.summary?.trim();
    return summary ? summary : undefined;
  }, [selectedState?.timelineNodes]);
  const artifactContentCacheValues = useMemo(
    () =>
      // 轮次缓存键是裸版本号，跨会话会碰撞（P1，审查 fix round 1）：
      // takeover 观测态不回退主 store 缓存，传空缓存走子会话 fetch——对齐 cacheContent 三元模式。
      takeoverSessionId === null
        ? numericContentCacheValues(state.artifactContentCache)
        : {},
    [takeoverSessionId, state.artifactContentCache],
  );
  const loadVersionMarkdown = useCallback(
    async (version: number) => {
      const response = await fetchWorkspaceArtifactVersion(selectedSessionId, version);
      return response.markdown;
    },
    [selectedSessionId],
  );
  const cacheVersionMarkdown = useCallback(
    (version: number, markdown: string) => {
      const storeState = useWorkspaceStore.getState();
      if (storeState.sessionId !== selectedSessionId) {
        return;
      }
      storeState.setArtifactContentCacheEntry(version, markdown);
    },
    [selectedSessionId],
  );
  // 退役留档（T5/REQ-RET-02）：handleAuthorDecision（cockpit author 决策面）
  // 随 author_decision 消息族删除（wp5-attribution-table.md §2）。
  const handleJumpToEntry = useCallback((entryId: string) => {
    setDrilldownView("conversation");
    setJumpEntryId(entryId);
  }, []);
  // 门确认/修订反馈发送族（sendHttpConfirm/confirmHttpGate/confirmBatchGate/
  // routeGateConfirm/sendRevisionFeedback，含 F-20/F-29/F-31/REQ-PCG-01/02 与
  // v38 反馈通路注释）拆至 useCockpitGateConfirm.ts，纯移动零行为变化。
  const { confirmHttpGate, confirmBatchGate, routeGateConfirm, sendRevisionFeedback } =
    useCockpitGateConfirm({ sessionId, workspaceWs });
  // v40 复验 #3：门面 adoptReview 接线——与主区/产物审核面板「采纳 Review
  // 意见」同款行为：最新 review 报告预填为修订反馈（ChatInputBar prefill，
  // 经「发送反馈」走 sendRevisionFeedback）并切回对话视图。纯客户端动作
  // （无 WS/HTTP 帧）；报告在调用时从 store 现读，与渲染判据
  // （latestReviewReport）同源且不吃 memo 闭包过期。
  const adoptLatestReview = useCallback(() => {
    const report = selectLatestReviewReport(useWorkspaceStore.getState());
    if (!report) {
      return;
    }
    chatInputRef.current?.prefill(`按以下 review 意见修订：\n\n${report}`);
    setDrilldownView("conversation");
  }, []);
  // F-38：门卡「查看产物」入口——切到产物审核页签（确认者就地看到待确认产物全文与
  // 评审结论）。纯视图动作，无 WS/HTTP 帧；无产物版本时门卡不渲染该入口（fail-closed）。
  const openArtifactView = useCallback(() => {
    setDrilldownView("artifact");
  }, []);
  const actions = useMemo(
    () =>
      createCockpitActionFacade({
        flowKind: state.flowKind,
        commandId:
          typeof state.humanGateTurn?.command_id === "string"
            ? state.humanGateTurn.command_id
            : null,
        getState: useWorkspaceStore.getState,
        sendConfirm: routeGateConfirm,
        sendAbandonGate: workspaceWs.sendAbandonGate,
        sendHumanGateFeedback: workspaceWs.sendHumanGateFeedback,
        sendAdvance: workspaceWs.sendAdvance,
        adoptReview: adoptLatestReview,
        sendBatchConfirm: confirmBatchGate,
        sendCompileRecovery: workspaceWs.sendWorkItemPlanCompileRecoveryAction,
      }),
    [
      state.flowKind,
      state.humanGateTurn?.command_id,
      state.humanGateClosure,
      state.humanGateSnapshot,
      state.sessionStatus,
      state.stage,
      workspaceWs.sendAdvance,
      routeGateConfirm,
      adoptLatestReview,
      workspaceWs.sendHumanGateFeedback,
      confirmBatchGate,
      workspaceWs.sendWorkItemPlanCompileRecoveryAction,
    ],
  );
  const auditRows = useMemo(
    () => selectOperationAuditRows({
      local: auditRecords,
      workspaceState: selectedState,
      codingState: null,
      target: auditTarget,
    }),
    [auditRecords, auditTarget, selectedState],
  );
  const openAudit = useCallback(() => {
    const auditState = selectedState ?? state;
    setAuditTarget({
      sessionId: auditState.sessionId ?? sessionId,
      gateId: selectGateProjection(auditState)?.key ?? null,
    });
    setAuditOpen(true);
  }, [selectedState, sessionId, state]);
  // k3 P2-2：author_confirm 阶段矩阵不放行 advance——即便 confirmed（HTTP confirm
  // 乐观态）也不露出「手动推进」（门面 advance 同款早退兜底）。
  const canManualAdvance =
    (state.humanGateClosure?.decision === "confirm" ||
      state.sessionStatus === "confirmed") &&
    state.stage !== "author_confirm";
  // F-30（v34 复验 session_0008）：终态会话（confirmed/terminated）即便页面
  // 还停在 prepare_context（session_state 未达窗口/长开 tab 残留），「开始生成」
  // 也必须禁用+就地如实提示——终态重跑是未定义路径（重跑走修订流程或新建会话）。
  const startGenerationBlockedByTerminalSession =
    state.stage === "prepare_context" &&
    (state.sessionStatus === "confirmed" || state.sessionStatus === "terminated");
  const startGenerationBlockedHint =
    state.sessionStatus === "terminated"
      ? "会话已终止（终态）——请新建会话"
      : "会话已确认（终态）——重新生成请走修订流程或新建会话";
  const handleTakeover = async (parentSessionId: string) => {
    const child = await takeoverWorkspaceSession(parentSessionId);
    useOperationAuditStore.getState().recordTakeoverLink(
      child.workspace_session_id,
      child.parent_session_id,
      child.takeover_event_id,
    );
    sessionStorage.setItem(
      `aria.takeover-parent:${child.workspace_session_id}`,
      child.parent_session_id,
    );
    useOperationAuditStore.getState().record({
      sessionId: parentSessionId,
      gateId: null,
      operation: "takeover",
      source: "takeover",
      outcome: "completed",
      detail: child.workspace_session_id,
    });
    watchSession(child.workspace_session_id);
    setTakeoverSessionId(child.workspace_session_id);
  };
  const handleBulkConfirm = useCallback((items: readonly CockpitInboxItem[]) => {
    // 当前会话必须复用既有 live driver 连接；另开 driver WS 会接管其 lease，
    // 令后续写操作收到 STALE_DRIVER_LEASE。
    const currentSessionItems = items.filter((item) => item.id.startsWith(`${sessionId}:gate:`));
    const otherSessionItems = items.filter((item) => {
      const itemSessionId = cockpitInboxItemSessionId(item.id);
      return itemSessionId !== null && itemSessionId !== sessionId;
    });
    if (currentSessionItems.length > 0) {
      const current = useWorkspaceStore.getState();
      const gate = selectGateProjection(current);
      if (gate !== null && gate.action_block_reason === null) {
        const expectedId = `${sessionId}:gate:${gate.key}`;
        if (currentSessionItems.some(
          (item) => item.id === expectedId && item.kind === "gate" &&
            item.gate?.key === gate.key && item.gate.action_block_reason === null,
        )) {
          actions.confirm();
        }
      }
    }
    if (otherSessionItems.length > 0) {
      useBulkConfirmStore.getState().start(
        otherSessionItems
          .map((item) => ({
            itemId: item.id,
            sessionId: cockpitInboxItemSessionId(item.id) ?? "",
            gateKey: item.gate?.key ?? null,
            title: item.title,
          }))
          .filter((target): target is BulkConfirmTarget =>
            target.sessionId !== "" && target.gateKey !== null,
          ),
      );
    }
  }, [actions, sessionId]);
  const handleRetry = useCallback((item: { id: string; source: string }) => {
    if (item.source !== "advance") {
      return;
    }
    const commandId = item.id.slice(item.id.lastIndexOf(":") + 1);
    workspaceWs.sendAdvance(commandId);
  }, [workspaceWs.sendAdvance]);
  // F-11/F-28 二轮：裸 driver 抢走租约（STALE_DRIVER_LEASE）或本连接以
  // observer 身份重连（OBSERVER_WRITE_REJECTED）后写操作被拒——重发 driver
  // hello 即重新持有租约（服务端 bind_role 对既有连接同样执行 lease.acquire），
  // 并撤下协议错误条目。
  const handleRetakeLease = useCallback(() => {
    const current = useWorkspaceStore.getState();
    if (
      current.protocolError === null ||
      RETAKABLE_LEASE_CODES[current.protocolError.code] !== true
    ) {
      return;
    }
    const lastSeenNodeId =
      current.activeNodeId ?? current.timelineNodes.at(-1)?.node_id ?? null;
    workspaceWs.sendHello(sessionId, lastSeenNodeId);
    current.setProtocolError(null);
  }, [sessionId, workspaceWs.sendHello]);
  // F-28 二轮（v34 复验「失败态点开始生成零反馈」）：classifyProtocolError
  // 把 gate/advance 分流之外的全部协议错误落 store.protocolError——就地面
  // 此前只认 STALE_DRIVER_LEASE 一码，长开 tab 断线重连后拒收码变为
  // OBSERVER_WRITE_REJECTED 或其它 hard_error 时生成按钮旁零显示。泛化为
  // hard_error 全族：lease 拒收两码给「重新接管」（复用上面 F-11 回调链），
  // 其余码报错误码+建议刷新（错误面仍与 F-30 终态禁用提示同屏共存）。
  // F-50 裁决 7（动作面归属）：当前会话 hard error 的完整动作面只有一个——
  // 抽屉打开时归收件箱错误条（当前会话条目 actionable），页级 ChatInputBar 缩为
  // 引用面（只报状态与处理入口，不重复重接管按钮）；抽屉收起时页级自持完整
  // 动作面。判据用抽屉开合状态，不新增协议字段（scope 契约归 lease change C 面）。
  // REQ-DLS-03 规范句「既有 STALE_DRIVER_LEASE 错误面 SHALL 引用最近相关转移
  // 事件」：当前会话 STALE 手动错误面激活时拉取只读诊断端点（REQ-DLS-02 单次
  // 自动重试失败后回落此面），页级错误面与抽屉错误条共用同一份摘要；端点失败
  // 静默降级为不展示——诊断不可用不阻塞恢复动作（重接管/刷新建议照常可用）。
  const staleLeaseNoticeActive =
    isCurrentSession && state.protocolError?.code === STALE_DRIVER_LEASE_CODE;
  const staleLeaseEvents = useLeaseDiagnostics(sessionId, staleLeaseNoticeActive);
  const hardErrorNotice =
    isCurrentSession && state.protocolError !== null
      ? {
          code: state.protocolError.code,
          message: state.protocolError.message,
          onRetakeLease:
            inboxDrawerOpen || RETAKABLE_LEASE_CODES[state.protocolError.code] !== true
              ? null
              : handleRetakeLease,
          referenceNote: inboxDrawerOpen ? HARD_ERROR_DRAWER_REFERENCE_NOTE : null,
          leaseEvents: staleLeaseNoticeActive ? staleLeaseEvents : null,
        }
      : null;
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const takeoverButtonRef = useRef<ConfirmTwiceButtonHandle | null>(null);
  const takeoverTargetSessionId = useMemo(
    () =>
      observedInbox.find(
        (item) =>
          item.kind === "stopped" && item.id.startsWith(`${sessionId}:`),
      )?.id.split(":")[0] ?? null,
    [observedInbox, sessionId],
  );
  // 收件箱抽屉聚焦挂起（F-31 visibility:hidden 子树 focus 失效补偿）与
  // confirm/feedback/takeover/advance 四组快捷键接线拆至 useCockpitHotkeyActions.ts，
  // 纯移动零行为变化。
  useCockpitHotkeyActions({
    inboxDrawerOpen,
    setInboxDrawerOpen,
    takeoverTargetSessionId,
    takeoverButtonRef,
    routeGateConfirm,
    adoptLatestReview,
    confirmBatchGate,
    workspaceWs,
  });
  const drilldownEntryId = useMemo(
    () =>
      drilldownNodeId && selectedState
        ? scrollTargetEntryIdForNode(selectedState.chatEntries, drilldownNodeId)
        : null,
    [drilldownNodeId, selectedState],
  );
  // F-39：选中执行流阶段卡 = 「定位到该节点的对话气泡」。非对话流页签（产物审核/
  // 计划审批）下 ChatEntryList 整块卸载——chatListRef.current === null，此前只写
  // drilldownNodeId：视图不切回、滚动不发生、零反馈（静默 no-op）。阶段卡、诊断计数
  // 与断连横幅三个入口统一走本回调，选中即切回对话流。
  const handleSelectTimelineNode = useCallback((nodeId: string | null) => {
    setDrilldownNodeId(nodeId);
    setDrilldownView("conversation");
  }, []);

  useEffect(() => {
    // 视图不是对话流时列表未挂载，滚动无意义；drilldownView 入依赖保证从产物/计划
    // 页签切回对话流后重滚（此前只依赖 drilldownEntryId，切回不重滚）。
    if (drilldownView !== "conversation" || !drilldownEntryId) {
      return;
    }
    chatListRef.current?.scrollToEntry(drilldownEntryId);
  }, [drilldownEntryId, drilldownView]);

  // v40 复验 #3 后续（刷新水合）：节点 detail 水合去重 + completed/active 节点
  // detail 拉取 + 断连中止节点确认态恢复拆至 useCockpitNodeDetailHydration.ts，
  // 纯移动零行为变化。
  useCockpitNodeDetailHydration({
    sessionId,
    timelineNodes: state.timelineNodes,
    activeNodeId: state.activeNodeId,
  });

  useEffect(() => {
    if (!jumpEntryId) {
      return;
    }
    chatListRef.current?.scrollToEntry(jumpEntryId);
    setJumpEntryId(null);
  }, [jumpEntryId]);
  useEffect(() => {
    if (
      selectedState?.stage === "author_confirm" &&
      (selectedState.workspaceType === "story" || selectedState.workspaceType === "design")
    ) {
      setDrilldownView("artifact");
    }
  }, [selectedState?.stage, selectedState?.workspaceType]);
  useProviderDefaultsApplication({
    ws: workspaceWs,
    sessionId,
    connected: workspaceWs.connectionStatus === "connected",
    providerEditable: stageConfig.providerEditable,
    providers: state.providers,
    isTakeover: takeoverSessionId !== null,
  });

  return (
    <div
      data-testid="cockpit-page"
      className="flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]"
    >
      <CockpitPageHeader
        sessionId={sessionId}
        watchWindow={watchWindow}
        parentSessionId={parentSessionId}
        onBack={onBack}
        onOpenSession={onOpenSession}
        onOpenAudit={openAudit}
        canManualAdvance={canManualAdvance}
        onAdvance={actions.advance}
        inboxCount={observedInbox.length}
        inboxOpen={inboxDrawerOpen}
        onToggleInbox={() => setInboxDrawerOpen((open) => !open)}
      />
      {isCurrentSession ? (
        <DisconnectBanner
          isReconnecting={workspaceWs.isReconnecting}
          attemptCount={workspaceWs.reconnectAttemptCount}
          onManualReconnect={workspaceWs.retryNow}
          abortedByDisconnect={
            abortedByDisconnectNode
              ? {
                  nodeId: abortedByDisconnectNode.node_id,
                  ts:
                    abortedByDisconnectNode.completed_at ??
                    abortedByDisconnectNode.started_at,
                }
              : null
          }
          onAcknowledge={(nodeIds) =>
            useWorkspaceStore.getState().setAcknowledgedAbortedNodes(nodeIds)
          }
          onViewTimeline={
            abortedByDisconnectNode
              ? () => handleSelectTimelineNode(abortedByDisconnectNode.node_id)
              : undefined
          }
          recoverableInterruptedRun={state.recoverableInterruptedRun}
          onRetryInterruptedRun={workspaceWs.retryInterruptedRun}
          retryResetKey={
            state.protocolError
              ? `${state.protocolError.code}:${state.protocolError.message}`
              : state.error
          }
        />
      ) : null}
      {auditOpen ? (
        <div className="border-b border-[var(--aria-line)] bg-[var(--aria-panel)]">
          <div className="flex items-center justify-between gap-2 border-b border-[var(--aria-line)] px-3 py-2">
            <h2 className="text-sm font-semibold text-[var(--aria-ink)]">操作审计</h2>
            <button
              type="button"
              onClick={() => setAuditOpen(false)}
              className="min-h-11 rounded-md px-3 text-xs font-semibold text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              收起审计
            </button>
          </div>
          <OperationAuditView
            rows={auditRows}
            target={auditTarget}
            onTargetChange={setAuditTarget}
          />
        </div>
      ) : null}

      {/* 主区：执行流退居左侧窄栏（w-48 纵向自上而下，不与对话流抢纵向空间），
          对话流成为主区域占满余宽；待处理仍由页头入口打开的右侧抽屉承载（UI-A）。 */}
      <main
        data-testid="cockpit-main-region"
        className="flex min-h-0 flex-1 flex-col gap-2 p-2"
      >
        <BulkConfirmReport />
        <div data-testid="cockpit-main-columns" className="flex min-h-0 flex-1 flex-row gap-2">
          <section
            data-testid="cockpit-execution-flow"
            aria-label="自动执行流"
            className="grid min-h-0 w-48 shrink-0 grid-rows-[auto_minmax(0,1fr)] rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)]"
          >
            {/* 窄栏页头：标题 + 诊断计数一行，生成状态换行跟随（信息不减，仅纵排）。 */}
            <div className="flex flex-col gap-1 border-b border-[var(--aria-line)] px-2 py-2">
              <div className="flex items-center justify-between gap-1.5">
                <h2 className="text-sm font-semibold text-[var(--aria-ink)]">自动执行流</h2>
                <button
                  type="button"
                  data-testid="cockpit-protocol-diagnostic-count"
                  onClick={() => handleSelectTimelineNode(selectedState?.activeNodeId ?? null)}
                  className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[11px] text-[var(--aria-ink-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  诊断 {selectedState?.protocolDiagnostics.length ?? 0}
                </button>
              </div>
              <p
                data-testid="cockpit-generation-status"
                role="status"
                className="text-xs leading-snug text-[var(--aria-ink-muted)]"
              >
                {generationStatusText(
                  isEmptyUnstarted ? "not_started" : statusState.providerStatus,
                  statusState.stage,
                  runningContext,
                  terminalContext,
                )}
              </p>
            </div>
            <TimelineNodeList
              nodes={selectedState?.timelineNodes ?? []}
              // 下钻选中的行即 ② 区的「当前步」（aria-current="step"）；未下钻时退回 store 的进行中节点。
              activeNodeId={drilldownNodeId ?? selectedState?.activeNodeId ?? null}
              selectedNodeId={drilldownNodeId}
              onSelectNode={handleSelectTimelineNode}
              variant="flow"
              flowRows={flowRows}
              nodeDetails={selectedState?.nodeDetails ?? {}}
              className="border-0"
            />
          </section>

          <section
            data-testid="cockpit-conversation-flow"
            aria-label="下钻对话流"
            className={[
              "grid min-h-0 min-w-0 flex-1 rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)]",
              pendingChoices.length > 0
                ? "grid-rows-[auto_auto_minmax(0,1fr)_auto_auto]"
                : "grid-rows-[auto_minmax(0,1fr)_auto_auto]",
            ].join(" ")}
          >
          <div className="flex min-w-0 items-center gap-2 px-3 py-2">
            <h2 className="text-sm font-semibold text-[var(--aria-ink)]">对话流</h2>
            {/* F-39：选中了阶段卡却在对话流里找不到对应气泡（未生成/未水合）时必须给出
                可辨反馈——此前静默 no-op，用户只看到「点了没反应」。 */}
            {drilldownView === "conversation" && drilldownNodeId && !drilldownEntryId ? (
              <span
                data-testid="drilldown-no-target-hint"
                role="status"
                className="text-xs leading-snug text-[var(--aria-ink-muted)]"
              >
                该阶段在对话流里没有可定位的内容（可能尚未生成或未水合）
              </span>
            ) : null}
            {canConfigureProviders ? (
              <>
                <ProviderConfigDialogButton
                  providers={state.providers}
                  editable={true}
                  onSelectProvider={(role, provider) =>
                    workspaceWs.selectProvider(role, provider)
                  }
                  reviewerEnabled={state.reviewerEnabled}
                  onToggleReviewer={(enabled) =>
                    useWorkspaceStore.setState({ reviewerEnabled: enabled })
                  }
                  permissionModes={state.permissionModes}
                  onPermissionModeSelect={(role, mode) =>
                    useWorkspaceStore.getState().setPermissionMode(role, mode)
                  }
                  rounds={state.reviewRounds}
                  onChangeRounds={(rounds) =>
                    useWorkspaceStore.setState({
                      reviewRounds: clampReviewRounds(rounds),
                    })
                  }
                />
                <button
                  type="button"
                  data-testid="save-provider-defaults"
                  onClick={handleSaveProviderDefaults}
                  className="btn-secondary h-9"
                >
                  设为默认
                </button>
                {defaultsSavedAt !== null ? (
                  <span role="status" className="text-xs text-[var(--aria-ink-muted)]">
                    已设为默认
                  </span>
                ) : null}
              </>
            ) : (
              <span className="text-xs text-[var(--aria-ink-muted)]">
                {providerSummary}
              </span>
            )}
            {isArtifactReviewSession || isPlanApprovalSession ? (
              <div
                role="tablist"
                aria-label="下钻视图"
                className="ml-auto flex items-center gap-1"
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={drilldownView === "conversation"}
                  data-testid="cockpit-conversation-tab"
                  onClick={() => setDrilldownView("conversation")}
                  className="inline-flex min-h-11 items-center rounded-md px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  对话流
                </button>
                {isArtifactReviewSession ? (
                  <button
                    type="button"
                    role="tab"
                    aria-selected={drilldownView === "artifact"}
                    data-testid="cockpit-artifact-review-tab"
                    onClick={() => setDrilldownView("artifact")}
                    className="inline-flex min-h-11 items-center rounded-md px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                  >
                    产物审核
                  </button>
                ) : null}
                {isPlanApprovalSession ? (
                  <button
                    type="button"
                    role="tab"
                    aria-selected={drilldownView === "plan"}
                    data-testid="cockpit-plan-approval-tab"
                    onClick={() => setDrilldownView("plan")}
                    className="inline-flex min-h-11 items-center rounded-md px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                  >
                    计划审批
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>
          {pendingChoices.length > 0 ? (
            <PendingChoiceNotice
              entries={selectedState?.chatEntries ?? []}
              onJump={handleJumpToEntry}
            />
          ) : null}
          {drilldownView === "artifact" && isArtifactReviewSession ? (
            <ArtifactReviewPanel
              artifactVersions={selectedState?.artifactVersions ?? []}
              artifact={selectedState?.artifact ?? null}
              sessionId={isCurrentSession ? sessionId : null}
              artifactContentCache={artifactContentCacheValues}
              loadArtifactVersion={loadVersionMarkdown}
              onCacheArtifactContent={cacheVersionMarkdown}
              changelogSummary={changelogSummary}
              onClose={() => setDrilldownView("conversation")}
              actions={
                isCurrentSession &&
                // F-38：动作位属于 story/design author 门本身（不是「阶段名等于
                // author_confirm」——plan 的 batch_confirm 门同样停在 author_confirm，
                // 套用旧判据会把 story/design 的定稿面泄到 plan 门上）。
                isStoryDesignAuthorConfirm(state) &&
                gateActionBlockReason(state) === null ? (
                  <>
                    {latestReviewReport ? (
                      <button
                        type="button"
                        className="btn-secondary h-9"
                        onClick={() => {
                          chatInputRef.current?.prefill(
                            `按以下 review 意见修订：\n\n${latestReviewReport}`,
                          );
                          setDrilldownView("conversation");
                        }}
                      >
                        <ClipboardCopy className="h-4 w-4" /> 采纳 Review 意见
                      </button>
                    ) : null}
                    {/* F-18/F-20 决策面（对照门卡/收件箱先例）：确认=门面 confirm
                        （story/design author 门经 routeGateConfirm 走 HTTP confirm
                        端点，定稿缺省）；终止=门面 terminate（WS abandon_human_gate）
                        +二次确认。F-31 纠偏：review 启用的会话追加「确认并评审」
                        （with_review=true，服务端接管进入评审轮）——评审由用户
                        选择，不再强制进入；未启用不露出该选择。 */}
                    <button
                      type="button"
                      className="btn-primary h-9"
                      onClick={() => actions.confirm()}
                    >
                      <Check className="h-4 w-4" aria-hidden="true" /> 确认定稿
                    </button>
                    {state.reviewerEnabled && gateKindOf(state) !== "batch_confirm" ? (
                      <button
                        type="button"
                        className="btn-secondary h-9"
                        onClick={() => confirmHttpGate(true)}
                      >
                        确认并评审
                      </button>
                    ) : null}
                    <ConfirmTwiceButton
                      variant="ghost"
                      label={GATE_TERMINATE_BUTTON_LABEL}
                      confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
                      onConfirm={actions.terminate}
                    />
                  </>
                ) : undefined
              }
              className="min-h-0"
            />
          ) : drilldownView === "plan" && isPlanApprovalSession ? (
            <PlanApprovalPanel
              key={selectedSessionId}
              sessionId={selectedSessionId}
              state={selectedState ?? state}
              onJumpToEntry={handleJumpToEntry}
              artifactContentCache={artifactContentCacheValues}
              loadVersionMarkdown={loadVersionMarkdown}
              onCacheVersionMarkdown={cacheVersionMarkdown}
            />
          ) : (
            <ChatEntryList
              ref={chatListRef}
              entries={selectedState?.chatEntries ?? []}
              actions={takeoverSessionId === null ? actions : undefined}
              onOpenArtifact={
                takeoverSessionId === null && isArtifactReviewSession
                  ? openArtifactView
                  : undefined
              }
              onPermissionResponse={
                takeoverSessionId === null ? handlePermissionResponse : undefined
              }
              onChoiceResponse={takeoverSessionId === null ? handleChoiceResponse : undefined}
              contentCache={contentCacheValues}
              loadContent={loadContent}
              onCacheContent={cacheContent}
              sessionId={selectedSessionId}
              testId="cockpit-conversation-flow-list"
            />
          )}
          {takeoverSessionId === null && state.streamBuffers[state.activeNodeId ?? ""]?.chunks.length ? (
            <StreamingConversationBlock
              content={state.streamBuffers[state.activeNodeId ?? ""]?.chunks.join("") ?? ""}
              role={state.streamBuffers[state.activeNodeId ?? ""]?.role ?? "author"}
            />
          ) : null}
          {/* F-19（cadence/notes 2026-09-19 阶段4监控）：生成/评审/修订期必须
              暴露中止入口——此前仅 prepare_context/author_confirm 渲染输入条，
              codex run 楔死 27min 期间页面无任何脱困控件。矩阵
              （workspace_ws_handler/protocol.rs Running/CrossReview/Revision 臂）
              均已放行 WsInMessage::Abort；ChatInputBar 的 BUSY_STAGES 自带
              禁输入+仅中止钮形态。 */}
          {isCurrentSession &&
          (state.stage === "prepare_context" ||
            state.stage === "author_confirm" ||
            state.stage === "running" ||
            state.stage === "cross_review" ||
            state.stage === "revision") ? (
            <ChatInputBar
              ref={chatInputRef}
              stage={state.stage}
              activeNodeType={activeNode?.node_type ?? null}
              workItemPlanArtifact={state.workItemPlanArtifact}
              disabled={workspaceWs.connectionStatus !== "connected"}
              hideStartGeneration={Boolean(state.recoverableInterruptedRun)}
              startGenerationDisabled={startGenerationBlockedByTerminalSession}
              startGenerationDisabledHint={
                startGenerationBlockedByTerminalSession
                  ? startGenerationBlockedHint
                  : null
              }
              onSendContextNote={workspaceWs.sendContextNote}
              onSendRevisionFeedback={
                state.workspaceType === "story" || state.workspaceType === "design"
                  ? sendRevisionFeedback
                  : undefined
              }
              onStartGeneration={handleStartGeneration}
              onAbort={workspaceWs.abort}
              hardErrorNotice={hardErrorNotice}
              gateOpen={selectGateProjection(state) !== null}
            />
          ) : null}
          {/* 退役留档（T5/REQ-RET-02）：review_decision 动作条随消息族删除。 */}
          </section>
        </div>
      </main>
      <CockpitInboxDrawer open={inboxDrawerOpen} onClose={() => setInboxDrawerOpen(false)}>
        <CockpitInbox
          items={observedInbox}
          actions={actions}
          onTakeover={handleTakeover}
          onRetry={handleRetry}
          onRetakeLease={handleRetakeLease}
          actionableSessionId={sessionId}
          takeoverButtonRef={takeoverButtonRef}
          onBulkConfirm={handleBulkConfirm}
          emptyHint={inboxEmptyHint}
          artifactVersions={selectedState?.artifactVersions}
          latestReviewSummary={latestReviewReport ?? null}
          leaseEvents={staleLeaseNoticeActive ? staleLeaseEvents : null}
          repairReservation={state.repairReservation}
        />
      </CockpitInboxDrawer>
    </div>
  );
}
