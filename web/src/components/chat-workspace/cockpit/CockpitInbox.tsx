import { AlertTriangle, Check, ClipboardList, CircleAlert, RotateCcw, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
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
}: {
  items: readonly CockpitInboxItem[];
  actions?: CockpitActionFacade;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  actionableSessionId?: string;
}) {
  return (
    <section
      data-testid="cockpit-inbox"
      aria-label="待处理收件箱"
      className="flex min-h-0 flex-col gap-2 overflow-auto rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)] p-3"
    >
      <h2 className="text-sm font-semibold text-[var(--aria-ink)]">待处理</h2>
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
}: {
  item: CockpitInboxItem;
  actions?: CockpitActionFacade;
  onTakeover?: (sessionId: string) => Promise<void>;
  onRetry?: (item: CockpitInboxItem) => void;
  actionable: boolean;
}) {
  const pulse = useCockpitInboxPulse(item.id);
  const Glyph = KIND_GLYPH[item.kind];
  const [pendingTerminate, setPendingTerminate] = useState(false);
  const [pendingTakeover, setPendingTakeover] = useState(false);
  const [takeoverError, setTakeoverError] = useState<string | null>(null);
  const [takeoverDisabled, setTakeoverDisabled] = useState(false);

  useEffect(() => {
    if (!pendingTerminate && !pendingTakeover) {
      return;
    }
    const timer = window.setTimeout(() => {
      setPendingTerminate(false);
      setPendingTakeover(false);
    }, 10_000);
    return () => window.clearTimeout(timer);
  }, [pendingTakeover, pendingTerminate]);

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
            <button
              type="button"
              disabled={takeoverDisabled || sessionId === null}
              title={sessionId === null ? "无法识别会话，不能接管" : undefined}
              onClick={() => {
                if (sessionId === null) {
                  return;
                }
                if (!pendingTakeover) {
                  setPendingTakeover(true);
                  return;
                }
                void onTakeover(sessionId)
                  .then(() => {
                    setPendingTakeover(false);
                  })
                  .catch((error: unknown) => {
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
                    } else {
                      const message = error instanceof Error ? error.message : "未知错误";
                      setTakeoverError(`接管失败：${message}`);
                    }
                    setPendingTakeover(false);
                  });
              }}
              className="inline-flex min-h-11 items-center gap-1 rounded-md border border-[var(--aria-line-strong)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] disabled:cursor-not-allowed disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              <CircleAlert className="h-3.5 w-3.5" aria-hidden="true" />
              {pendingTakeover ? "确认接管" : "接管"}
            </button>
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
            <button
              type="button"
              onClick={() => {
                if (pendingTerminate) {
                  actions.terminate();
                  setPendingTerminate(false);
                  return;
                }
                setPendingTerminate(true);
              }}
              className="inline-flex min-h-11 items-center gap-1 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              <X className="h-3.5 w-3.5" aria-hidden="true" />
              {pendingTerminate ? "确认终止" : "终止"}
            </button>
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
  const [pendingTerminate, setPendingTerminate] = useState(false);
  const [feedbackEditorOpen, setFeedbackEditorOpen] = useState(false);
  const [feedback, setFeedback] = useState("");
  const typed = item.gate?.flow_kind === "single_candidate";
  const typedGateAwaitingCommand =
    typed && typeof item.gate?.turn?.command_id !== "string";

  useEffect(() => {
    if (!pendingTerminate) {
      return;
    }
    const timer = window.setTimeout(() => setPendingTerminate(false), 10_000);
    return () => window.clearTimeout(timer);
  }, [pendingTerminate]);

  return (
    <div className="mt-2 flex flex-wrap gap-2">
      {typed ? (
        <>
          {typedGateAwaitingCommand ? (
            // 无活 turn（重连/刷新后仅剩快照门）：提示态而非禁用态——
            // 引擎允许客户端自生成 command_id 开新回合提交反馈。
            <p className="w-full text-xs text-[var(--aria-ink-muted)]">
              未同步门命令，将以新命令提交
            </p>
          ) : null}
          {feedbackEditorOpen ? (
            <>
              <textarea
                aria-label="门禁反馈"
                value={feedback}
                onChange={(event) => setFeedback(event.target.value)}
                placeholder="请输入反馈内容"
                className="min-h-11 w-full rounded-md border border-[var(--aria-line-strong)] bg-white px-3 py-2 text-xs text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              />
              <button
                type="button"
                disabled={!feedback.trim()}
                onClick={() => actions.feedback(feedback)}
                className="inline-flex min-h-11 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50 disabled:cursor-not-allowed disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              >
                <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />
                提交反馈
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={() => setFeedbackEditorOpen(true)}
              className="inline-flex min-h-11 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />
              编辑反馈
            </button>
          )}
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
        onClick={() => actions.confirm()}
        className="inline-flex min-h-11 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        <Check className="h-3.5 w-3.5" aria-hidden="true" />
        确认
      </button>
      <button
        type="button"
        onClick={() => {
          if (pendingTerminate) {
            actions.terminate();
            setPendingTerminate(false);
            return;
          }
          setPendingTerminate(true);
        }}
        className="inline-flex min-h-11 items-center gap-1 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        <X className="h-3.5 w-3.5" aria-hidden="true" />
        {pendingTerminate ? "确认终止" : "终止"}
      </button>
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
