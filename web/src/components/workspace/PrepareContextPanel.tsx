import { useState, type FormEvent } from "react";

interface PrepareContextPanelProps {
  onSendContextNote: (content: string) => void;
  onStartGeneration: () => void;
  contextNotes: string[];
  disabled?: boolean;
}

export function PrepareContextPanel({
  onSendContextNote,
  onStartGeneration,
  contextNotes,
  disabled = false,
}: PrepareContextPanelProps) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const listScrollable = contextNotes.length > 3;

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!trimmedInput) {
      return;
    }
    onSendContextNote(trimmedInput);
    setInput("");
  }

  return (
    <section
      data-testid="prepare-context-panel"
      className="flex flex-col gap-4"
      aria-label="准备上下文"
    >
      <div className="space-y-2">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
          已补充上下文 {contextNotes.length} 条
        </h2>
        {contextNotes.length > 0 ? (
          <ul className={`space-y-1 ${listScrollable ? "max-h-32 overflow-y-auto" : ""}`}>
            {contextNotes.map((note, index) => (
              <li
                key={`${index}-${note}`}
                className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1.5 text-sm text-[var(--aria-ink)]"
              >
                {note}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-[var(--aria-ink-muted)]">
            还没有补充上下文。发送后等待后端确认再进入列表。
          </p>
        )}
      </div>

      <form onSubmit={handleSubmit} className="space-y-2">
        <textarea
          data-testid="context-note-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          placeholder="补充上下文（回车换行，点击发送按钮提交）"
          disabled={disabled}
          rows={3}
          className="min-h-24 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        />
        <div className="flex justify-end">
          <button
            data-testid="send-context-note"
            type="submit"
            disabled={disabled || !trimmedInput}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
          >
            发送上下文
          </button>
        </div>
      </form>

      <button
        data-testid="start-generation"
        type="button"
        onClick={onStartGeneration}
        disabled={disabled}
        className="inline-flex h-9 w-full items-center justify-center rounded-md bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:opacity-50"
      >
        开始生成
      </button>
    </section>
  );
}
