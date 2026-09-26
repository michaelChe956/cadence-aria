import {
  AlertTriangle,
  Check,
  ChevronRight,
  ClipboardCopy,
  ClipboardList,
  CircleAlert,
  ListChecks,
  Play,
  RotateCcw,
  UserRound,
} from "lucide-react";
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
  gateActionBlockCopy,
  isStaleDriverLeaseItem,
  type CockpitInboxItem,
} from "../../../state/workspace-cockpit-projection";
import {
  GATE_TERMINATE_BUTTON_LABEL,
  GATE_TERMINATE_CONFIRM_LABEL,
} from "../../../state/gate-prompt-copy";
import {
  PROTOCOL_ERROR_DETAILS_LABEL,
  PROTOCOL_ERROR_NO_RETRY_NOTE,
  protocolErrorCopy,
} from "../../../state/protocol-error-copy";
import { LeaseDiagnosticsSummary } from "../LeaseDiagnosticsSummary";
import type { LeaseDiagnosticsEvent } from "../../../state/lease-diagnostics";
import { ConfirmTwiceButton } from "./ConfirmTwiceButton";
import { GateFeedbackEditor } from "./GateFeedbackEditor";
import {
  BTN_GHOST_CLASS,
  BTN_PRIMARY_CLASS,
  BTN_SECONDARY_CLASS,
  DISCLOSURE_CHEVRON_CLASS,
  DISCLOSURE_SUMMARY_CLASS,
  GATE_CARD_CLASS,
  GATE_NESTED_BLOCK_CLASS,
} from "../gate-visual-tokens";
import { useCockpitInboxPulse } from "../../cockpit/CockpitShell";

const KIND_GLYPH = {
  gate: ClipboardList,
  stopped: CircleAlert,
  hard_error: AlertTriangle,
  choice: ListChecks,
} as const;

// F-50 视觉 v2 §3：门禁条目与门卡同一视觉常量（中性底+琥珀左线）；stopped 维持
// 中性弱化——琥珀不再做整卡底色。
const KIND_CLASS = {
  gate: GATE_CARD_CLASS,
  stopped: "rounded-lg border border-slate-200 bg-gray-50 px-3 py-2",
  choice: GATE_CARD_CLASS,
} as const;

export function CockpitInbox({
  items,
  actions,
  onTakeover,
  onRetry,
  onRetakeLease,
  actionableSessionId,
  takeoverButtonRef,
  onBulkConfirm,
  onChoiceRespond,
  emptyHint,
  artifactVersions = [],
  leaseEvents = null,
  latestReviewSummary = null,
  repairReservation = null,
}: {
  items: readonly CockpitInboxItem[];
  actions?: CockpitActionFacade;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  onRetakeLease?: () => void;
  actionableSessionId?: string;
  takeoverButtonRef?: Ref<ConfirmTwiceButtonHandle>;
  onBulkConfirm?: (items: readonly CockpitInboxItem[]) => void;
  /** REQ-WIGA-05 Task 11：choice 就地作答回写通道（REST 命令由页面接线）。 */
  onChoiceRespond?: (item: CockpitInboxItem, payload: ChoiceResponsePayload) => void;
  /** 空收件箱时的引导文案；缺省渲染既有「暂无待处理项」。 */
  emptyHint?: string | null;
  artifactVersions?: readonly ArtifactVersionSummary[];
  /** REQ-DLS-03：STALE 错误条引用的最近租约转移事件（诊断端点拉取，缺省不渲染）。 */
  leaseEvents?: readonly LeaseDiagnosticsEvent[] | null;
  latestReviewSummary?: string | null;
  repairReservation?: WorkItemPlanRepairReservation | null;
}) {
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
  const humanItems = items.filter((item) => item.kind !== "hard_error");
  const rowProps = (item: CockpitInboxItem) => ({
    item,
    actions,
    onChoiceRespond,
    onTakeover,
    onRetry,
    actionable: actionableSessionId === cockpitInboxItemSessionId(item.id),
    onRetakeLease,
    selectable: selectableIds.has(item.id),
    selected: selectedIds.has(item.id),
    onSelectionChange: () => toggleSelected(item),
    artifactVersions,
    leaseEvents,
    latestReviewSummary,
    repairReservation,
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
        </div>
      )}
    </section>
  );
}

function CockpitInboxRow({
  item,
  actions,
  onChoiceRespond,
  onTakeover,
  onRetry,
  onRetakeLease,
  actionable,
  selectable,
  selected,
  onSelectionChange,
  artifactVersions,
  leaseEvents,
  latestReviewSummary,
  repairReservation,
  takeoverButtonRef,
}: {
  item: CockpitInboxItem;
  actions?: CockpitActionFacade;
  /** REQ-WIGA-05 Task 11：choice 就地作答回写通道（透传页面 REST 接线）。 */
  onChoiceRespond?: (item: CockpitInboxItem, payload: ChoiceResponsePayload) => void;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  onRetakeLease?: () => void;
  actionable: boolean;
  selectable: boolean;
  selected: boolean;
  onSelectionChange(): void;
  artifactVersions: readonly ArtifactVersionSummary[];
  leaseEvents: readonly LeaseDiagnosticsEvent[] | null;
  latestReviewSummary: string | null;
  repairReservation: WorkItemPlanRepairReservation | null;
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
        {item.kind === "stopped" && onTakeover ? (
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

/**
 * F-50 §4.1-7/8/10：协议/引擎错误条——紧凑 alert（role=alert），不再复刻门禁卡
 * 高度；原文与长诊断进折叠详情；无安全重放命令（source ≠ advance）的重试不渲染
 * （灰置会被误读为暂时忙），advance 来源保留「重试」。
 */
function HardErrorInboxRow({
  item,
  pulse,
  actions,
  actionable,
  onRetry,
  onRetakeLease,
  leaseEvents,
}: {
  item: CockpitInboxItem;
  pulse: boolean;
  actions?: CockpitActionFacade;
  actionable: boolean;
  onRetry?: (item: CockpitInboxItem) => void;
  onRetakeLease?: () => void;
  /** REQ-DLS-03：STALE 错误条引用的最近租约转移事件（缺省不渲染）。 */
  leaseEvents: readonly LeaseDiagnosticsEvent[] | null;
}) {
  const Glyph = KIND_GLYPH.hard_error;
  return (
    <article
      data-testid="cockpit-inbox-item-hard_error"
      data-pulse={pulse}
      role="alert"
      className={[
        // F-50 视觉 v2 §3：错误条中性底——红只上图标+mono 码（不再红底整条）。
        "rounded-lg border border-slate-200 bg-white px-3 py-2",
        pulse ? "motion-safe:animate-pulse ring-2 ring-[var(--aria-danger)]" : "",
      ].join(" ")}
    >
      <div className="flex items-start gap-2">
        <Glyph className="mt-0.5 h-4 w-4 shrink-0 text-red-600" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-baseline gap-x-2">
            <span className="text-sm font-semibold text-slate-900">{item.title}</span>
            {item.protocolErrorCode ? (
              <span
                data-testid="cockpit-inbox-error-code"
                className="aria-mono text-xs font-normal text-red-600"
              >
                {item.protocolErrorCode}
              </span>
            ) : null}
          </p>
          {/* F-50 裁决 6：已知码配中文正文；没有可负责任的译文时不编（正文缺省，
              原文已在折叠详情）。 */}
          {item.protocolErrorCode && protocolErrorCopy(item.protocolErrorCode).body ? (
            <p className="mt-1 text-xs leading-4 text-slate-600">
              {protocolErrorCopy(item.protocolErrorCode).body}
            </p>
          ) : null}
          {/* REQ-DLS-03 规范句：STALE 错误条引用最近租约转移事件（可定案
              偷窃者身份）；非 STALE 硬错误或诊断缺省（端点失败静默降级）不渲染。 */}
          {isStaleDriverLeaseItem(item) && leaseEvents ? (
            <LeaseDiagnosticsSummary events={leaseEvents} />
          ) : null}
          {item.inlineError ? (
            <p className="aria-mono mt-1 break-words text-xs text-red-600">
              {item.inlineError.code} · {item.inlineError.message}
            </p>
          ) : null}
          <details
            data-testid="cockpit-inbox-error-details"
            className="group mt-1"
          >
            <summary
              className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 text-xs font-medium text-slate-600`}
            >
              <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
              {PROTOCOL_ERROR_DETAILS_LABEL}
            </summary>
            <p className="aria-mono mt-1 break-words text-xs leading-4 text-slate-600">
              {item.summary}
            </p>
            {/* 裁决 5：不可安全重放的原因作为可见文本（title 悬停不可达）。 */}
            {item.source !== "advance" ? (
              <p className="mt-1 text-xs text-slate-600">
                {PROTOCOL_ERROR_NO_RETRY_NOTE}
              </p>
            ) : null}
          </details>
          {actions && actionable ? (
            <div className="mt-2 flex flex-wrap gap-2">
              {isStaleDriverLeaseItem(item) && onRetakeLease ? (
                <ConfirmTwiceButton
                  label="重新接管"
                  confirmLabel="确认重新接管"
                  onConfirm={onRetakeLease}
                />
              ) : null}
              {item.source === "advance" ? (
                <button
                  type="button"
                  onClick={() => onRetry?.(item)}
                  className={BTN_SECONDARY_CLASS}
                >
                  <RotateCcw className="h-4 w-4" aria-hidden="true" />
                  重试推进
                </button>
              ) : null}
              {/* 裁决 4：terminate → abandon_human_gate（门级），命名显式带作用域。 */}
              <ConfirmTwiceButton
                variant="ghost"
                label={GATE_TERMINATE_BUTTON_LABEL}
                confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
                onConfirm={actions.terminate}
              />
            </div>
          ) : null}
        </div>
      </div>
    </article>
  );
}

function GateInboxActions({
  item,
  actions,
  artifactVersions,
  latestReviewSummary,
  repairReservation,
}: {
  item: CockpitInboxItem;
  actions: CockpitActionFacade;
  artifactVersions: readonly ArtifactVersionSummary[];
  latestReviewSummary: string | null;
  repairReservation: WorkItemPlanRepairReservation | null;
}) {
  const [feedback, setFeedback] = useState("");
  const actionBlockReason = item.gate?.action_block_reason ?? null;
  // F-21：终止专属判据（缺省回退通用判据）——plan 会话停在 human_confirm 的
  // context blocker/author 失败/缺相位门允许终止（矩阵+引擎只校验 stage+flow）。
  // F-21：terminate_block_reason 显式 null=终止放行；仅缺省（undefined，旧投影
  // 形态）才回退通用判据——?? 会把 null 吞成回退。
  const terminateBlockReason =
    item.gate !== null && item.gate.terminate_block_reason !== undefined
      ? item.gate.terminate_block_reason
      : actionBlockReason;
  // REQ-PCG-01/02：批次确认与 compile recovery 是 durable node 门（无 typed
  // turn/snapshot），动作面独立于既有 typed/legacy 分流——阻断时只呈现原因，
  // 写动作一律不露出（F-30 终态守卫）。
  const nodeGateKind =
    item.gate?.kind === "batch_confirm" || item.gate?.kind === "compile_recovery"
      ? item.gate.kind
      : null;
  if (nodeGateKind !== null) {
    if (actionBlockReason !== null) {
      return (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
          {gateActionBlockCopy(actionBlockReason)}
        </p>
      );
    }
    return nodeGateKind === "batch_confirm" ? (
      <BatchConfirmActions actions={actions} />
    ) : (
      <CompileRecoveryActions actions={actions} />
    );
  }

  // C2（REQ-HGC-02/plan T2 ④，F-54 §4.2）：phase_mismatch 通用相位提示行删除
  // （与门卡同源裁决）——不渲染误导文案，终止通路照常。
  if (actionBlockReason && actionBlockReason !== "phase_mismatch" && terminateBlockReason) {
    return (
      <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
        {gateActionBlockCopy(actionBlockReason)}
      </p>
    );
  }

  // F-31（v37 复验 #2）：story/design author 门（产物确认阶段）——HTTP confirm
  // 通路，确认语义为定稿；评审可选（review_available=reviewerEnabled）。
  const authorGate = item.gate?.stage === "author_confirm";
  const typed = item.gate?.flow_kind === "single_candidate";
  const typedGateNeedsNewCommand =
    typed &&
    typeof item.gate?.turn?.command_id !== "string" &&
    repairReservation?.owner_session_id === cockpitInboxItemSessionId(item.id) &&
    (repairReservation.state === "reserved" || repairReservation.state === "provider_started");
  const summary = gateSummary(artifactVersions, latestReviewSummary);

  return (
    <div className="mt-3 space-y-3">
      {summary ? <GateSummary {...summary} /> : null}
      <div className="flex flex-wrap gap-2">
        {actionBlockReason !== null && actionBlockReason !== "phase_mismatch" ? (
          // F-21：phase_mismatch 的 plan 门终止专属放行——只露终止，说明行替代
          // 确认/反馈（相位纪律维持）。
          <p className="w-full text-xs text-slate-500">
            {gateActionBlockCopy(actionBlockReason)}，可终止后重新发起
          </p>
        ) : null}
        {typed && actionBlockReason === null ? (
          <>
            {typedGateNeedsNewCommand ? (
              <p className="w-full text-xs text-slate-500">
                未同步门命令，将以新命令提交
              </p>
            ) : null}
            <GateFeedbackEditor
              multiline
              value={feedback}
              onChange={setFeedback}
              onSubmit={actions.feedback}
            />
          </>
        ) : null}
        {/* L1（REQ-RET-02）：legacy 门「采纳建议并返修」（request-change）发送面删除。 */}
        {actionBlockReason === null ? (
          <button
            type="button"
            onClick={actions.confirm}
            className={BTN_PRIMARY_CLASS}
          >
            <Check className="h-4 w-4" aria-hidden="true" />
            {authorGate ? "确认定稿" : "确认"}
          </button>
        ) : null}
        {/* F-31（v37 复验 #2）：author 门与主区门卡动作面对齐——reviewer 启用
            （投影 review_available）追加「确认并评审」（with_review=true，服务端
            接管进入评审轮）；未启用不露出，与主区同源判据。 */}
        {authorGate && item.gate?.review_available && actionBlockReason === null ? (
          <button
            type="button"
            onClick={actions.confirmReview}
            className={BTN_SECONDARY_CLASS}
          >
            确认并评审
          </button>
        ) : null}
        {/* v40 复验 #3：author 门 review 已完成（存在未被修订取代的最新 review
            报告，同主区 latestReviewReport 判据）时补齐第四钮——与主区/产物
            审核面板「采纳 Review 意见」同款：门面 adoptReview 预填修订反馈+
            切回对话视图。不与 review_available（可发起评审）耦合：评审完成后
            即使 reviewer 已关，既有报告仍可采纳。无 review 结果不露出。 */}
        {authorGate && latestReviewSummary !== null && actionBlockReason === null ? (
          <button
            type="button"
            onClick={actions.adoptReview}
            className={BTN_SECONDARY_CLASS}
          >
            <ClipboardCopy className="h-4 w-4" aria-hidden="true" />
            采纳 Review 意见
          </button>
        ) : null}
        <ConfirmTwiceButton
          variant="ghost"
          label={GATE_TERMINATE_BUTTON_LABEL}
          confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
          onConfirm={actions.terminate}
        />
      </div>
    </div>
  );
}

/**
 * REQ-PCG-01：整组 Work Item Draft 确认门动作面——[确认整组]（HTTP confirm 通路，
 * 门面 confirmBatch）+ [终止]（既有 WS abandon 二次确认）。确认语义是整组 Draft，
 * 不重新暴露 legacy 逐段决策，也不提供批量勾选（批量 runner 只发 WS confirm 帧）。
 */
function BatchConfirmActions({ actions }: { actions: CockpitActionFacade }) {
  return (
    <div className="mt-3 flex flex-wrap gap-2">
      <button
        type="button"
        onClick={() => {
          void actions.confirmBatch();
        }}
        className={BTN_PRIMARY_CLASS}
      >
        <Check className="h-4 w-4" aria-hidden="true" />
        确认整组
      </button>
      <ConfirmTwiceButton
        variant="ghost"
        label={GATE_TERMINATE_BUTTON_LABEL}
        confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
        onConfirm={actions.terminate}
      />
    </div>
  );
}

/**
 * REQ-PCG-02：Final Compile recovery 动作面——既有 `WorkItemPlanCompileRecoveryAction`
 * 三动作原样派发（不映射为 confirm/feedback/abandon）；`human_triage` 可携带原因
 * （wire 可选 reason，能力对齐 legacy WorkItemPlanStagedPanel 的动作集合）。
 */
function CompileRecoveryActions({ actions }: { actions: CockpitActionFacade }) {
  const [triageReason, setTriageReason] = useState("");
  // F-50 视觉 v2 §1：recovery 三动作=次操作描边（确认类才上实心主操作）。
  return (
    <div className="mt-3 space-y-2">
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => {
            void actions.recoverCompile("continue");
          }}
          className={BTN_SECONDARY_CLASS}
        >
          <Play className="h-4 w-4" aria-hidden="true" />
          继续
        </button>
        <button
          type="button"
          onClick={() => {
            void actions.recoverCompile("abort_and_rollback");
          }}
          className={BTN_SECONDARY_CLASS}
        >
          <RotateCcw className="h-4 w-4" aria-hidden="true" />
          放弃并回滚
        </button>
        <button
          type="button"
          onClick={() => {
            void actions.recoverCompile("human_triage", triageReason.trim() || undefined);
          }}
          className={BTN_SECONDARY_CLASS}
        >
          <UserRound className="h-4 w-4" aria-hidden="true" />
          转人工
        </button>
      </div>
      <label className="flex min-h-11 items-center gap-2 text-xs font-medium text-slate-700">
        转人工原因（可选）
        <input
          type="text"
          value={triageReason}
          onChange={(event) => setTriageReason(event.target.value)}
          className="min-h-9 flex-1 rounded-md border border-slate-300 bg-gray-50 px-2 text-xs text-slate-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400"
        />
      </label>
    </div>
  );
}

function gateSummary(
  versions: readonly ArtifactVersionSummary[],
  reviewSummary: string | null,
): { title: string; version: number; verdict: string | null; changes: string; review: string | null } | null {
  const current = versions.find((version) => version.is_current) ??
    [...versions].sort((left, right) => right.version - left.version)[0];
  const markdown = current?.markdown?.trim();
  if (!current || !markdown) {
    return null;
  }
  const lines = markdown.split("\n").map((line) => line.trim()).filter(Boolean);
  const title = lines.find((line) => line.startsWith("#"))?.replace(/^#+\s*/u, "") ?? "当前方案";
  const changes = lines
    .filter((line) => /^[-*]\s+/u.test(line))
    .map((line) => line.replace(/^[-*]\s+/u, ""))
    .slice(0, 3)
    .join("；");
  return { title, version: current.version, verdict: current.review_verdict ?? null, changes, review: reviewSummary };
}

function GateSummary({
  title,
  version,
  verdict,
  changes,
  review,
}: {
  title: string;
  version: number;
  verdict: string | null;
  changes: string;
  review: string | null;
}) {
  const verdictLabel = verdict === "pass" ? "审核通过" : verdict === "revise" ? "需要返修" : verdict === "needs_human" ? "需要人工判断" : "待审核";
  return (
    <section aria-label="等待确认的内容" className={GATE_NESTED_BLOCK_CLASS}>
      <p className="text-xs font-semibold text-slate-500">等待确认的内容</p>
      <p className="mt-1 truncate text-sm font-semibold text-slate-900">{title}</p>
      <p className="mt-1 text-xs text-slate-500">版本 {version} · {verdictLabel}</p>
      {changes ? <p className="mt-2 text-xs leading-5 text-slate-700">{changes}</p> : null}
      {review ? <p className="mt-2 text-xs leading-5 text-slate-500">{review}</p> : null}
    </section>
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
function ChoiceInboxCard({
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

