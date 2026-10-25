import {
  ArrowLeft,
  CheckCircle2,
  Clock,
  History,
  Settings2,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { useRouter } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  ApiRequestError,
  deletePlanCodingWorkspaces,
  enterVerificationTriage,
  getCodingAttemptSnapshot,
  getPlanCodingProgress,
  listVerificationTriageRecords,
  readCodingPolicyText,
  reauthorizeCodingPolicy,
  rerunPlannedCommand,
  getVerificationCommandEvidence,
} from "../api/client";
import type {
  ChoiceResponseRequest,
  VerificationCommandEvidence,
  VerificationTriageRecord,
} from "../api/types";
import type { CodingAttemptAddress } from "../api/types";
import { CodingTimeline } from "../components/coding-workspace/CodingTimeline";
import { CodingDashboard } from "../components/coding-workspace/dashboard/CodingDashboard";
import { PendingChoiceNotice } from "../components/chat-workspace/PendingChoiceNotice";
import { useCockpitSettingsSlotRef } from "../components/cockpit/CockpitShell";
import {
  PlanRepairCenter,
  type PlanRepairAction,
} from "../components/coding-workspace/PlanRepairCenter";
import { CodingProviderConfigPanel } from "../components/coding-workspace/CodingProviderConfigPanel";
import { RoleRunHistoryPanel } from "../components/coding-workspace/RoleRunHistoryPanel";
import { OperationAuditView } from "../components/cockpit/OperationAuditView";
import { useModalBehavior } from "../components/shared/DialogShell";
import {
  ChatEntryList,
  type ChatEntryListHandle,
} from "../components/chat-workspace/ChatEntryList";
import {
  AutomationTerminatedBanner,
  AutomationTerminationConfirmSurface,
  AutomationTerminationEntryButton,
} from "../components/chat-workspace/cockpit/CockpitInboxWaitingCards";
import {
  useChoiceRespondCommand,
  type ChoiceRespondCommandState,
} from "../hooks/useChoiceRespondCommand";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useCockpitHotkeys } from "../hooks/useCockpitHotkeys";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import type { ChatEntry, ChoiceResponsePayload } from "../state/chat-entries";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useLinkedWorkspaceAmendmentStore } from "../state/linked-workspace-amendment-store";
import {
  COCKPIT_HOTKEYS,
  codingStartupRejectionCopy,
} from "../state/cockpit-operation-semantics";
import {
  selectOperationAuditRows,
  type OperationAuditTarget,
} from "../state/operation-audit-projection";
import { useOperationAuditStore } from "../state/operation-audit-store";
import type { PlanRepairSessionState } from "../state/plan-repair-session";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { CodingArtifactTabs } from "./CodingWorkspaceArtifacts";
import {
  ACTIVE_ATTEMPT_STATUSES,
  ActionButtons,
  CodingComposer,
  CodingPanelTabs,
  GatePanel,
  errorMessage,
  isVerificationTriageEligibleGate,
  lockedProviderRole,
  requestIdFromEntry,
} from "./CodingWorkspaceControls";
import { CodingWorkspaceGroupProgress } from "./CodingWorkspaceGroupProgress";
import {
  CodingPlanOverviewModal,
  type PlanOverviewOpenTarget,
} from "./CodingPlanOverviewModal";
import { PrepareExecutionPlanPanel, StatusBadge } from "./CodingWorkspaceReports";
import { planRepairActionGenerationKey } from "./plan-repair-action-generation";

type CodingWorkspaceDrawer = "providers" | "runs" | "audit";
type PendingRepairAction = {
  action: Exclude<PlanRepairAction, "adjust_scope" | "open_workspace">;
  childSessionId: string;
  amendmentId: string;
  repairGenerationKey: string;
  baselineWorkspaceStage: string;
  baselineSessionSnapshotGeneration: number;
};
type ScopedRepairActionError = Omit<PendingRepairAction, "action"> & {
  message: string;
};
type RepairActionScope = Pick<
  PendingRepairAction,
  "childSessionId" | "amendmentId" | "repairGenerationKey"
>;

// P1-c（REQ-MTG-12）：总览/全局待办下钻焦点——router 把 search 的
// focus/gateId/choiceId 解析后传入；choice 定位对话内选择卡、gate 定位门卡、
// final_confirm 定位确认区，其余 kind 进入页面即可见面不单独定位。
export type CodingWorkspaceFocusTarget = {
  attentionKind?: string;
  gateId?: string;
  choiceId?: string;
};

type PendingCardFocus =
  | { type: "gate"; gateId: string }
  | { type: "final_confirm" };

/**
 * F-B（owner 真机复验 2026-10-25）：「查看全部 WI」入口形态——
 * - inline（默认）：本页弹窗承载「全部 WI 总览」。不跨页导航——跨页会离开
 *   Coding Attempt 页，WS 断连并 reset 全局 store（当前 attempt 的运行上下文
 *   与门状态全丢，回来还要重连重建），这正是 owner 复验指出的问题。
 * - external：交回宿主的 onOpenPlanOverview 回调。抽屉宿主用它回到抽屉总览
 *   （那里总览已由抽屉承载，页面再弹一层是重复浮层）。
 */
export type PlanOverviewEntryMode = "inline" | "external";

export function CodingWorkspacePage({
  address,
  onBack,
  focus,
  onOpenPlanOverview,
  planOverviewMode = "inline",
}: {
  address: CodingAttemptAddress;
  onBack: () => void;
  /** P1-c：下钻定位目标（未携带时不做任何定位）。 */
  focus?: CodingWorkspaceFocusTarget;
  /** P1-c：GroupProgress「查看全部 WI」入口；未接线不渲染（携带 planId）。 */
  onOpenPlanOverview?: (planId: string) => void;
  /** 「查看全部 WI」入口形态；默认 inline（本页弹窗），external 交回宿主。 */
  planOverviewMode?: PlanOverviewEntryMode;
}) {
  const api = useCodingWorkspaceWs(address);
  // F-B：总览下钻到别的 attempt 需要导航能力。useRouter 在没有 RouterProvider
  // 时返回 undefined（页面单测即独立渲染），此处不抛错——拿不到路由就不导航。
  const router = useRouter({ warn: false });
  // P2-e（REQ-CRO-11）：choice 作答走 REST 命令谱系（同 command 重试/202
  // 复查/expired 不路由新 run），不再经 WS choice_response。
  const { respondChoice } = useChoiceRespondCommand(address, handleChoiceCommandState);
  const store = useCodingWorkspaceStore();

  // 批 2 T4（REQ-WIGA-09/08）：托管判据用 T3 同源投影（owner=server 即
  // managed，terminated 仍 managed）；终止态优先投影 terminated_at，其次
  // 本页提交的 durable 结果（同一 project/issue 才呈现，防跨页残留）。
  const automationManaged = store.automation?.owner === "server";
  const terminationTerminatedAt =
    store.automation?.terminated_at ??
    (store.planTermination.terminated !== null &&
      store.planTermination.terminated.projectId === (store.projectId ?? address.projectId) &&
      store.planTermination.terminated.issueId === (store.issueId ?? address.issueId)
      ? store.planTermination.terminated.terminatedAt
      : null);
  const openAutomationTermination = () => {
    useCodingWorkspaceStore.getState().openTerminationConfirm({
      projectId: store.projectId ?? address.projectId,
      issueId: store.issueId ?? address.issueId,
      enrollmentId: store.automation?.enrollment_id ?? null,
      policyRevision: store.automation?.policy_revision ?? null,
    });
  };
  const settingsSlotRef = useCockpitSettingsSlotRef();
  const planRepairApi = useWorkspaceWs(
    store.activePlanRepair?.childSessionId ?? null,
  );
  const planRepairWorkspaceError = useWorkspaceStore((state) => state.error);
  const planRepairProtocolError = useWorkspaceStore((state) => state.protocolError);
  const planRepairWorkspaceSessionId = useWorkspaceStore((state) => state.sessionId);
  const planRepairWorkspaceStage = useWorkspaceStore((state) => state.stage);
  const linkedAmendmentStatus = useLinkedWorkspaceAmendmentStore(
    (state) => state.status,
  );
  const linkedAmendmentSnapshot = useLinkedWorkspaceAmendmentStore(
    (state) => state.snapshot,
  );
  const linkedAmendmentError = useLinkedWorkspaceAmendmentStore(
    (state) => state.error,
  );
  const connected = store.connectionStatus === "connected";
  const activeTab = store.activeTab;
  const [activePanel, setActivePanel] = useState<"chat" | "results">("chat");
  // F-43 fix1：横幅「定位选择卡」请求的目标条目。对话列表只在 chat 页签且无
  // Plan Repair 时挂载，未挂载时 ref=null——直接 scrollToEntry 会静默无操作，
  // 故先记录目标 + 切回 chat，待列表重挂后由 effect 执行滚动（ChatCockpitPage
  // jumpEntryId 先例）。
  const [pendingJumpEntryId, setPendingJumpEntryId] = useState<string | null>(null);
  // P1-c：下钻定位的门卡/确认区目标——等元素真出现（快照/WS 帧有先后，
  // 可能晚于挂载）再 scrollIntoView，定位成功即消费。
  const [pendingCardFocus, setPendingCardFocus] = useState<PendingCardFocus | null>(null);
  const [groupDeleteBusy, setGroupDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [repairActionError, setRepairActionError] =
    useState<ScopedRepairActionError | null>(null);
  const [pendingRepairAction, setPendingRepairAction] =
    useState<PendingRepairAction | null>(null);
  const [activeDrawer, setActiveDrawer] = useState<CodingWorkspaceDrawer | null>(null);
  // F-B：「全部 WI 总览」本页弹窗的开关（inline 形态）。
  const [planOverviewOpen, setPlanOverviewOpen] = useState(false);
  const [auditTarget, setAuditTarget] = useState<OperationAuditTarget | null>(null);
  // 辅助抽屉浮层行为（body 锁/Esc/焦点移入归还/Tab 圈闭），与 DialogShell 同一规范。
  const drawerPanelRef = useRef<HTMLElement | null>(null);
  useModalBehavior({
    active: activeDrawer !== null,
    onClose: () => setActiveDrawer(null),
    closeDisabled: false,
    containerRef: drawerPanelRef,
  });
  const auditRecords = useOperationAuditStore((audit) => audit.records);
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const addressKey = JSON.stringify([address.projectId, address.issueId, address.attemptId]);
  const addressKeyRef = useRef(addressKey);
  const deleteGenerationRef = useRef(0);
  const pendingRepairActionRef = useRef<PendingRepairAction | null>(null);
  const groupDeleteCommandRef = useRef<{ planId: string; commandId: string } | null>(null);
  const mountedRef = useRef(false);
  if (addressKeyRef.current !== addressKey) {
    addressKeyRef.current = addressKey;
    deleteGenerationRef.current += 1;
  }
  // F-B：换 attempt（地址变化）后不再保留旧 attempt 的总览弹窗——弹窗里的
  // planId 与当前 attempt 不同源，留着会误导。
  useEffect(() => {
    setPlanOverviewOpen(false);
  }, [address.attemptId]);
  const childWorkspaceError =
    store.activePlanRepair &&
    planRepairWorkspaceSessionId === store.activePlanRepair.childSessionId
    ? planRepairProtocolError
      ? `${planRepairProtocolError.code}: ${planRepairProtocolError.message}`
      : planRepairWorkspaceError
    : null;
  const activeRepairActionPending = repairScopeMatches(
    pendingRepairAction,
    store.activePlanRepair,
  );
  const activeRepairActionError = repairScopeMatches(
    repairActionError,
    store.activePlanRepair,
  )
    ? repairActionError?.message ?? null
    : null;
  const planRepairActionStatus = store.activePlanRepair
    ? childWorkspaceError ??
      activeRepairActionError ??
      (activeRepairActionPending
        ? "正在提交 Repair 操作，等待 Child Workspace 响应。"
        : null) ??
      (planRepairApi.connectionStatus === "connected"
        ? null
        : "Child Workspace 正在连接，Repair 操作暂不可用。")
    : null;
  const pageError = store.activePlanRepair ? deleteError : planError ?? deleteError;
  // P1-c：GroupProgress「查看全部 WI」透传——planId 取 store（router 回调导航）。
  const groupPlanId = store.workItemGroupId;
  // F-B：「查看全部 WI」默认在本页弹窗内承载总览（inline）——不跨页导航，
  // 当前 attempt 的 WS/store 与门状态原地保留。external 形态（抽屉宿主）
  // 才交回宿主回调。
  const openGroupPlanOverview = groupPlanId
    ? planOverviewMode === "external"
      ? onOpenPlanOverview
        ? () => onOpenPlanOverview(groupPlanId)
        : undefined
      : () => setPlanOverviewOpen(true)
    : undefined;
  // F-B：总览里点行/徽标下钻到单个 Attempt 详情——不在弹窗内就地展开（会
  // 同时挂载第二个 attempt 的 WS/store），而是导航到目标 attempt：同一时刻
  // 只有一个 attempt 实例在页面上。
  const handleOpenOverviewAttempt = (target: PlanOverviewOpenTarget) => {
    setPlanOverviewOpen(false);
    if (target.attemptId === address.attemptId) {
      // 已在当前 attempt 详情上——关弹窗即可，不做重复导航。
      return;
    }
    void router?.navigate({
      to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
      params: {
        projectId: address.projectId,
        issueId: address.issueId,
        attemptId: target.attemptId,
      },
      search: {
        planId: groupPlanId ?? undefined,
        workItemId: target.workItemId,
        gateId: target.gateId,
        choiceId: target.choiceId,
      },
    });
  };
  const providerSummary = store.roleProviderConfigSnapshot
    ? `Coder ${store.roleProviderConfigSnapshot.coder} · Reviewer ${store.roleProviderConfigSnapshot.code_reviewer}`
    : "provider pending";
  const roleRunSummary =
    store.roleRuns.length === 0 ? "暂无运行记录" : `${store.roleRuns.length} 次角色运行`;
  const pendingGate = store.pendingGates.at(-1) ?? null;
  const storeMatchesAddress =
    store.projectId === address.projectId &&
    store.issueId === address.issueId &&
    store.attemptId === address.attemptId;
  const startupErrorCode = (() => {
    const code = store.protocolError?.code ?? null;
    if (!codingStartupRejectionCopy(code)) return null;
    const latestStartup = [...auditRecords]
      .reverse()
      .find(
        (record) =>
          record.sessionId === address.attemptId &&
          record.operation === "start_coding" &&
          record.source === "coding",
      );
    return latestStartup?.outcome === "sent" ||
      (latestStartup?.outcome === "rejected" && latestStartup.detail === code)
      ? code
      : null;
  })();
  const linkedAmendmentTargets = {
    story: normalizedRefs(store.workItemExecutionPlan?.story_refs),
    design: normalizedRefs(store.workItemExecutionPlan?.design_refs),
  };

  const auditRows = useMemo(
    () => selectOperationAuditRows({
      local: auditRecords,
      workspaceState: null,
      codingState: store,
      target: auditTarget,
    }),
    [auditRecords, auditTarget, store],
  );

  // ── C2 oracle C-2a/b：GatePanel 验证处理/并列证据数据源与操作 ──
  const pendingGateEligibleForTriage =
    pendingGate != null && pendingGate.kind === "blocked"
      ? isVerificationTriageEligibleGate(pendingGate)
      : false;
  const [triageRecords, setTriageRecords] = useState<VerificationTriageRecord[]>([]);
  const [commandEvidenceList, setCommandEvidenceList] = useState<
    VerificationCommandEvidence[]
  >([]);
  const [triageSurfaceError, setTriageSurfaceError] = useState<string | null>(null);
  const [triageActionBusy, setTriageActionBusy] = useState(false);
  const triageFetchGateId = pendingGate?.gate_id ?? null;

  useEffect(() => {
    if (!triageFetchGateId || !pendingGateEligibleForTriage || !storeMatchesAddress) {
      return;
    }
    let cancelled = false;
    setTriageSurfaceError(null);
    Promise.all([
      listVerificationTriageRecords(address),
      getVerificationCommandEvidence(address).catch(() => ({
        attempt_id: "",
        rework_count: 0,
        evidence: [] as VerificationCommandEvidence[],
      })),
    ])
      .then(([records, evidence]) => {
        if (cancelled) return;
        setTriageRecords(records.records);
        setCommandEvidenceList(evidence.evidence);
      })
      .catch((reason) => {
        if (cancelled) return;
        setTriageSurfaceError(errorMessage(reason, "验证处理记录读取失败"));
      });
    return () => {
      cancelled = true;
    };
  }, [triageFetchGateId, pendingGateEligibleForTriage, storeMatchesAddress, address]);

  const verificationTriage =
    triageRecords.find((record) => record.status === "pending") ??
    triageRecords[triageRecords.length - 1] ??
    null;
  const selectedEvidence = useMemo(() => {
    if (commandEvidenceList.length === 0) return null;
    const pendingCheckId = verificationTriage?.check_id;
    return (
      commandEvidenceList.find(
        (item) => pendingCheckId != null && item.check_id === pendingCheckId,
      ) ??
      commandEvidenceList.find((item) => item.mismatch) ??
      commandEvidenceList[0]
    );
  }, [commandEvidenceList, verificationTriage?.check_id]);

  async function refreshTriageSurface() {
    try {
      const [records, evidence] = await Promise.all([
        listVerificationTriageRecords(address),
        getVerificationCommandEvidence(address).catch(() => ({
          attempt_id: "",
          rework_count: 0,
          evidence: [] as VerificationCommandEvidence[],
        })),
      ]);
      setTriageRecords(records.records);
      setCommandEvidenceList(evidence.evidence);
    } catch {
      // 刷新失败保留既有呈现；下一次门变更会重拉。
    }
  }

  async function handleEnterVerificationTriage(gateId: string) {
    void gateId;
    if (!selectedEvidence) {
      setTriageSurfaceError("无可绑定的验证 check（请刷新后重试）");
      return;
    }
    const reportRef = pendingGate?.evidence_refs?.find((ref) => ref.trim().length > 0);
    // REQ-CVT-06：coder 门（coding_output_human_triage／
    // coding_operational_blocker）evidence_refs 恒空，回退绑定该门自身
    // （finding_id = gate_id#0，对应门的 plan defect finding）；CR 门
    // 仍要求 evidence_refs 命中 code review report。
    const coderGateFindingBase =
      pendingGate != null &&
      (pendingGate.reason_code === "coding_output_human_triage" ||
        pendingGate.reason_code === "coding_operational_blocker")
        ? pendingGate.gate_id
        : null;
    const findingBase = reportRef ?? coderGateFindingBase;
    if (!findingBase) {
      setTriageSurfaceError("门缺少 finding 绑定证据（evidence_refs 为空）");
      return;
    }
    setTriageActionBusy(true);
    setTriageSurfaceError(null);
    try {
      await enterVerificationTriage(address, {
        finding_id: `${findingBase}#0`,
        check_id: selectedEvidence.check_id,
        original_command: selectedEvidence.planned_command ?? null,
        alternative_command: null,
        cwd: null,
        outcome: null,
        test_execution_count: null,
        environment: null,
      });
      await refreshTriageSurface();
    } catch (reason) {
      setTriageSurfaceError(errorMessage(reason, "转入验证处理失败"));
    } finally {
      setTriageActionBusy(false);
    }
  }

  async function handleRerunPlannedCommand(checkId: string) {
    if (!pendingGate) return;
    setTriageActionBusy(true);
    setTriageSurfaceError(null);
    try {
      await rerunPlannedCommand(address, {
        command_id: `cmd-c2-rerun-${pendingGate.gate_id}-${checkId}`,
        gate_id: pendingGate.gate_id,
        check_id: checkId,
        expected_version: store.reworkCount,
      });
      await refreshTriageSurface();
    } catch (reason) {
      setTriageSurfaceError(errorMessage(reason, "重跑原计划命令失败"));
    } finally {
      setTriageActionBusy(false);
    }
  }

  // ── C2 oracle C-2c：blocked／rework 等待面受限政策读取/重新授权 ──
  const policySurfaceVisible =
    storeMatchesAddress &&
    (store.status === "blocked" || store.status === "waiting_for_human");
  const [policyText, setPolicyText] = useState<{
    policy_id: string;
    policy_revision: number;
    policy_digest: string;
    text: string;
  } | null>(null);
  const [policyAttemptVersion, setPolicyAttemptVersion] = useState<number | null>(null);
  const [policyError, setPolicyError] = useState<string | null>(null);
  const [policyBusy, setPolicyBusy] = useState(false);
  const [policyRole, setPolicyRole] = useState<"coder" | "reviewer">("coder");
  const [policyReauthStatus, setPolicyReauthStatus] = useState<string | null>(null);

  async function handleReadPolicy() {
    setPolicyBusy(true);
    setPolicyError(null);
    setPolicyReauthStatus(null);
    try {
      const response = await readCodingPolicyText(address);
      setPolicyText(response.policy);
      setPolicyAttemptVersion(response.attempt_version);
    } catch (reason) {
      setPolicyText(null);
      setPolicyError(errorMessage(reason, "读取政策失败（fail-closed 停等）"));
    } finally {
      setPolicyBusy(false);
    }
  }

  async function handlePolicyReauthorization() {
    if (!policyText || policyAttemptVersion == null) return;
    setPolicyBusy(true);
    setPolicyError(null);
    try {
      const result = await reauthorizeCodingPolicy(address, {
        command_id: `cmd-c2-policy-reauth-${address.attemptId}-${policyRole}`,
        attempt_id: address.attemptId,
        role: policyRole,
        policy_digest: policyText.policy_digest,
        expected_version: policyAttemptVersion,
      });
      setPolicyReauthStatus(
        result.state === "accepted"
          ? "已重新授权（仅下一次返修 run 运行态有效）"
          : result.state === "replayed"
            ? "重放首次授权结果"
            : `${result.state}${result.reason ? `：${result.reason}` : ""}`,
      );
    } catch (reason) {
      setPolicyError(errorMessage(reason, "重新授权失败"));
    } finally {
      setPolicyBusy(false);
    }
  }
  useUnloadGuard({
    enabled: store.status === "running",
    message: "Coding attempt 运行中。刷新/关闭可能中断当前操作，是否继续？",
  });

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      deleteGenerationRef.current += 1;
    };
  }, []);

  useEffect(() => {
    // 地址切换：整组删除 busy 复位（在途请求由地址守卫忽略结算）。
    setGroupDeleteBusy(false);
    setDeleteError(null);
    setPlanError(null);
    setPendingJumpEntryId(null);
    setPendingCardFocus(null);
    pendingRepairActionRef.current = null;
    setPendingRepairAction(null);
    setRepairActionError(null);
  }, [addressKey]);

  // F-43 fix1：目标条目确定后，等对话列表真挂载（chat 页签且无 Plan Repair）再滚动。
  // Plan Repair 期间列表被整块替换，此 effect 会保持挂起直到修复会话结束/退出。
  useEffect(() => {
    if (pendingJumpEntryId === null || store.activePlanRepair || activePanel !== "chat") {
      return;
    }
    const target = chatListRef.current;
    if (!target) {
      return;
    }
    target.scrollToEntry(pendingJumpEntryId);
    setPendingJumpEntryId(null);
  }, [activePanel, pendingJumpEntryId, store.activePlanRepair]);

  // P1-c（REQ-MTG-12）：总览徽标/A1 待办下钻焦点。choice 复用 pendingJumpEntryId
  // 先例（先切回对话页签，列表挂载后滚动）；gate 卡与 final_confirm 确认区由
  // pendingCardFocus 等元素真正出现再定位。focus 信号（地址+目标）只消费一次。
  const focusSignal = focus
    ? [
        addressKey,
        focus.attentionKind ?? "",
        focus.gateId ?? "",
        focus.choiceId ?? "",
      ].join("\n")
    : null;
  const focusSignalRef = useRef<string | null>(null);
  useEffect(() => {
    if (focusSignal === null || focusSignalRef.current === focusSignal) {
      return;
    }
    focusSignalRef.current = focusSignal;
    const target = focus;
    if (!target) {
      return;
    }
    if (target.choiceId) {
      setPendingJumpEntryId(`choice_request:${target.choiceId}`);
      setActivePanel("chat");
      return;
    }
    if (target.gateId) {
      setPendingCardFocus({ type: "gate", gateId: target.gateId });
      setActivePanel("chat");
      return;
    }
    if (target.attentionKind === "final_confirm") {
      setPendingCardFocus({ type: "final_confirm" });
    }
    // 其余 kind（blocked/manual_recovery 等）无独立定位面，进入页面即可见。
  }, [focusSignal]);

  // P1-c：定位消费——门卡要求 chat 页签且 gate_id 匹配当前 pendingGate；
  // 确认区横幅为页级元素，无页签前提。元素未出现时保持挂起等待。
  useEffect(() => {
    if (!pendingCardFocus) {
      return;
    }
    if (pendingCardFocus.type === "gate") {
      if (store.activePlanRepair || activePanel !== "chat") {
        return;
      }
      if (pendingGate?.gate_id !== pendingCardFocus.gateId) {
        return;
      }
      const card = document.querySelector<HTMLElement>(
        '[data-testid="coding-pending-gate"]',
      );
      if (!card) {
        return;
      }
      card.scrollIntoView({ block: "center" });
      setPendingCardFocus(null);
      return;
    }
    const banner = document.querySelector<HTMLElement>(
      '[data-testid="coding-final-confirm-waiting"]',
    );
    if (!banner) {
      return;
    }
    banner.scrollIntoView({ block: "center" });
    setPendingCardFocus(null);
  }, [
    pendingCardFocus,
    activePanel,
    pendingGate,
    store.activePlanRepair,
    store.stage,
    store.status,
    store.attemptScope,
  ]);

  useEffect(() => {
    const activeRepair = store.activePlanRepair;
    if (!repairScopeMatches(pendingRepairActionRef.current, activeRepair)) {
      pendingRepairActionRef.current = null;
      setPendingRepairAction(null);
    }
    setRepairActionError((current) =>
      repairScopeMatches(current, activeRepair) ? current : null,
    );
  }, [store.activePlanRepair]);

  useEffect(() => {
    const pending = pendingRepairActionRef.current;
    if (
      !pending ||
      planRepairWorkspaceSessionId !== pending.childSessionId ||
      !planRepairProtocolError ||
      !protocolErrorMatchesAction(pending.action, planRepairProtocolError.code)
    ) {
      return;
    }
    pendingRepairActionRef.current = null;
    setPendingRepairAction(null);
  }, [planRepairProtocolError, planRepairWorkspaceSessionId]);

  useEffect(() => {
    const pending = pendingRepairActionRef.current;
    if (
      pending &&
      planRepairWorkspaceSessionId === pending.childSessionId &&
      planRepairWorkspaceStage !== pending.baselineWorkspaceStage
    ) {
      pendingRepairActionRef.current = null;
      setPendingRepairAction(null);
    }
    setRepairActionError((current) =>
      current &&
      planRepairWorkspaceSessionId === current.childSessionId &&
      planRepairWorkspaceStage !== current.baselineWorkspaceStage
        ? null
        : current,
    );
  }, [planRepairWorkspaceSessionId, planRepairWorkspaceStage]);

  useEffect(() => {
    const snapshotGeneration = planRepairApi.sessionSnapshotGeneration;
    const pending = pendingRepairActionRef.current;
    if (
      pending &&
      planRepairWorkspaceSessionId === pending.childSessionId &&
      snapshotGeneration > pending.baselineSessionSnapshotGeneration
    ) {
      pendingRepairActionRef.current = null;
      setPendingRepairAction(null);
    }
    setRepairActionError((current) =>
      current &&
      planRepairWorkspaceSessionId === current.childSessionId &&
      snapshotGeneration > current.baselineSessionSnapshotGeneration
        ? null
        : current,
    );
  }, [planRepairApi.sessionSnapshotGeneration, planRepairWorkspaceSessionId]);

  function isCurrentDeleteRequest(requestAddressKey: string, requestGeneration: number) {
    return (
      mountedRef.current &&
      addressKeyRef.current === requestAddressKey &&
      deleteGenerationRef.current === requestGeneration
    );
  }

  // H5（设计 C，冻结契约先行、H4 端点后行）：plan 聚合标题区整组删除——服务端
  // 分类 → required 409 → 恰一次 window.confirm → confirmed=true 复删；取消零
  // 出站破坏请求。command_id 同一删除动作稳定复用（失败重试不换 key，服务端
  // 幂等回执）；expected_generation 取 GET workspace_lifecycle.generation 且同
  // 一操作内冻结重用（confirmed 复删不跨代授权）；GET 缺席/失败不阻断服务端
  // 预检（不发送该参数，服务端分类权威）。成功返回工作台（onBack 刷新
  // lifecycle 与 run facts）；typed 409/500 直达错误面，不自动重试。
  async function handleDeleteAllCodingWorkspaces() {
    const planId = store.workItemGroupId;
    // P2-1：防重复点击（防止 groupDeleteBusy 永久卡死清剩余唯一出口）。
    if (!planId || groupDeleteBusy || !storeMatchesAddress) {
      return;
    }
    const requestAddressKey = addressKey;
    const requestGeneration = deleteGenerationRef.current + 1;
    deleteGenerationRef.current = requestGeneration;
    const previous = groupDeleteCommandRef.current;
    const commandId =
      previous?.planId === planId
        ? previous.commandId
        : newPlanGroupDeleteCommandId();
    groupDeleteCommandRef.current = { planId, commandId };
    let generation: number | null = null;
    let generationResolved = false;
    const resolveGeneration = async () => {
      if (generationResolved) {
        return;
      }
      generationResolved = true;
      try {
        const progress = await getPlanCodingProgress(
          address.projectId,
          address.issueId,
          planId,
        );
        generation = progress.workspace_lifecycle?.generation ?? null;
      } catch {
        // 聚合缺席（旧服务端/未建 run）不阻断删除预检：省略 generation，服务端分类。
        generation = null;
      }
    };
    const sendDelete = async (confirmed: boolean) => {
      await resolveGeneration();
      return deletePlanCodingWorkspaces(address.projectId, address.issueId, planId, {
        command_id: commandId,
        ...(generation !== null ? { expected_generation: generation } : {}),
        ...(confirmed ? { confirmed: true } : {}),
      });
    };
    setGroupDeleteBusy(true);
    setDeleteError(null);
    try {
      try {
        await sendDelete(false);
      } catch (reason) {
        if (
          !(
            reason instanceof ApiRequestError &&
            reason.code === "deletion_confirmation_required"
          )
        ) {
          throw reason;
        }
        // 服务端分类文案权威（浅/强分级由 required 409 message 给出）；缺失时
        // 兜底强警告文案。
        const confirmationMessage =
          reason.message.trim().length > 0
            ? reason.message
            : "确定终止并删除本计划全部 Coding 工作区？运行中的成员会先终止，日志、测试输出与 worktree 将被删除且不可恢复。计划与工作项保留。";
        if (!window.confirm(confirmationMessage)) {
          return;
        }
        await sendDelete(true);
      }
      if (isCurrentDeleteRequest(requestAddressKey, requestGeneration)) {
        onBack();
      }
    } catch (reason) {
      if (isCurrentDeleteRequest(requestAddressKey, requestGeneration)) {
        setDeleteError(errorMessage(reason, "删除本计划全部 Coding 工作区失败"));
      }
    } finally {
      if (isCurrentDeleteRequest(requestAddressKey, requestGeneration)) {
        setGroupDeleteBusy(false);
      }
    }
  }

  function handleSelectTimelineNode(nodeId: string) {
    useCodingWorkspaceStore.getState().setSelectedNode(nodeId);
    const targetEntry = useCodingWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.node_id === nodeId);
    if (targetEntry) {
      chatListRef.current?.scrollToEntry(targetEntry.id);
    }
  }

  function handlePlanRepairAction(action: PlanRepairAction) {
    if (action === "adjust_scope" || action === "open_workspace") {
      return;
    }
    const activeRepair = store.activePlanRepair;
    const amendmentId = activeRepair?.amendment?.id;
    if (!activeRepair || !amendmentId) return;
    const currentPending = pendingRepairActionRef.current;
    if (repairScopeMatches(currentPending, activeRepair)) return;
    const pending: PendingRepairAction = {
      action,
      childSessionId: activeRepair.childSessionId,
      amendmentId,
      repairGenerationKey: planRepairActionGenerationKey(activeRepair),
      baselineWorkspaceStage: planRepairWorkspaceStage,
      baselineSessionSnapshotGeneration: planRepairApi.sessionSnapshotGeneration,
    };
    pendingRepairActionRef.current = pending;
    setPendingRepairAction(pending);
    if (planRepairWorkspaceSessionId === activeRepair.childSessionId) {
      const childStore = useWorkspaceStore.getState();
      childStore.setProtocolError(null);
      childStore.setError(null);
    }
    let sent = false;
    switch (action) {
      case "confirm":
        sent = planRepairApi.confirmPlanAmendment(amendmentId);
        break;
      case "regenerate":
        // L1 typed 重承载（REQ-RET-02）：regenerate 走 request_revision——WorkItemPlan
        // 子会话入站即 request_work_item_plan_revision（SC plan-repair 消费通道）。
        sent = planRepairApi.sendRequestRevision("要求重新生成 Plan Repair 修订");
        break;
      case "cancel":
        sent = planRepairApi.cancelPlanAmendment(amendmentId, "用户取消修订");
        break;
    }
    if (sent) {
      setRepairActionError(null);
      return;
    }
    pendingRepairActionRef.current = null;
    setPendingRepairAction(null);
    setRepairActionError({
      childSessionId: activeRepair.childSessionId,
      amendmentId,
      repairGenerationKey: planRepairActionGenerationKey(activeRepair),
      baselineWorkspaceStage: planRepairWorkspaceStage,
      baselineSessionSnapshotGeneration: planRepairApi.sessionSnapshotGeneration,
      message: "Plan Repair 操作发送失败，请检查 Child Workspace 连接。",
    });
  }

  const hotkeyHandlers = useMemo(
    () => ({
      confirm: () => {
        if (store.activePlanRepair) {
          handlePlanRepairAction("confirm");
          return;
        }
        if (
          store.stage === "final_confirm" &&
          store.status === "waiting_for_human" &&
          store.groupFinalReadiness?.status === "complete" &&
          store.groupFinalReadiness.diagnostics.length === 0
        ) {
          api.finalConfirm();
          return;
        }
        if (pendingGate?.kind === "stage_gate" && pendingGate.stage) {
          api.confirmStageGate(pendingGate.stage);
        }
      },
      feedback: () => {
        document.querySelector<HTMLTextAreaElement>("textarea[aria-label='补充 Coding 上下文']")?.focus();
      },
      takeover: () => {},
      advance: () => {
        if (
          startupErrorCode === null &&
          (store.stage === "prepare_context" ||
            (store.stage === "review_request" &&
              store.status !== null &&
              ACTIVE_ATTEMPT_STATUSES.has(store.status)))
        ) {
          api.startCoding();
        }
      },
    }),
    [api, pendingGate, startupErrorCode, store, COCKPIT_HOTKEYS],
  );
  useCockpitHotkeys(hotkeyHandlers);

  return (
    <div
      data-testid="coding-workspace-page"
      className="flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]"
    >
      <div
        data-testid="coding-workspace-top-bar"
        className="flex h-11 min-w-0 shrink-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3"
      >
        <button
          type="button"
          onClick={onBack}
          className="inline-flex h-8 shrink-0 items-center gap-2 rounded-md px-2 text-sm text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
        >
          <ArrowLeft className="h-4 w-4" />
          返回
        </button>
        <div className="min-w-0 flex-1 truncate text-center text-sm font-semibold">
          Coding Attempt #{store.attemptId ?? address.attemptId}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <StatusBadge value={store.status ?? "created"} />
          {connected ? (
            <Wifi aria-label="已连接" className="h-4 w-4 text-[var(--aria-success)]" />
          ) : (
            <WifiOff aria-label="未连接" className="h-4 w-4 text-[var(--aria-danger)]" />
          )}
          <button
            type="button"
            aria-label="操作审计"
            onClick={() => {
              setAuditTarget({ sessionId: store.attemptId ?? address.attemptId, gateId: null });
              setActiveDrawer("audit");
            }}
            className="inline-flex min-h-11 shrink-0 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <History aria-hidden="true" className="h-3.5 w-3.5" />
            <span>操作审计</span>
          </button>
          {/* F-43 ③：驾驶舱设置入口宿主（F-37 先例）。此前本页不登记 slot，
              CockpitShell 只能落 fixed 兜底浮层压住页面内容。 */}
          <div
            data-testid="cockpit-settings-slot"
            ref={settingsSlotRef}
            className="flex items-center"
          />
        </div>
      </div>

      {/* F-43 ②：未处理 choice 的到达提示挂在**页级**（顶栏之下），不随
          「运行对话/结果」页签切换消失——卡本身只在对话页签里，用户停在
          结果页签时同样必须看到「有选择请求待处理」。 */}
      <PendingChoiceNotice
        entries={store.chatEntries}
        onJump={(entryId) => {
          // F-43 fix1：先切回对话页签（列表在「运行结果」页签/Plan Repair 下未挂载，
          // ref=null 时同步 scrollToEntry 是静默 no-op），滚动交给挂载后的 effect。
          setPendingJumpEntryId(entryId);
          setActivePanel("chat");
        }}
      />

      <header className="grid min-h-16 min-w-0 shrink-0 gap-2 overflow-hidden border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-3 md:grid-cols-[minmax(0,1fr)_auto]">
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <span className="text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
              {displayCodingStage(store.stage ?? "prepare_context")}
            </span>
            <span className="text-xs text-[var(--aria-ink-muted)]">
              {store.baseBranch ?? "HEAD"} {"->"} {store.branchName ?? "未创建分支"}
            </span>
          </div>
          <div className="mt-1 truncate font-mono text-xs text-[var(--aria-ink-muted)]">
            {store.worktreePath ?? "worktree pending"}
          </div>
        </div>
        <div className="flex min-w-0 items-center justify-end gap-2">
          {pendingGate?.kind === "blocked" ? null : (
            <ActionButtons
              api={api}
              stage={store.stage}
              status={store.status}
              startupErrorCode={startupErrorCode}
              groupFinalReadinessStatus={store.groupFinalReadiness?.status}
              groupFinalReadinessDiagnostics={store.groupFinalReadiness?.diagnostics}
              automationManaged={automationManaged}
              automationTerminated={terminationTerminatedAt !== null}
            />
          )}
          {/* 批 2 T4（REQ-WIGA-09）：托管且未终止——「终止整个自动化」统一
              入口（门阻塞时同样可达）；终止后由下方横幅呈现 terminated_at。 */}
          {automationManaged && terminationTerminatedAt === null ? (
            <AutomationTerminationEntryButton onOpen={openAutomationTermination} />
          ) : null}
        </div>
      </header>
      {/* 批 2 T4：终止态呈现（terminated_at）＋统一二次确认面（任意入口共用）。 */}
      {terminationTerminatedAt !== null ? (
        <AutomationTerminatedBanner
          terminatedAt={terminationTerminatedAt}
          outcome={store.planTermination.terminated?.outcome ?? null}
        />
      ) : null}
      <AutomationTerminationConfirmSurface />
      {/* P3 WIGA Task 4（§4.2 / D5）：readiness Complete 且 waiting_for_human/
          final_confirm 的等待态——明确「编码执行完成，待最终确认」，人手
          点击「确认完成」（驾驶舱或本页按钮）后才允许完成横幅；unit 全
          完成不提前冒称整组已交付。 */}
      {store.attemptScope === "work_item_group" &&
      store.stage === "final_confirm" &&
      store.status === "waiting_for_human" ? (
        <div
          role="status"
          aria-live="polite"
          data-testid="coding-final-confirm-waiting"
          className="flex shrink-0 items-center gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-2 text-xs text-[var(--aria-ink)]"
        >
          <Clock className="h-4 w-4 shrink-0" />
          <span className="font-semibold">编码执行完成，待最终确认</span>
          <span className="text-[var(--aria-ink-muted)]">
            人工点击「确认完成」后才会最终确认
          </span>
        </div>
      ) : null}
      {store.attemptScope === "work_item_group" &&
      store.status === "completed" ? (
        <div
          role="status"
          aria-live="polite"
          className="flex shrink-0 items-center gap-2 border-b border-[var(--aria-success)] bg-[var(--aria-success-soft)] px-4 py-2 text-xs text-[var(--aria-success)]"
        >
          <CheckCircle2 className="h-4 w-4 shrink-0" />
          <span className="font-semibold">组级 Coding Workspace 已完成</span>
          <span className="text-[var(--aria-ink-muted)]">
            {store.units.length > 0
              ? `${store.units.length} 个 Work Item 已完成并确认`
              : "所有 Work Item 已完成并确认"}
          </span>
          {store.headCommit ? (
            <span className="truncate font-mono text-[var(--aria-ink-muted)]">
              最终提交 {store.headCommit.slice(0, 12)}
            </span>
          ) : null}
        </div>
      ) : null}
      {store.attemptScope === "work_item_group" &&
      (store.units.length > 0 || store.workItemGroupId) ? (
        <CodingWorkspaceGroupProgress
          planId={store.workItemGroupId}
          currentWorkItemId={store.currentWorkItemId}
          units={store.units}
          projectId={address.projectId}
          issueId={address.issueId}
          selectedAttemptId={address.attemptId}
          refreshKey={`${store.status ?? "-"}-${store.stage ?? "-"}-${store.units.length}`}
          onDeleteAllCodingWorkspaces={() => void handleDeleteAllCodingWorkspaces()}
          groupDeleteBusy={groupDeleteBusy}
          onOpenPlanOverview={openGroupPlanOverview}
        />
      ) : null}
      <CodingDashboard projectId={address.projectId} issueId={address.issueId} />

      <main className="grid min-h-0 min-w-0 flex-1 grid-cols-1 overflow-hidden md:grid-cols-[16rem_minmax(0,1fr)]">
        <CodingTimeline
          nodes={store.timelineNodes}
          activeNodeId={store.activeNodeId}
          selectedNodeId={store.selectedNodeId}
          onSelectNode={handleSelectTimelineNode}
          planRepair={store.activePlanRepair}
        />
        <section className="grid min-h-0 min-w-0 grid-rows-[auto_minmax(0,1fr)] overflow-hidden bg-[var(--aria-panel)]">
          {store.activePlanRepair ? (
            <div className="row-span-2 min-h-0 min-w-0 overflow-hidden">
              <PlanRepairCenter
                state={store.activePlanRepair}
                onAction={handlePlanRepairAction}
                actionsDisabled={
                  planRepairApi.connectionStatus !== "connected" ||
                  activeRepairActionPending
                }
                actionPending={activeRepairActionPending}
                actionStatus={planRepairActionStatus}
                linkedAmendmentTargets={linkedAmendmentTargets}
                linkedAmendmentStatus={linkedAmendmentStatus}
                linkedAmendmentSnapshot={linkedAmendmentSnapshot}
                linkedAmendmentError={linkedAmendmentError}
                onStartLinkedAmendment={
                  planRepairApi.startLinkedWorkspaceAmendment
                }
              />
            </div>
          ) : (
            <CodingPanelTabs activePanel={activePanel} onSelectPanel={setActivePanel} />
          )}
          {store.activePlanRepair ? null : activePanel === "results" ? (
            <CodingArtifactTabs
              address={address}
              activeTab={activeTab}
              onRetryPush={api.retryPush}
              className="min-h-0"
            />
          ) : (
            <div
              className={[
                "grid min-h-0 min-w-0 overflow-hidden",
                store.stage === "prepare_context" && store.workItemExecutionPlan
                  ? "grid-rows-[auto_auto_minmax(0,1fr)_auto_auto]"
                  : "grid-rows-[auto_minmax(0,1fr)_auto_auto]",
              ].join(" ")}
            >
              {store.stage === "prepare_context" && store.workItemExecutionPlan ? (
                <PrepareExecutionPlanPanel
                  address={address}
                  plan={store.workItemExecutionPlan}
                  requireConfirm={store.requireExecutionPlanConfirm}
                  onError={setPlanError}
                />
              ) : null}
              <div
                data-testid="coding-chat-toolbar"
                className="flex h-11 min-w-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] bg-white px-3"
              >
                <div className="min-w-0 truncate text-xs text-[var(--aria-ink-muted)]">
                  <span className="font-semibold text-[var(--aria-ink)]">运行对话</span>
                  <span className="ml-2">{store.chatEntries.length} 条消息</span>
                </div>
                <div className="flex min-w-0 shrink-0 items-center gap-1.5">
                  <button
                    type="button"
                    aria-label="Provider 设置"
                    onClick={() => setActiveDrawer("providers")}
                    className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)]"
                  >
                    <Settings2 className="h-3.5 w-3.5" />
                    <span>Provider 设置</span>
                    <span
                      aria-hidden="true"
                      className="hidden max-w-[14rem] truncate font-mono text-[11px] font-normal text-[var(--aria-ink-muted)] lg:inline"
                    >
                      {providerSummary}
                    </span>
                  </button>
                  <button
                    type="button"
                    aria-label="角色运行历史"
                    onClick={() => setActiveDrawer("runs")}
                    className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)]"
                  >
                    <History className="h-3.5 w-3.5" />
                    <span>角色运行历史</span>
                    <span
                      aria-hidden="true"
                      className="hidden max-w-[8rem] truncate text-[11px] font-normal text-[var(--aria-ink-muted)] md:inline"
                    >
                      {roleRunSummary}
                    </span>
                  </button>
                </div>
              </div>
              <div
                data-testid="coding-chat-entry-list"
                className="min-h-0 min-w-0 overflow-hidden"
              >
                <ChatEntryList
                  ref={chatListRef}
                  entries={store.chatEntries}
                  onPermissionResponse={handlePermissionResponse}
                  onChoiceResponse={handleChoiceResponse}
                  className="h-full"
                />
              </div>
              <GatePanel
                gate={pendingGate}
                onRespond={api.respondGate}
                onConfirmStage={api.confirmStageGate}
                onAbort={automationManaged ? openAutomationTermination : api.abortAttempt}
                automationManaged={automationManaged}
                onOpenTermination={automationManaged ? openAutomationTermination : undefined}
                verificationTriage={verificationTriage}
                onEnterVerificationTriage={
                  pendingGateEligibleForTriage && !triageActionBusy
                    ? handleEnterVerificationTriage
                    : undefined
                }
                commandEvidence={selectedEvidence}
                onRerunPlannedCommand={
                  !triageActionBusy ? handleRerunPlannedCommand : undefined
                }
              />
              {triageSurfaceError ? (
                <div
                  data-testid="coding-verification-surface-error"
                  className="border-t border-amber-200 bg-amber-50 px-3 py-1 text-xs font-semibold text-[var(--aria-danger)]"
                >
                  {triageSurfaceError}
                </div>
              ) : null}
              {policySurfaceVisible ? (
                <div
                  data-testid="coding-policy-surface"
                  className="border-t border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900"
                >
                  <div className="font-semibold">受限政策读取／重新授权</div>
                  <div className="mt-1 flex flex-wrap items-center gap-1.5">
                    <button
                      type="button"
                      disabled={policyBusy}
                      onClick={handleReadPolicy}
                      className="inline-flex h-7 items-center rounded-md border border-amber-300 bg-white px-2 font-semibold hover:bg-amber-100 disabled:opacity-60"
                    >
                      读取政策
                    </button>
                    <select
                      aria-label="重新授权角色"
                      value={policyRole}
                      onChange={(event) =>
                        setPolicyRole(event.target.value === "reviewer" ? "reviewer" : "coder")
                      }
                      className="h-7 rounded-md border border-amber-300 bg-white px-1"
                    >
                      <option value="coder">coder</option>
                      <option value="reviewer">reviewer</option>
                    </select>
                    <button
                      type="button"
                      disabled={policyBusy || !policyText}
                      onClick={handlePolicyReauthorization}
                      className="inline-flex h-7 items-center rounded-md border border-amber-300 bg-white px-2 font-semibold hover:bg-amber-100 disabled:opacity-60"
                    >
                      重新授权
                    </button>
                  </div>
                  {policyText ? (
                    <div data-testid="coding-policy-text" className="mt-1 grid gap-0.5 font-mono">
                      <div>policy_id：{policyText.policy_id}</div>
                      <div>revision：{policyText.policy_revision}</div>
                      <div>digest：{policyText.policy_digest}</div>
                      <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded border border-amber-200 bg-white p-1">
                        {policyText.text}
                      </pre>
                    </div>
                  ) : null}
                  {policyReauthStatus ? (
                    <div className="mt-1 font-semibold">{policyReauthStatus}</div>
                  ) : null}
                  {policyError ? (
                    <div className="mt-1 font-semibold text-[var(--aria-danger)]">
                      {policyError}
                    </div>
                  ) : null}
                </div>
              ) : null}
              <CodingComposer
                api={api}
                stage={store.stage}
                status={store.status}
                statusText={
                  store.protocolError
                    ? `${store.protocolError.code}: ${store.protocolError.message}`
                    : pendingGate?.title ?? "Coding Workspace"
                }
                pendingGate={pendingGate}
                startupErrorCode={startupErrorCode}
                groupFinalReadinessStatus={store.groupFinalReadiness?.status}
                groupFinalReadinessDiagnostics={store.groupFinalReadiness?.diagnostics}
                automationManaged={automationManaged}
              />
            </div>
          )}
        </section>
      </main>

      {activeDrawer ? (
        <div className="fixed inset-0 z-40 flex justify-end bg-black/35">
          <button
            type="button"
            aria-label="关闭辅助面板"
            className="absolute inset-0 cursor-default"
            onClick={() => setActiveDrawer(null)}
          />
          <aside
            ref={drawerPanelRef}
            role="dialog"
            aria-modal="true"
            aria-label={activeDrawer === "providers" ? "Provider 设置" : activeDrawer === "runs" ? "角色运行历史" : "操作审计"}
            tabIndex={-1}
            className="relative grid h-full w-full max-w-[42rem] grid-rows-[auto_minmax(0,1fr)] overflow-hidden border-l border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl outline-none"
          >
            <div className="flex h-12 min-w-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4">
              <div className="min-w-0">
                <div className="truncate text-sm font-semibold text-[var(--aria-ink)]">
                  {activeDrawer === "providers" ? "Provider 设置" : activeDrawer === "runs" ? "角色运行历史" : "操作审计"}
                </div>
                <div className="truncate text-xs text-[var(--aria-ink-muted)]">
                  {activeDrawer === "providers" ? providerSummary : activeDrawer === "runs" ? roleRunSummary : "本地浏览器与会话快照"}
                </div>
              </div>
              <button
                type="button"
                aria-label="关闭辅助面板"
                onClick={() => setActiveDrawer(null)}
                className="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors duration-200 motion-reduce:transition-none hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="min-h-0 overflow-auto overscroll-contain p-4">
              {activeDrawer === "providers" ? (
                <CodingProviderConfigPanel
                  snapshot={store.roleProviderConfigSnapshot}
                  attemptScope={store.attemptScope}
                  lockedRole={lockedProviderRole(store.stage, store.status, store.pendingGates)}
                  configLocked={
                    store.status !== "created" || store.stage !== "prepare_context"
                  }
                  maxAutoRework={store.maxAutoRework}
                  onSelect={api.sendProviderSelect}
                  onPermissionModeSelect={api.sendPermissionModeSelect}
                  onMaxAutoReworkSelect={api.sendMaxAutoReworkSelect}
                />
              ) : activeDrawer === "runs" ? (
                <RoleRunHistoryPanel
                  roleRuns={store.roleRuns}
                  timelineNodes={store.timelineNodes}
                  selectedNodeId={store.selectedNodeId}
                  onSelectNode={handleSelectTimelineNode}
                />
              ) : (
                <OperationAuditView
                  rows={auditRows}
                  target={auditTarget}
                  onTargetChange={setAuditTarget}
                />
              )}
            </div>
          </aside>
        </div>
      ) : null}

      {/* F-B：「查看全部 WI」在本页弹窗内承载总览——不跨页导航，当前 attempt
          的 WS/store 与门状态原地保留；弹窗关闭即卸载（轮询随之停止）。 */}
      {planOverviewOpen && groupPlanId ? (
        <CodingPlanOverviewModal
          projectId={address.projectId}
          issueId={address.issueId}
          planId={groupPlanId}
          onOpenAttempt={handleOpenOverviewAttempt}
          onClose={() => setPlanOverviewOpen(false)}
        />
      ) : null}

      <div
        data-testid="coding-status-bar"
        className="flex h-8 shrink-0 items-center justify-between gap-3 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs text-[var(--aria-ink-muted)]"
      >
        <span>{displayCodingStage(store.stage ?? "prepare_context")}</span>
        <span className={pageError ? "text-[var(--aria-danger)]" : undefined}>
          {pageError ?? store.connectionStatus}
        </span>
        <span>Coder 修复次数 {store.reworkCount}/{store.maxAutoRework}</span>
      </div>
    </div>
  );

  function handlePermissionResponse(entry: ChatEntry, approved: boolean) {
    const requestId = requestIdFromEntry(entry);
    if (!requestId) return;
    api.respondPermission(requestId, approved);
  }

  function handleChoiceResponse(entry: ChatEntry, response: ChoiceResponsePayload) {
    const requestId = requestIdFromEntry(entry);
    if (!requestId) return;
    void submitChoiceViaRest(requestId, entry, response);
  }

  // P2-e：expected_run_id 取自 entry.metadata（快照路径，pendingChoiceEntry
  // 已写入 handler 按活跃 run 注入的化身）；实时帧路径 metadata 不带化身，
  // 回落取最新 REST 快照。两处皆无 → 无唯一活跃 run（X1）：不发请求、卡片
  // 只显示人话提示，不提供「恢复运行」按钮。
  async function submitChoiceViaRest(
    requestId: string,
    entry: ChatEntry,
    response: ChoiceResponsePayload,
  ) {
    const expectedRunId = await resolveChoiceExpectedRunId(requestId, entry);
    if (!expectedRunId) {
      markChoiceEntry(requestId, (metadata) => ({
        ...metadata,
        choice_submit_error: "AI 当前没有在运行，这个问题暂时无法回答",
      }));
      return;
    }
    respondChoice(requestId, expectedRunId, response);
  }

  async function resolveChoiceExpectedRunId(requestId: string, entry: ChatEntry) {
    const fromStore = choiceMetadataString(choiceEntryFromState(requestId), "expected_run_id");
    if (fromStore) return fromStore;
    const fromEntry = choiceMetadataString(entry, "expected_run_id");
    if (fromEntry) return fromEntry;
    try {
      const snapshot = await getCodingAttemptSnapshot(address);
      const gate = snapshot.pending_choices.find((choice) => choice.choice_id === requestId);
      return gate?.expected_run_id ?? null;
    } catch {
      return null;
    }
  }

  // REST 命令谱系回写卡片：delivered → 已选择（含逐题 answers）；expired →
  // 失效横幅；error → errorMessage 保留卡片可重试（F-9/F-10）。
  function handleChoiceCommandState(
    choiceId: string,
    state: ChoiceRespondCommandState,
    request: ChoiceResponseRequest,
  ) {
    if (state.status === "delivered") {
      const freeTexts = request.answers
        .map((answer) => answer.free_text)
        .filter((text): text is string => Boolean(text));
      markChoiceEntry(
        choiceId,
        (metadata) => ({
          ...metadata,
          response: {
            selected_option_ids: request.answers.flatMap((answer) => answer.selected_option_ids),
            free_text: freeTexts.length > 0 ? freeTexts.join("\n\n") : null,
            answers: request.answers,
          },
        }),
        true,
      );
      return;
    }
    if (state.status === "expired") {
      markChoiceEntry(
        choiceId,
        (metadata) => ({ ...metadata, choice_expired: true }),
        true,
      );
      return;
    }
    if (state.status === "error") {
      markChoiceEntry(choiceId, (metadata) => ({
        ...metadata,
        choice_submit_error: state.errorMessage ?? "应答请求失败",
      }));
    }
  }
}

function choiceEntryFromState(requestId: string) {
  return (
    useCodingWorkspaceStore
      .getState()
      .chatEntries.find(
        (candidate) =>
          candidate.type === "choice_request" && candidate.metadata?.request_id === requestId,
      ) ?? null
  );
}

function choiceMetadataString(entry: ChatEntry | null, key: string) {
  const metadata = entry?.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.[key] === "string" ? (metadata[key] as string) : null;
}

function markChoiceEntry(
  requestId: string,
  merge: (metadata: Record<string, unknown>) => Record<string, unknown>,
  resolved = false,
) {
  const store = useCodingWorkspaceStore.getState();
  const entry = choiceEntryFromState(requestId);
  if (!entry) return;
  store.appendChatEntry({
    ...entry,
    resolved,
    metadata: merge({ ...(entry.metadata as Record<string, unknown>) }),
  });
}

function repairScopeMatches(
  scope: RepairActionScope | null,
  repair: PlanRepairSessionState | null,
) {
  return Boolean(
    scope &&
      repair &&
      scope.childSessionId === repair.childSessionId &&
      scope.amendmentId === repair.amendment?.id &&
      scope.repairGenerationKey === planRepairActionGenerationKey(repair),
  );
}

function protocolErrorMatchesAction(
  action: PendingRepairAction["action"],
  code: string,
) {
  if (action === "confirm") return code === "PLAN_AMENDMENT_CONFIRMATION_FAILED";
  if (action === "cancel") return code === "PLAN_AMENDMENT_CANCEL_FAILED";
  return true;
}

function normalizedRefs(refs: string[] | undefined): string[] {
  return [...new Set((refs ?? []).map((ref) => ref.trim()).filter(Boolean))].sort();
}

function displayCodingStage(stage: string) {
  const labels: Record<string, string> = {
    prepare_context: "准备上下文",
    worktree_prepare: "准备 Worktree",
    coding: "Coder",
    code_review: "Code Reviewer",
    review_request: "准备 PR",
    internal_pr_review: "GroupFinalReview",
    final_confirm: "人工组最终确认",
  };
  return labels[stage] ?? stage;
}

// H5：整组删除一次用户动作生成一次（同动作失败重试复用，服务端按 command_id
// 幂等回执；与 workbench 的 plan_start_ui_ 同一约定）。
function newPlanGroupDeleteCommandId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `plan_delete_ui_${crypto.randomUUID()}`;
  }
  return `plan_delete_ui_${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}
