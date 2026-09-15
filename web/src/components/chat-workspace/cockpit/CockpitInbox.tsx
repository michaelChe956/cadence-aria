import { AlertTriangle, Check, ClipboardList, CircleAlert, RotateCcw } from "lucide-react";
import { useEffect, useMemo, useState, type Ref } from "react";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import {
  canBulkApply,
  type ConfirmTwiceButtonHandle,
} from "../../../state/cockpit-operation-semantics";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
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
  actionableSessionId,
  takeoverButtonRef,
  onBulkConfirm,
}: {
  items: readonly CockpitInboxItem[];
  actions?: CockpitActionFacade;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  actionableSessionId?: string;
  takeoverButtonRef?: Ref<ConfirmTwiceButtonHandle>;
  onBulkConfirm?: (items: readonly CockpitInboxItem[]) => void;
}) {
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(() => new Set());
  const selectableItems = useMemo(
    () =>
      actionableSessionId === undefined
        ? []
        : items.filter((item) => isSelectableGate(item, actionableSessionId)),
    [actionableSessionId, items],
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
        同会话批量确认；当前引擎每会话仅一个开态门，通常只确认 1 项
      </p>
      {canBulkApply("confirm") && onBulkConfirm && selectedItems.length > 0 ? (
        <button
          type="button"
          onClick={handleBulkConfirm}
          className="min-h-11 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          批量确认 {selectedItems.length} 项
        </button>
      ) : null}
      {items.length === 0 ? (
        <p className="text-xs text-[var(--aria-ink-muted)]">暂无待处理项</p>
      ) : (
        items.map((item) => (
          <CockpitInboxRow
            key={item.id}
            item={item}
            actions={actions}
            onTakeover={onTakeover}
            onRetry={onRetry}
            actionable={actionableSessionId === sessionIdForItem(item.id)}
            selectable={actionableSessionId !== undefined && selectableIds.has(item.id)}
            selected={selectedIds.has(item.id)}
            onSelectionChange={() => toggleSelected(item)}
            takeoverButtonRef={
              actionableSessionId === sessionIdForItem(item.id) ? takeoverButtonRef : undefined
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
  actionable,
  selectable,
  selected,
  onSelectionChange,
  takeoverButtonRef,
}: {
  item: CockpitInboxItem;
  actions?: CockpitActionFacade;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  actionable: boolean;
  selectable: boolean;
  selected: boolean;
  onSelectionChange(): void;
  takeoverButtonRef?: Ref<ConfirmTwiceButtonHandle>;
}) {
  const pulse = useCockpitInboxPulse(item.id);
  const Glyph = KIND_GLYPH[item.kind];
  const [takeoverError, setTakeoverError] = useState<string | null>(null);
  const [takeoverDisabled, setTakeoverDisabled] = useState(false);

  const sessionId = sessionIdForItem(item.id);
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
          <label className="flex min-h-11 min-w-11 items-center justify-center">
            <input
              type="checkbox"
              aria-label={`选择 ${item.title}`}
              checked={selected}
              onChange={onSelectionChange}
              className="h-5 w-5 accent-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            />
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
          <GateInboxActions item={item} actions={actions} />
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
}: {
  item: CockpitInboxItem;
  actions: CockpitActionFacade;
}) {
  const [feedback, setFeedback] = useState("");
  const typed = item.gate?.flow_kind === "single_candidate";
  const typedGateAwaitingCommand =
    typed && typeof item.gate?.turn?.command_id !== "string";

  return (
    <div className="mt-2 flex flex-wrap gap-2">
      {typed ? (
        <>
          {typedGateAwaitingCommand ? (
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
      ) : (
        <button
          type="button"
          onClick={() =>
            actions.requestChange({
              description: "采用 findings",
              source: "review_findings",
            })
          }
          className="inline-flex min-h-11 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />
          采纳建议并返修
        </button>
      )}
      <button
        type="button"
        onClick={actions.confirm}
        className="inline-flex min-h-11 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
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
  );
}

function sessionIdForItem(itemId: string): string | null {
  const separator = itemId.indexOf(":");
  if (separator <= 0) {
    return null;
  }
  const sessionId = itemId.slice(0, separator);
  return sessionId === "gate" || sessionId === "hard_error" || sessionId === "stopped"
    ? null
    : sessionId;
}

function isSelectableGate(item: CockpitInboxItem, sessionId: string): boolean {
  return item.kind === "gate" && item.gate?.closed === null && item.id === `${sessionId}:gate:${item.gate.key}`;
}
