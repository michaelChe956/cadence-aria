import { Sparkles } from "lucide-react";
import { DialogShell } from "../shared/DialogShell";
import type { ChangelogEntry } from "../../whats-new/changelog";

export function WhatsNewDialog({
  entries,
  onClose,
}: {
  entries: ChangelogEntry[];
  onClose: () => void;
}) {
  return (
    <DialogShell
      label="版本更新说明"
      onClose={onClose}
      // e2e 以可访问名「关闭」点击本按钮（workbench.page dismissFirstRunOverlays）。
      closeLabel="关闭"
      icon={<Sparkles className="h-4 w-4" aria-hidden="true" />}
    >
      <div className="divide-y divide-[var(--aria-line)]">
        {entries.map((entry) => (
          <section
            key={entry.version}
            aria-label={`${entry.version} · ${entry.date}`}
            className="py-5 first:pt-0 last:pb-0"
          >
            <h3 className="text-sm font-semibold text-[var(--aria-ink)]">
              v{entry.version} · {entry.date}
            </h3>
            <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">{entry.title}</p>
            <ul className="mt-3 space-y-2.5">
              {entry.highlights.map((item) => (
                <li
                  key={item}
                  className="flex items-start gap-2.5 text-sm text-[var(--aria-ink)]"
                >
                  <span
                    aria-hidden="true"
                    className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--aria-primary)]"
                  />
                  <span>{item}</span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>

      <div className="mt-6 flex justify-end">
        <button
          type="button"
          onClick={onClose}
          className="inline-flex cursor-pointer items-center gap-1.5 rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 py-2 text-sm font-semibold text-white transition-colors duration-200 motion-reduce:transition-none hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          知道了
        </button>
      </div>
    </DialogShell>
  );
}
