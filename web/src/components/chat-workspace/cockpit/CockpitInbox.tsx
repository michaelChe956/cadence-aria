import { ChevronRight } from "lucide-react";
import { useEffect, useMemo, useState, type Ref } from "react";
import type { ChatEntry, ChoiceResponsePayload } from "../../../state/chat-entries";
import { ChoiceRequestEntry } from "../entries/ChoiceRequestEntry";
import type { ArtifactVersionSummary } from "../../../state/workspace-ws-store-types";
import type { WorkItemPlanRepairReservation } from "../../../state/workspace-ws-store-types";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import {
  canBulkApply,
  type ConfirmTwiceButtonHandle,
} from "../../../state/cockpit-operation-semantics";
import {
  cockpitInboxItemSessionId,
  type CockpitInboxItem,
} from "../../../state/workspace-cockpit-projection";
import type { LeaseDiagnosticsEvent } from "../../../state/lease-diagnostics";
import { ConfirmTwiceButton } from "./ConfirmTwiceButton";
import {
  BTN_PRIMARY_CLASS,
  BTN_SECONDARY_CLASS,
  DISCLOSURE_CHEVRON_CLASS,
  DISCLOSURE_SUMMARY_CLASS,
} from "../gate-visual-tokens";
import {
  codingAttentionLocationItem,
  useCockpitAttentionTargetOpener,
  useCockpitInboxFocusOpener,
  useCockpitInboxPulse,
} from "../../cockpit/CockpitShell";
import type { CodingAttentionLocationItem } from "../../../router";
import {
  inboxFocusTargetFromAttention,
  type WorkbenchInboxFocusTarget,
} from "../../../state/workbench-inbox-focus";
import { C1RecoveryCard, LcBootstrapCard } from "./CockpitInboxWaitingCards";
import { KIND_CLASS, KIND_GLYPH } from "./cockpitInboxKindTokens";
import { GateInboxActions } from "./CockpitInboxGateActions";
import { HardErrorInboxRow } from "./CockpitInboxHardError";
import { LcResumeRecoveryCard } from "./CockpitInboxLcResumeCard";

export function CockpitInbox({
  items,
  actions,
  onTakeover,
  onRetry,
  onRetryFailedSc,
  onRetakeLease,
  actionableSessionId,
  takeoverButtonRef,
  onBulkConfirm,
  onChoiceRespond,
  emptyHint,
  onOpenInfoSession,
  onOpenInfoCoding,
  onOpenAttentionTarget,
  onOpenInboxFocus,
  artifactVersions = [],
  leaseEvents = null,
  latestReviewSummary = null,
  repairReservation = null,
  onStartLcResumeFresh,
  resumeFreshConnectionBlocked = false,
  resumeFreshTerminalBlockReason = null,
}: {
  items: readonly CockpitInboxItem[];
  actions?: CockpitActionFacade;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  /** P2 GAP-E/G（Task 0.1）：失败 SC 现场的人工显式重驱（REST 由页面接线）。 */
  onRetryFailedSc?: (failedNodeId: string) => void;
  onRetakeLease?: () => void;
  actionableSessionId?: string;
  takeoverButtonRef?: Ref<ConfirmTwiceButtonHandle>;
  onBulkConfirm?: (items: readonly CockpitInboxItem[]) => void;
  /** REQ-WIGA-05 Task 11：choice 就地作答回写通道（REST 命令由页面接线）。 */
  onChoiceRespond?: (item: CockpitInboxItem, payload: ChoiceResponsePayload) => void;
  /** 空收件箱时的引导文案；缺省渲染既有「暂无待处理项」。 */
  emptyHint?: string | null;
  /** REQ-WIGA-07 Task 9：info 条目的 plan session 下钻（只读，无批量/危险面）。 */
  onOpenInfoSession?: (sessionId: string) => void;
  onOpenInfoCoding?: (address: {
    projectId: string;
    issueId: string;
    attemptId: string;
  }) => void;
  artifactVersions?: readonly ArtifactVersionSummary[];
  /**
   * P4-e（REQ-UI37-19）：coding_action 卡「去处理」的 A1 target 统一导航
   *（显式 prop 优先，缺省回落 shell context；均未接线不出按钮）。
   */
  onOpenAttentionTarget?: (item: CodingAttentionLocationItem) => void;
  /**
   * UI-3（待处理统一）：coding_action「去处理」打开共享待处理抽屉并聚焦
   *（显式 prop 优先，缺省回落 shell context；attempt 目标优先走此通道，
   * plan/workspace 目标回落 onOpenAttentionTarget 统一导航）。
   */
  onOpenInboxFocus?: (target: WorkbenchInboxFocusTarget) => void;
  /** REQ-DLS-03：STALE 错误条引用的最近租约转移事件（诊断端点拉取，缺省不渲染）。 */
  leaseEvents?: readonly LeaseDiagnosticsEvent[] | null;
  latestReviewSummary?: string | null;
  repairReservation?: WorkItemPlanRepairReservation | null;
  /** D（Task 7.1，设计 §6）：等待卡显式 fresh 出口——页面一次用户动作生成
   * 稳定 operation_id 并经 C6.2 sendLcResumeFresh 发送；返回是否已发送。
   * 缺省（宿主未接线）不出写动作面。 */
  onStartLcResumeFresh?: (recoveryId: string) => boolean;
  /** D（Task 7.1）：断连守卫（页面按 ws connectionStatus 传入，true=禁用）。 */
  resumeFreshConnectionBlocked?: boolean;
  /** D fix-loop r1（P2）：终态守卫——非 null 即禁用并以该文案如实提示
   *（fresh 链后端绕过 engine 终态拒收，前端补位；文案与 F-30 同源）。 */
  resumeFreshTerminalBlockReason?: string | null;
}) {
  // P4-e：显式 prop 优先，缺省回落 shell 注入的统一导航（均未接线为 null）。
  const shellAttentionOpener = useCockpitAttentionTargetOpener();
  const openAttentionTarget = onOpenAttentionTarget ?? shellAttentionOpener;
  // UI-3：共享待处理抽屉聚焦（同上回落规则）。
  const shellInboxFocusOpener = useCockpitInboxFocusOpener();
  const openInboxFocus = onOpenInboxFocus ?? shellInboxFocusOpener;
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(() => new Set());
  const selectableItems = useMemo(
    () => (onBulkConfirm ? items.filter(isSelectableGate) : []),
    [items, onBulkConfirm],
  );
  const selectableIds = useMemo(
    () => new Set(selectableItems.map((item) => item.id)),
    [selectableItems],
  );
  const selectedItems = useMemo(
    () => selectableItems.filter((item) => selectedIds.has(item.id)),
    [selectableItems, selectedIds],
  );

  useEffect(() => {
    setSelectedIds((previous) => {
      const next = new Set(Array.from(previous).filter((id) => selectableIds.has(id)));
      return next.size === previous.size ? previous : next;
    });
  }, [selectableIds]);

  const toggleSelected = (item: CockpitInboxItem) => {
    if (!selectableIds.has(item.id)) {
      return;
    }
    setSelectedIds((previous) => {
      const next = new Set(previous);
      if (next.has(item.id)) {
        next.delete(item.id);
      } else {
        next.add(item.id);
      }
      return next;
    });
  };

  const handleBulkConfirm = () => {
    setSelectedIds(new Set());
    onBulkConfirm?.(selectedItems);
  };
  // F-50 §4.1-1/6（第一批布局减负）：抽屉顶栏已定义「待处理」作用域，收件箱
  // 不再内嵌同名标题；滚动归抽屉 body（唯一滚动容器），本 section 不自带
  // overflow-auto。条目按「连接问题 / 需要人工处理」语义分组，空组不留壳。
  const connectionItems = items.filter((item) => item.kind === "hard_error");
  const humanItems = items.filter(
    (item) => item.kind !== "hard_error" && item.kind !== "info",
  );
  const infoItems = items.filter((item) => item.kind === "info");
  const rowProps = (item: CockpitInboxItem) => ({
    item,
    actions,
    onChoiceRespond,
    onOpenInfoSession,
    onOpenInfoCoding,
    openAttentionTarget,
    openInboxFocus,
    onTakeover,
    onRetry,
    onRetryFailedSc,
    actionable: actionableSessionId === cockpitInboxItemSessionId(item.id),
    onRetakeLease,
    selectable: selectableIds.has(item.id),
    selected: selectedIds.has(item.id),
    onSelectionChange: () => toggleSelected(item),
    artifactVersions,
    leaseEvents,
    latestReviewSummary,
    repairReservation,
    onStartLcResumeFresh,
    resumeFreshConnectionBlocked,
    resumeFreshTerminalBlockReason,
    takeoverButtonRef:
      actionableSessionId === cockpitInboxItemSessionId(item.id) ? takeoverButtonRef : undefined,
  });
  return (
    <section
      data-testid="cockpit-inbox"
      aria-label="待处理收件箱"
      className="flex flex-col gap-2 rounded-xl border border-slate-200 bg-white p-3"
    >
      {/* F-50 裁决 8：批量规则默认一句短句，REQ 编号与完整规则收进折叠详情，
          不再占一整组首屏高度。 */}
      <p data-testid="inbox-bulk-rule" className="text-xs text-slate-500">
        跨会话批量确认 · 每个会话一次
        <details
          data-testid="inbox-bulk-rule-details"
          className="group ml-1 inline-block"
        >
          <summary
            className={`${DISCLOSURE_SUMMARY_CLASS} ml-0.5 inline-flex items-center gap-0.5 text-xs font-medium text-indigo-600`}
          >
            <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
            详情
          </summary>
          <span className="mt-1 block text-xs leading-4">
            每会话仅一个开态门，每个所选会话恰好确认一次（REQ-CFC-06）
          </span>
        </details>
      </p>
      {canBulkApply("confirm") && onBulkConfirm && selectedItems.length > 0 ? (
        <button
          type="button"
          onClick={handleBulkConfirm}
          className={`${BTN_PRIMARY_CLASS} w-full`}
        >
          批量确认 {selectedItems.length} 项（{new Set(selectedItems.map((item) => cockpitInboxItemSessionId(item.id))).size} 个会话）
        </button>
      ) : null}
      {items.length === 0 ? (
        <p className="text-xs text-slate-500">{emptyHint ?? "暂无待处理项"}</p>
      ) : (
        // F-50 视觉 v2 §3：分组标题弱化（uppercase tracking-wider），组间距
        // space-y-4；组内条目 6px 紧排——组间 16px / 组内 6px 拉开层级。
        <div className="space-y-4">
          {connectionItems.length > 0 ? (
            <div className="space-y-1.5">
              <h3 className="text-xs font-medium uppercase tracking-wider text-slate-500">
                连接问题
              </h3>
              {connectionItems.map((item) => (
                <CockpitInboxRow key={item.id} {...rowProps(item)} />
              ))}
            </div>
          ) : null}
          {humanItems.length > 0 ? (
            <div className="space-y-1.5">
              <h3 className="text-xs font-medium uppercase tracking-wider text-slate-500">
                需要人工处理
              </h3>
              {humanItems.map((item) => (
                <CockpitInboxRow key={item.id} {...rowProps(item)} />
              ))}
            </div>
          ) : null}
          {infoItems.length > 0 ? (
            <div className="space-y-1.5">
              <h3 className="text-xs font-medium uppercase tracking-wider text-slate-500">
                进度信息
              </h3>
              {infoItems.map((item) => (
                <CockpitInboxRow key={item.id} {...rowProps(item)} />
              ))}
            </div>
          ) : null}
        </div>
      )}
    </section>
  );
}

/**
 * P4-e（REQ-UI37-19）+ UI-3：coding_action 卡「去处理」——durable 事实投影
 * 与页面内卡同源，收件箱只定位不执行动作。UI-3 起 attempt 目标优先打开
 * **共享待处理抽屉**并聚焦（onOpenInboxFocus，设计 §3 能力 6）；plan/
 * workspace 目标及未接线时回落 `onOpenAttentionTarget` 统一导航（显式
 * prop 优先，context 兜底；均未接线时不出按钮，fail-closed——本组件不
 * import router，避免循环加载）。
 */
function CodingAttentionOpenButton({
  info,
  openTarget,
  openInboxFocus,
}: {
  info: NonNullable<CockpitInboxItem["attentionInfo"]>;
  openTarget: ((item: CodingAttentionLocationItem) => void) | null;
  openInboxFocus: ((target: WorkbenchInboxFocusTarget) => void) | null;
}) {
  const handleClick = () => {
    if (openInboxFocus !== null) {
      const focusTarget = inboxFocusTargetFromAttention(info);
      if (focusTarget !== null) {
        openInboxFocus(focusTarget);
        return;
      }
    }
    openTarget?.(codingAttentionLocationItem(info));
  };
  return (
    <button
      type="button"
      data-testid="cockpit-attention-open"
      onClick={handleClick}
      className="mt-2 min-h-9 rounded-md border border-amber-300 bg-white px-2.5 py-1 text-xs font-semibold text-amber-900 transition-colors duration-200 hover:bg-amber-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500"
    >
      去处理
    </button>
  );
}

function CockpitInboxRow({
  item,
  actions,
  onChoiceRespond,
  onOpenInfoSession,
  onOpenInfoCoding,
  openAttentionTarget,
  openInboxFocus,
  onTakeover,
  onRetry,
  onRetryFailedSc,
  onRetakeLease,
  actionable,
  selectable,
  selected,
  onSelectionChange,
  artifactVersions,
  leaseEvents,
  latestReviewSummary,
  repairReservation,
  onStartLcResumeFresh,
  resumeFreshConnectionBlocked,
  resumeFreshTerminalBlockReason,
  takeoverButtonRef,
}: {
  item: CockpitInboxItem;
  actions?: CockpitActionFacade;
  /** REQ-WIGA-05 Task 11：choice 就地作答回写通道（透传页面 REST 接线）。 */
  onChoiceRespond?: (item: CockpitInboxItem, payload: ChoiceResponsePayload) => void;
  /** REQ-WIGA-07 Task 9：info 条目的 plan session 下钻（只读导航）。 */
  onOpenInfoSession?: (sessionId: string) => void;
  /** REQ-WIGA-07/R5 Task 9：coding FinalConfirm info 的 attempt 地址下钻。 */
  onOpenInfoCoding?: (address: {
    projectId: string;
    issueId: string;
    attemptId: string;
  }) => void;
  /** P4-e（REQ-UI37-19）：coding_action「去处理」导航（见 CockpitInbox 同名 prop）。 */
  openAttentionTarget: ((item: CodingAttentionLocationItem) => void) | null;
  /** UI-3：coding_action「去处理」的共享待处理抽屉聚焦通道（可 null）。 */
  openInboxFocus: ((target: WorkbenchInboxFocusTarget) => void) | null;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  /** P2 GAP-E/G（Task 0.1）：失败 SC 现场的人工显式重驱（透传页面 REST 接线）。 */
  onRetryFailedSc?: (failedNodeId: string) => void;
  onRetakeLease?: () => void;
  actionable: boolean;
  selectable: boolean;
  selected: boolean;
  onSelectionChange(): void;
  artifactVersions: readonly ArtifactVersionSummary[];
  leaseEvents: readonly LeaseDiagnosticsEvent[] | null;
  latestReviewSummary: string | null;
  repairReservation: WorkItemPlanRepairReservation | null;
  /** D（Task 7.1）：恢复等待卡显式 fresh 出口（页面接线，见 CockpitInbox 同名 prop）。 */
  onStartLcResumeFresh?: (recoveryId: string) => boolean;
  resumeFreshConnectionBlocked: boolean;
  /** D fix-loop r1（P2）：终态守卫禁用原因（非 null 即禁用，见同名 prop）。 */
  resumeFreshTerminalBlockReason: string | null;
  takeoverButtonRef?: Ref<ConfirmTwiceButtonHandle>;
}) {
  const pulse = useCockpitInboxPulse(item.id);
  const Glyph = KIND_GLYPH[item.kind];
  const [takeoverError, setTakeoverError] = useState<string | null>(null);
  const [takeoverDisabled, setTakeoverDisabled] = useState(false);

  const sessionId = cockpitInboxItemSessionId(item.id);
  if (item.kind === "hard_error") {
    return (
      <HardErrorInboxRow
        item={item}
        pulse={pulse}
        actions={actions}
        actionable={actionable}
        onRetry={onRetry}
        onRetakeLease={onRetakeLease}
        leaseEvents={leaseEvents}
      />
    );
  }
  return (
    <article
      data-testid={`cockpit-inbox-item-${item.kind}`}
      data-pulse={pulse}
      className={[
        "flex items-start gap-2",
        "motion-safe:transition-colors motion-safe:duration-200",
        pulse ? "motion-safe:animate-pulse ring-2 ring-[var(--aria-danger)]" : "",
        KIND_CLASS[item.kind],
      ].join(" ")}
    >
      <Glyph className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        {/* F-50 视觉 v2 §3：标题行 truncate+min-w-0，副信息换行不挤同行。 */}
        <p className="min-w-0 truncate text-sm font-semibold text-slate-900">{item.title}</p>
        {item.summary ? (
          <p className="mt-1 break-words text-xs leading-4 text-slate-600">
            {item.summary}
          </p>
        ) : null}
        {selectable ? (
          <label className="mt-2 flex min-h-11 items-center gap-2 text-sm font-medium text-slate-700">
            <input
              type="checkbox"
              aria-label={`选择 ${item.title}`}
              checked={selected}
              onChange={onSelectionChange}
              className="h-5 w-5 accent-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            />
            选择此门以批量确认
          </label>
        ) : null}
        {item.inlineError ? (
          <p className="aria-mono mt-1 text-xs text-[var(--aria-danger)]">
            {item.inlineError.code} · {item.inlineError.message}
          </p>
        ) : null}
        {takeoverError ? (
          <p className="aria-mono mt-1 text-xs text-[var(--aria-danger)]">{takeoverError}</p>
        ) : null}
        {item.kind === "info" && item.codingInfo && onOpenInfoCoding ? (
          <button
            type="button"
            onClick={() =>
              onOpenInfoCoding({
                projectId: item.codingInfo?.projectId ?? "",
                issueId: item.codingInfo?.issueId ?? "",
                attemptId: item.codingInfo?.attemptId ?? "",
              })
            }
            className="mt-2 min-h-9 rounded-md border border-emerald-300 bg-white px-2.5 py-1 text-xs font-semibold text-emerald-700 transition-colors duration-200 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-500"
          >
            查看 Coding Workspace
          </button>
        ) : null}
        {item.kind === "info" && item.planInfo && onOpenInfoSession ? (
          <button
            type="button"
            onClick={() => onOpenInfoSession(item.planInfo?.sessionId ?? sessionId ?? "")}
            className="mt-2 min-h-9 rounded-md border border-emerald-300 bg-white px-2.5 py-1 text-xs font-semibold text-emerald-700 transition-colors duration-200 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-500"
          >
            查看 Plan 会话
          </button>
        ) : null}
        {item.kind === "coding_action" &&
        item.attentionInfo &&
        (openAttentionTarget !== null || openInboxFocus !== null) ? (
          <CodingAttentionOpenButton
            info={item.attentionInfo}
            openTarget={openAttentionTarget}
            openInboxFocus={openInboxFocus}
          />
        ) : null}
        {item.kind === "gate" && actions && actionable ? (
          <GateInboxActions
            item={item}
            actions={actions}
            artifactVersions={artifactVersions}
            latestReviewSummary={latestReviewSummary}
            repairReservation={repairReservation}
          />
        ) : null}
        {item.kind === "choice" && actionable ? (
          <ChoiceInboxCard item={item} onChoiceRespond={onChoiceRespond} />
        ) : null}
        {item.kind === "c1_recovery" && item.c1Info ? (
          <C1RecoveryCard item={item} actions={actions} />
        ) : null}
        {item.kind === "lc_bootstrap" && item.bootstrapInfo ? (
          <LcBootstrapCard item={item} actions={actions} />
        ) : null}
        {item.kind === "lc_resume_recovery" && item.recovery ? (
          <LcResumeRecoveryCard
            info={item.recovery}
            actionable={actionable}
            onStartLcResumeFresh={onStartLcResumeFresh}
            connectionBlocked={resumeFreshConnectionBlocked}
            terminalBlockReason={resumeFreshTerminalBlockReason}
          />
        ) : null}
        {item.kind === "sc_failed" && item.scFailure && onRetryFailedSc ? (
          <button
            type="button"
            onClick={() => onRetryFailedSc(item.scFailure?.failedNodeId ?? "")}
            className="mt-2 min-h-9 rounded-md border border-slate-300 bg-white px-2.5 py-1 text-xs font-semibold text-slate-700 transition-colors duration-200 hover:bg-slate-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-500"
          >
            人工重新驱动
          </button>
        ) : null}
        {item.kind === "stopped" && item.automationManaged === true ? (
          // 批 2 P5（REQ-WIGA-08 C1 前端半）：enabled 精确绑定当前 enrollment
          // 的托管 stopped 卡不渲染接管按钮（服务端 P4 已拒
          // workspace_session_takeover_not_allowed，隐藏仅同步呈现）；指引走
          // 原 plan 对话窗口补说明继续，不转手动。视觉与 P2-d 卡片体系同源
          //（琥珀指引底 + 中性弱化卡面）。
          <p
            data-testid="stopped-managed-hint"
            className="mt-2 rounded-md border border-amber-200 bg-amber-50 px-2.5 py-1.5 text-xs leading-4 text-amber-900"
          >
            自动化托管中——在原对话窗口补说明后「继续自动化」，不支持接管转手动
          </p>
        ) : null}
        {item.kind === "stopped" && item.automationManaged !== true && onTakeover ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {takeoverDisabled ? (
              <button
                type="button"
                disabled
                className={`${BTN_SECONDARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
              >
                接管
              </button>
            ) : (
              <ConfirmTwiceButton
                ref={takeoverButtonRef}
                label="接管"
                confirmLabel="确认接管"
                onConfirm={() => {
                  if (sessionId === null) {
                    return;
                  }
                  void onTakeover(sessionId).catch((error: unknown) => {
                    if (
                      typeof error === "object" &&
                      error !== null &&
                      "code" in error &&
                      error.code === "workspace_session_takeover_not_allowed"
                    ) {
                      const details = "details" in error ? error.details : null;
                      const reason =
                        typeof details === "object" &&
                        details !== null &&
                        "reason" in details &&
                        typeof details.reason === "string"
                          ? details.reason
                          : "";
                      setTakeoverError(
                        `workspace_session_takeover_not_allowed${reason ? ` · ${reason}` : ""}`,
                      );
                      setTakeoverDisabled(true);
                      return;
                    }
                    const message = error instanceof Error ? error.message : "未知错误";
                    setTakeoverError(`接管失败：${message}`);
                  });
                }}
              />
            )}
          </div>
        ) : null}
      </div>
    </article>
  );
}

function isSelectableGate(item: CockpitInboxItem): boolean {
  const sessionId = cockpitInboxItemSessionId(item.id);
  // F-20：author_confirm 门（story/design AuthorConfirm）的 confirm 通路是 HTTP
  // 端点，而批量 runner 只发 WS confirm 帧（该阶段被矩阵拒收）——不提供批量勾选。
  // REQ-PCG-01/02：整组确认（HTTP）与 compile recovery（WS recovery 动作）都不是
  // WS confirm 门——只有 kind="human_gate" 提供勾选，未知 kind 一并 fail-closed。
  return item.kind === "gate" &&
    item.gate?.kind === "human_gate" &&
    item.gate?.stage !== "author_confirm" &&
    item.gate?.closed === null &&
    item.gate.action_block_reason === null &&
    sessionId !== null &&
    item.id === `${sessionId}:gate:${item.gate.key}`;
}

/**
 * P0 1.3（REQ-WIGA-05）Task 11：choice 就地卡——复用 ChoiceRequestEntry 的
 * 逐题表单（answers 原样、多题不扁平化）；状态面按命令回执渲染：
 * submitting/resolving=处理中（禁重复分配答案），expired=已失效（不路由到
 * 新 run），409 冲突经条目 inlineError 呈现（同命令可重试）。无
 * expected_run_id 的旧投影只提示刷新后作答，不猜 run。
 */
export function ChoiceInboxCard({
  item,
  onChoiceRespond,
}: {
  item: CockpitInboxItem;
  onChoiceRespond?: (item: CockpitInboxItem, payload: ChoiceResponsePayload) => void;
}) {
  const choice = item.choice;
  if (choice === null) {
    return null;
  }
  const pending = choice.status === "submitting" || choice.status === "resolving";
  const entry: ChatEntry = {
    id: `choice:${choice.choiceId}`,
    type: "choice_request",
    role: "system",
    content: choice.prompt,
    timestamp: item.createdAt ?? "",
    resolved: false,
    metadata: {
      request_id: choice.choiceId,
      prompt: choice.prompt,
      options: [],
      allow_multiple: choice.allowMultiple,
      allow_free_text: choice.allowFreeText,
      questions: choice.questions,
      source: choice.source,
    },
  } as unknown as ChatEntry;

  if (choice.status === "expired") {
    return (
      <div className="mt-2 space-y-1">
        <p className="text-xs font-medium text-[var(--aria-ink-muted)]">已失效</p>
        <p className="text-xs text-slate-500">应答窗口或所属 run 已结束，刷新后按新选择作答</p>
      </div>
    );
  }
  if (choice.expectedRunId === null) {
    return (
      <div className="mt-2 space-y-2">
        <p className="text-xs text-slate-500">旧会话未携带 run 信息，刷新后作答</p>
        <ChoiceRequestEntry embedded entry={entry} submittingOverride />
      </div>
    );
  }
  if (choice.questions.length === 0) {
    return (
      <div className="mt-2 space-y-2">
        <p className="text-xs text-slate-500">题目结构未同步，刷新后作答</p>
        <ChoiceRequestEntry embedded entry={entry} submittingOverride />
      </div>
    );
  }
  return (
    <div className="mt-3 space-y-2">
      {pending ? (
        <p data-testid="choice-card-status" className="text-xs font-medium text-slate-500">
          处理中
        </p>
      ) : null}
      <ChoiceRequestEntry
        embedded
        entry={entry}
        submittingOverride={pending}
        onRespond={(_entry, payload) => onChoiceRespond?.(item, payload)}
      />
    </div>
  );
}
