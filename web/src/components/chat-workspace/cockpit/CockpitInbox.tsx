import { AlertTriangle, Check, ClipboardList, CircleAlert, RotateCcw } from "lucide-react";
import { useEffect, useMemo, useState, type Ref } from "react";
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
import { ConfirmTwiceButton } from "./ConfirmTwiceButton";
import { GateFeedbackEditor } from "./GateFeedbackEditor";
import { useCockpitInboxPulse } from "../../cockpit/CockpitShell";

const KIND_GLYPH = {
  gate: ClipboardList,
  stopped: CircleAlert,
  hard_error: AlertTriangle,
} as const;

const KIND_CLASS = {
  gate: "border-[var(--aria-gate-open-border)] bg-[var(--aria-gate-open-bg)]",
  stopped: "border-[var(--aria-line-strong)] bg-[var(--aria-panel-subtle)]",
  hard_error: "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)]",
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
  emptyHint,
  artifactVersions = [],
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
  /** 空收件箱时的引导文案；缺省渲染既有「暂无待处理项」。 */
  emptyHint?: string | null;
  artifactVersions?: readonly ArtifactVersionSummary[];
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
  return (
    <section
      data-testid="cockpit-inbox"
      aria-label="待处理收件箱"
      className="flex min-h-0 flex-col gap-2 overflow-auto rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)] p-3"
    >
      <h2 className="text-sm font-semibold text-[var(--aria-ink)]">待处理</h2>
      <p className="text-xs text-[var(--aria-ink-muted)]">
        跨会话批量确认；每会话仅一个开态门，每个所选会话恰好确认一次（REQ-CFC-06）
      </p>
      {canBulkApply("confirm") && onBulkConfirm && selectedItems.length > 0 ? (
        <button
          type="button"
          onClick={handleBulkConfirm}
          className="min-h-11 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          批量确认 {selectedItems.length} 项（{new Set(selectedItems.map((item) => cockpitInboxItemSessionId(item.id))).size} 个会话）
        </button>
      ) : null}
      {items.length === 0 ? (
        <p className="text-xs text-[var(--aria-ink-muted)]">{emptyHint ?? "暂无待处理项"}</p>
      ) : (
        items.map((item) => (
          <CockpitInboxRow
            key={item.id}
            item={item}
            actions={actions}
            onTakeover={onTakeover}
            onRetry={onRetry}
            actionable={actionableSessionId === cockpitInboxItemSessionId(item.id)}
            onRetakeLease={onRetakeLease}
            selectable={selectableIds.has(item.id)}
            selected={selectedIds.has(item.id)}
            onSelectionChange={() => toggleSelected(item)}
            artifactVersions={artifactVersions}
            latestReviewSummary={latestReviewSummary}
            repairReservation={repairReservation}
            takeoverButtonRef={
              actionableSessionId === cockpitInboxItemSessionId(item.id) ? takeoverButtonRef : undefined
            }
          />
        ))
      )}
    </section>
  );
}

function CockpitInboxRow({
  item,
  actions,
  onTakeover,
  onRetry,
  onRetakeLease,
  actionable,
  selectable,
  selected,
  onSelectionChange,
  artifactVersions,
  latestReviewSummary,
  repairReservation,
  takeoverButtonRef,
}: {
  item: CockpitInboxItem;
  actions?: CockpitActionFacade;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  onRetakeLease?: () => void;
  actionable: boolean;
  selectable: boolean;
  selected: boolean;
  onSelectionChange(): void;
  artifactVersions: readonly ArtifactVersionSummary[];
  latestReviewSummary: string | null;
  repairReservation: WorkItemPlanRepairReservation | null;
  takeoverButtonRef?: Ref<ConfirmTwiceButtonHandle>;
}) {
  const pulse = useCockpitInboxPulse(item.id);
  const Glyph = KIND_GLYPH[item.kind];
  const [takeoverError, setTakeoverError] = useState<string | null>(null);
  const [takeoverDisabled, setTakeoverDisabled] = useState(false);

  const sessionId = cockpitInboxItemSessionId(item.id);
  return (
    <article
      data-testid={`cockpit-inbox-item-${item.kind}`}
      data-pulse={pulse}
      className={[
        "flex min-h-11 items-start gap-2 rounded-lg border-2 px-3 py-2",
        "motion-safe:transition-colors motion-safe:duration-200",
        pulse ? "motion-safe:animate-pulse ring-2 ring-[var(--aria-danger)]" : "",
        KIND_CLASS[item.kind],
      ].join(" ")}
    >
      <Glyph className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-[var(--aria-ink)]">{item.title}</p>
        <p className="mt-1 break-words text-xs leading-4 text-[var(--aria-ink-muted)]">
          {item.summary}
        </p>
        {selectable ? (
          <label className="mt-2 flex min-h-11 items-center gap-2 text-xs font-medium text-[var(--aria-ink)]">
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
        {item.kind === "stopped" && onTakeover ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {takeoverDisabled ? (
              <button
                type="button"
                disabled
                className="inline-flex min-h-11 items-center gap-1 rounded-md border border-[var(--aria-line-strong)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:cursor-not-allowed disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
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
        {item.kind === "hard_error" && actions && actionable ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {isStaleDriverLeaseItem(item) && onRetakeLease ? (
              <ConfirmTwiceButton
                label="重新接管"
                confirmLabel="确认重新接管"
                onConfirm={onRetakeLease}
              />
            ) : null}
            <button
              type="button"
              disabled={item.source !== "advance"}
              title={item.source === "advance" ? undefined : "该错误没有可安全重放的命令"}
              onClick={() => onRetry?.(item)}
              className="inline-flex min-h-11 items-center gap-1 rounded-md border border-[var(--aria-line-strong)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink-muted)] disabled:cursor-not-allowed disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />
              重试
            </button>
            <ConfirmTwiceButton
              label="终止"
              confirmLabel="确认终止"
              onConfirm={actions.terminate}
            />
          </div>
        ) : null}
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
  if (actionBlockReason) {
    return (
      <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
        {gateActionBlockCopy(actionBlockReason)}
      </p>
    );
  }

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
        {typed ? (
          <>
            {typedGateNeedsNewCommand ? (
              <p className="w-full text-xs text-[var(--aria-ink-muted)]">
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
        <button
          type="button"
          onClick={actions.confirm}
          className="btn-primary inline-flex min-h-11 items-center gap-1 px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          <Check className="h-3.5 w-3.5" aria-hidden="true" />
          确认
        </button>
        <ConfirmTwiceButton
          label="终止"
          confirmLabel="确认终止"
          onConfirm={actions.terminate}
        />
      </div>
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
    <section aria-label="等待确认的内容" className="rounded-md border border-[var(--aria-line)] bg-white/70 p-3">
      <p className="text-xs font-semibold text-[var(--aria-ink-muted)]">等待确认的内容</p>
      <p className="mt-1 text-sm font-semibold text-[var(--aria-ink)]">{title}</p>
      <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">版本 {version} · {verdictLabel}</p>
      {changes ? <p className="mt-2 text-xs leading-5 text-[var(--aria-ink)]">{changes}</p> : null}
      {review ? <p className="mt-2 text-xs leading-5 text-[var(--aria-ink-muted)]">{review}</p> : null}
    </section>
  );
}


function isSelectableGate(item: CockpitInboxItem): boolean {
  const sessionId = cockpitInboxItemSessionId(item.id);
  // F-20：author_confirm 门（story/design AuthorConfirm）的 confirm 通路是 HTTP
  // 端点，而批量 runner 只发 WS confirm 帧（该阶段被矩阵拒收）——不提供批量勾选。
  return item.kind === "gate" &&
    item.gate?.stage !== "author_confirm" &&
    item.gate?.closed === null &&
    item.gate.action_block_reason === null &&
    sessionId !== null &&
    item.id === `${sessionId}:gate:${item.gate.key}`;
}
