import {
  Download,
  ImagePlus,
  LoaderCircle,
  MessageSquare,
  Send,
  Sparkles,
  Wand2,
} from "lucide-react";
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
  const [elapsed, setElapsed] = useState(0);
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!isBusy) {
      setElapsed(0);
      return;
    }
    setElapsed(0);
    const id = window.setInterval(() => setElapsed((s) => s + 1), 1000);
    return () => window.clearInterval(id);
  }, [isBusy]);

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
      className="flex min-h-[36rem] flex-col overflow-hidden rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-[0_2px_8px_rgba(15,23,42,0.04),0_12px_32px_rgba(15,23,42,0.07)] transition-all duration-200 hover:shadow-md"
    >
      <div className="flex items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-gradient-to-b from-white to-[var(--aria-panel-muted)] px-5 py-4">
        <div className="flex min-w-0 items-center gap-3">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
            <MessageSquare aria-hidden="true" className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <h2 id="image-create-chat-heading" className="text-base font-semibold">
              创作对话
            </h2>
            <p className="mt-0.5 truncate text-xs text-[var(--aria-ink-muted)]">
              {currentSession
                ? `${currentSession.session.provider_name} · ${connectionStatusLabel(connectionStatus)}`
                : "选择会话后与创作 Agent 对话"}
            </p>
          </div>
        </div>
        {isBusy ? (
          <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--aria-primary)]/30 bg-[var(--aria-primary-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--aria-ink)] shadow-sm">
            <LoaderCircle aria-hidden="true" className="h-3.5 w-3.5 motion-safe:animate-spin" />
            正在处理
          </span>
        ) : null}
      </div>
      <div ref={listRef} className="min-h-0 flex-1 space-y-4 overflow-y-auto bg-[var(--aria-panel-muted)]/60 p-5">
        {entries.length === 0 ? (
          <div className="flex min-h-72 items-center justify-center">
            <div className="max-w-sm text-center">
              <span className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl border border-white bg-gradient-to-br from-[var(--aria-primary-soft)] to-white text-[var(--aria-primary)] shadow-[0_8px_24px_rgba(8,145,178,0.14)]">
                {currentSession ? (
                  <Wand2 aria-hidden="true" className="h-7 w-7" />
                ) : (
                  <ImagePlus aria-hidden="true" className="h-7 w-7" />
                )}
              </span>
              <p className="mt-4 text-sm font-semibold text-[var(--aria-ink)]">
                {currentSession ? "从描述创作目标开始" : "准备开始图片创作"}
              </p>
              <p className="mt-1.5 text-sm leading-6 text-[var(--aria-ink-muted)]">
                {currentSession
                  ? "发送需求，让 Agent 帮你完善图片提示词。"
                  : "选择或创建会话以开始图片创作。"}
              </p>
            </div>
          </div>
        ) : (
          entries.map((entry) => <ChatEntryView key={entry.id} entry={entry} />)
        )}
        {isBusy ? (
          <div
            role="status"
            aria-live="polite"
            className="rounded-xl border border-[var(--aria-primary)]/40 bg-[var(--aria-primary-soft)] px-4 py-3 text-sm text-[var(--aria-ink)] shadow-sm"
          >
            <div className="flex items-center gap-2 font-semibold">
              <span className="relative flex h-3 w-3">
                <span className="absolute inline-flex h-full w-full rounded-full bg-[var(--aria-primary)] opacity-50 motion-safe:animate-ping" />
                <span className="relative inline-flex h-3 w-3 rounded-full bg-[var(--aria-primary)]" />
              </span>
              正在处理，请稍候…已等待 {elapsed} 秒
            </div>
            <p className="mt-1.5 text-xs leading-5">
              Agent 正在迭代提示词或生成图片，请耐心等待，<strong>不要重复点击或刷新页面</strong>。
            </p>
          </div>
        ) : null}
      </div>
      <form
        data-testid="image-create-chat-form"
        onSubmit={handleSubmit}
        className="border-t border-[var(--aria-line)] bg-[var(--aria-panel)] p-4"
      >
        <textarea
          aria-label="创作消息"
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          disabled={isBusy || !currentSession}
          rows={3}
          placeholder={isBusy ? "Agent 正在处理上一条消息…" : "描述图片目标或提出修改意见"}
          className="block min-h-24 w-full resize-y rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-3 text-sm leading-6 text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 placeholder:text-[var(--aria-ink-muted)] hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:bg-[var(--aria-panel-muted)] disabled:opacity-70"
        />
        <div className="mt-3 flex items-center justify-between gap-3">
          <span className="text-xs text-[var(--aria-ink-muted)]">
            {isBusy ? "忙碌时输入已锁定" : "发送后等待 Agent 返回 suggested_prompt"}
          </span>
          <button
            type="submit"
            disabled={isBusy || !currentSession || !message.trim()}
            className="inline-flex cursor-pointer items-center gap-2 rounded-xl bg-gradient-to-b from-[var(--aria-primary)] to-[#0e7490] px-4 py-2.5 text-sm font-semibold text-white shadow-[0_4px_12px_rgba(8,145,178,0.24)] transition-all duration-200 hover:translate-y-px hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Send aria-hidden="true" className="h-4 w-4" />
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
        <article className="ml-auto max-w-[85%] rounded-2xl rounded-br-md bg-gradient-to-br from-[var(--aria-primary)] to-[#0e7490] px-4 py-3 text-sm leading-6 text-white shadow-[0_5px_14px_rgba(8,145,178,0.2)]">
          {entry.content}
        </article>
      );
    case "provider_text":
      return (
        <article className="max-w-[90%] whitespace-pre-wrap rounded-2xl rounded-bl-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-3 text-sm leading-6 shadow-sm">
          {entry.content}
        </article>
      );
    case "prompt_block":
      return (
        <article className="rounded-2xl border border-[var(--aria-primary)]/50 bg-[var(--aria-primary-soft)] px-4 py-4 shadow-sm">
          <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-[var(--aria-ink)]">
            <Sparkles aria-hidden="true" className="h-3.5 w-3.5 text-[var(--aria-primary)]" />
            Suggested prompt{entry.version ? ` · v${entry.version}` : ""}
          </div>
          <p className="mt-2 whitespace-pre-wrap text-sm leading-6 text-[var(--aria-ink)]">
            {entry.content}
          </p>
        </article>
      );
    case "generation_image": {
      const extension = entry.mediaType.split("/")[1] ?? "png";
      const dataUri = `data:${entry.mediaType};base64,${entry.base64}`;
      return (
        <article className="overflow-hidden rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-[0_4px_16px_rgba(15,23,42,0.08),0_16px_36px_rgba(15,23,42,0.08)] transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg">
          <div className="bg-[var(--aria-panel-subtle)] p-2">
            <img
              src={dataUri}
              alt={entry.prompt || "生成图片"}
              className="max-h-[32rem] w-full rounded-xl object-contain"
            />
          </div>
          <div className="flex items-center justify-between gap-3 border-t border-[var(--aria-line)] px-4 py-3">
            <p className="min-w-0 text-xs leading-5 text-[var(--aria-ink-muted)]">{entry.prompt}</p>
            <a
              href={dataUri}
              download={`image-create-${Date.now()}.${extension}`}
              className="inline-flex shrink-0 cursor-pointer items-center gap-1.5 rounded-xl bg-gradient-to-b from-[var(--aria-primary)] to-[#0e7490] px-3 py-2 text-xs font-semibold text-white shadow-[0_3px_10px_rgba(8,145,178,0.22)] transition-all duration-200 hover:translate-y-px hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
            >
              <Download aria-hidden="true" className="h-3.5 w-3.5" />
              下载原图
            </a>
          </div>
        </article>
      );
    }
    case "generation_error":
      return (
        <div
          role="alert"
          className="rounded-xl border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-4 py-3 text-sm font-semibold text-[var(--aria-ink)] shadow-sm"
        >
          生成失败：{entry.content}
        </div>
      );
    case "system_notice":
      return (
        <div className="rounded-xl border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-4 py-3 text-sm text-[var(--aria-ink)] shadow-sm">
          {entry.content}
        </div>
      );
    case "busy_notice":
      return (
        <div className="rounded-xl border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-4 py-3 text-sm font-semibold text-[var(--aria-ink)] shadow-sm">
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
