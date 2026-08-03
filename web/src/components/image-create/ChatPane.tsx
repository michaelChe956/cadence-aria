import { useEffect, useRef, useState, type FormEvent } from "react";
import type { ImageChatEntry } from "../../state/image-create-entries";
import { useImageCreateStore } from "../../state/image-create-store";

export function ChatPane() {
  const entries = useImageCreateStore((state) => state.entries);
  const currentSession = useImageCreateStore((state) => state.currentSession);
  const connectionStatus = useImageCreateStore((state) => state.connectionStatus);
  const isBusy = useImageCreateStore((state) => state.isBusy);
  const sendMessage = useImageCreateStore((state) => state.sendMessage);
  const [message, setMessage] = useState("");
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [entries.length, isBusy]);

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = message.trim();
    if (!trimmed || isBusy) {
      return;
    }
    sendMessage(trimmed);
    setMessage("");
  }

  return (
    <section
      aria-labelledby="image-create-chat-heading"
      className="flex min-h-[32rem] flex-col overflow-hidden rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-sm"
    >
      <div className="flex items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
        <div>
          <h2 id="image-create-chat-heading" className="text-base font-semibold">
            创作对话
          </h2>
          <p className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">
            {currentSession
              ? `${currentSession.session.provider_name} · ${connectionStatusLabel(connectionStatus)}`
              : "选择会话后与创作 Agent 对话"}
          </p>
        </div>
        {isBusy ? (
          <span className="rounded-full bg-[var(--aria-primary-soft)] px-2.5 py-1 text-xs font-semibold text-[var(--aria-ink)]">
            正在处理
          </span>
        ) : null}
      </div>
      <div ref={listRef} className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
        {entries.length === 0 ? (
          <div className="flex min-h-64 items-center justify-center text-center text-sm text-[var(--aria-ink-muted)]">
            {currentSession
              ? "发送需求，让 Agent 帮你完善图片提示词。"
              : "选择或创建会话以开始图片创作。"}
          </div>
        ) : (
          entries.map((entry) => <ChatEntryView key={entry.id} entry={entry} />)
        )}
        {isBusy ? (
          <div
            role="status"
            className="rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]"
          >
            正在处理，请稍候…
          </div>
        ) : null}
      </div>
      <form
        data-testid="image-create-chat-form"
        onSubmit={handleSubmit}
        className="border-t border-[var(--aria-line)] p-3"
      >
        <textarea
          aria-label="创作消息"
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          disabled={isBusy || !currentSession}
          rows={3}
          placeholder={isBusy ? "Agent 正在处理上一条消息…" : "描述图片目标或提出修改意见"}
          className="block min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm text-[var(--aria-ink)] transition-colors placeholder:text-[var(--aria-ink-muted)] hover:border-[var(--aria-line-strong)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:opacity-70"
        />
        <div className="mt-2 flex items-center justify-between gap-3">
          <span className="text-xs text-[var(--aria-ink-muted)]">
            {isBusy ? "忙碌时输入已锁定" : "发送后等待 Agent 返回 suggested_prompt"}
          </span>
          <button
            type="submit"
            disabled={isBusy || !currentSession || !message.trim()}
            className="rounded-md bg-[var(--aria-primary)] px-4 py-2 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
          >
            发送
          </button>
        </div>
      </form>
    </section>
  );
}

function ChatEntryView({ entry }: { entry: ImageChatEntry }) {
  switch (entry.type) {
    case "user_message":
      return (
        <article className="ml-auto max-w-[85%] rounded-xl bg-[var(--aria-primary)] px-3 py-2 text-sm leading-6 text-white">
          {entry.content}
        </article>
      );
    case "provider_text":
      return (
        <article className="max-w-[90%] whitespace-pre-wrap rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm leading-6">
          {entry.content}
        </article>
      );
    case "prompt_block":
      return (
        <article className="rounded-xl border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-3 py-3">
          <div className="text-xs font-semibold uppercase tracking-wide text-[var(--aria-ink)]">
            Suggested prompt{entry.version ? ` · v${entry.version}` : ""}
          </div>
          <p className="mt-1 whitespace-pre-wrap text-sm leading-6 text-[var(--aria-ink)]">
            {entry.content}
          </p>
        </article>
      );
    case "generation_image":
      return (
        <article className="overflow-hidden rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)]">
          <img
            src={`data:${entry.mediaType};base64,${entry.base64}`}
            alt={entry.prompt || "生成图片"}
            className="max-h-[32rem] w-full object-contain"
          />
          <p className="border-t border-[var(--aria-line)] px-3 py-2 text-xs text-[var(--aria-ink-muted)]">
            {entry.prompt}
          </p>
        </article>
      );
    case "generation_error":
      return (
        <div
          role="alert"
          className="rounded-lg border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]"
        >
          生成失败：{entry.content}
        </div>
      );
    case "system_notice":
      return (
        <div className="rounded-lg border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-sm text-[var(--aria-ink)]">
          {entry.content}
        </div>
      );
    case "busy_notice":
      return (
        <div className="rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
          {entry.content}
        </div>
      );
  }
}

function connectionStatusLabel(status: string) {
  switch (status) {
    case "connected":
      return "已连接";
    case "connecting":
      return "连接中";
    case "error":
      return "连接异常";
    default:
      return "未连接";
  }
}
