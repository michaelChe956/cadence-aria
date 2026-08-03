import { useImageCreateStore } from "../../state/image-create-store";

export function PromptBlock() {
  const prompt = useImageCreateStore((state) => state.params.prompt);
  const entries = useImageCreateStore((state) => state.entries);
  const isBusy = useImageCreateStore((state) => state.isBusy);
  const editPromptBlock = useImageCreateStore((state) => state.editPromptBlock);
  const retainedPromptNotice = [...entries]
    .reverse()
    .find(
      (entry) =>
        entry.type === "system_notice" &&
        entry.content.includes("已保留上一版"),
    );

  return (
    <section
      aria-labelledby="image-create-prompt-heading"
      className="rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-sm"
    >
      <div className="flex items-center justify-between gap-3">
        <h2 id="image-create-prompt-heading" className="text-base font-semibold">
          建议提示词
        </h2>
        <span className="text-xs text-[var(--aria-ink-muted)]">可直接编辑</span>
      </div>
      <textarea
        aria-label="建议提示词"
        value={prompt}
        onChange={(event) => editPromptBlock(event.target.value)}
        disabled={isBusy}
        rows={7}
        placeholder="Agent 给出的 suggested_prompt 会显示在这里，也可手动填写。"
        className="mt-3 block min-h-36 w-full resize-y rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm leading-6 text-[var(--aria-ink)] transition-colors placeholder:text-[var(--aria-ink-muted)] hover:border-[var(--aria-line-strong)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:opacity-70"
      />
      {retainedPromptNotice ? (
        <p
          role="status"
          className="mt-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs font-semibold text-[var(--aria-ink)]"
        >
          {retainedPromptNotice.content}
        </p>
      ) : null}
    </section>
  );
}
