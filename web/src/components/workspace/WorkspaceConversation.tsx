import { Check, Play, RotateCcw, Send } from "lucide-react";
import { useState, type FormEvent } from "react";
import type { WorkspaceMessage } from "../../api/types";

export function WorkspaceConversation({
  messages,
  onMessage,
  onRunNext,
  onConfirm,
  onRequestChange,
}: {
  messages: WorkspaceMessage[];
  onMessage: (content: string) => void | Promise<void>;
  onRunNext: () => void | Promise<void>;
  onConfirm: () => void | Promise<void>;
  onRequestChange: () => void | Promise<void>;
}) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const content = draft.trim();
    if (!content || busy) {
      return;
    }

    setBusy(true);
    setError(null);
    try {
      await onMessage(content);
      setDraft("");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "发送失败");
    } finally {
      setBusy(false);
    }
  }

  async function handleAction(action: () => void | Promise<void>) {
    if (busy) {
      return;
    }

    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "操作失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      role="region"
      aria-label="Provider 对话"
      className="grid min-h-0 grid-rows-[minmax(0,1fr)_auto] border-r border-[var(--aria-line)]"
    >
      <div className="min-h-0 space-y-2 overflow-auto p-3">
        {messages.length === 0 ? (
          <p className="text-sm text-[var(--aria-ink-muted)]">暂无对话</p>
        ) : (
          messages.map((message, index) => (
            <article
              key={`${message.created_at}:${message.role}:${index}`}
              className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
            >
              <div className="mb-1 flex flex-wrap justify-between gap-2 font-mono text-[11px] text-[var(--aria-ink-muted)]">
                <span>{message.role}</span>
                <span>{message.created_at}</span>
              </div>
              <p className="whitespace-pre-wrap text-sm text-[var(--aria-ink)]">{message.content}</p>
            </article>
          ))
        )}
      </div>
      <form
        onSubmit={handleSubmit}
        className="space-y-3 border-t border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
      >
        <label className="block text-sm font-semibold text-[var(--aria-ink)]">
          补充指令
          <textarea
            value={draft}
            onChange={(event) => {
              setDraft(event.target.value);
              setError(null);
            }}
            className="mt-1 block min-h-20 w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
          />
        </label>
        {error ? (
          <p role="alert" className="text-sm font-semibold text-[var(--aria-danger)]">
            {error}
          </p>
        ) : null}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void handleAction(onRunNext)}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
            >
              <Play className="mr-1 h-4 w-4" />
              下一步
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void handleAction(onConfirm)}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
            >
              <Check className="mr-1 h-4 w-4" />
              确认
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void handleAction(onRequestChange)}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
            >
              <RotateCcw className="mr-1 h-4 w-4" />
              要求修改
            </button>
          </div>
          <button
            type="submit"
            disabled={busy}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:opacity-60"
          >
            <Send className="mr-1 h-4 w-4" />
            发送
          </button>
        </div>
      </form>
    </section>
  );
}
