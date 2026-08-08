import { Sparkles } from "lucide-react";
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
      className="rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-[0_2px_8px_rgba(15,23,42,0.04),0_10px_28px_rgba(15,23,42,0.06)] transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md"
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
            <Sparkles aria-hidden="true" className="h-4 w-4" />
          </span>
          <h2 id="image-create-prompt-heading" className="text-base font-semibold">
            建议提示词
          </h2>
        </div>
        <span className="rounded-full bg-[var(--aria-panel-subtle)] px-2.5 py-1 text-xs font-medium text-[var(--aria-ink-muted)]">可直接编辑</span>
      </div>
      <textarea
        aria-label="建议提示词"
        value={prompt}
        onChange={(event) => editPromptBlock(event.target.value)}
        disabled={isBusy}
        rows={7}
        placeholder="Agent 给出的 suggested_prompt 会显示在这里，也可手动填写。"
        className="mt-4 block min-h-36 w-full resize-y rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-3 text-base leading-6 text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 placeholder:text-[var(--aria-ink-muted)] hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:bg-[var(--aria-panel-muted)] disabled:opacity-70 sm:text-sm"
      />
      {retainedPromptNotice ? (
        <p
          role="status"
          className="mt-3 rounded-xl border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs font-semibold text-[var(--aria-ink)] shadow-sm"
        >
          {retainedPromptNotice.content}
        </p>
      ) : null}
    </section>
  );
}
