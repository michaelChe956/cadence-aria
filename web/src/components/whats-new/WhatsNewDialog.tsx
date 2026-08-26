import { Sparkles, X } from "lucide-react";
import type { ChangelogEntry } from "../../whats-new/changelog";

export function WhatsNewDialog({
  entries,
  onClose,
}: {
  entries: ChangelogEntry[];
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/40 p-4 backdrop-blur-sm">
      <div
        role="dialog"
        aria-label="版本更新说明"
        aria-modal="true"
        className="max-h-[calc(100vh-2rem)] w-full max-w-lg overflow-y-auto rounded-2xl border border-white/80 bg-[var(--aria-panel)] p-5 shadow-[0_24px_64px_rgba(15,23,42,0.22)]"
      >
        <div className="mb-5 flex items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
              <Sparkles aria-hidden="true" className="h-5 w-5" />
            </span>
            <h2 className="text-base font-semibold text-[var(--aria-ink)]">版本更新说明</h2>
          </div>
          <button
            type="button"
            aria-label="关闭"
            onClick={onClose}
            className="inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
          >
            <X aria-hidden="true" className="h-4 w-4" />
          </button>
        </div>

        <div className="divide-y divide-[var(--aria-line)]">
          {entries.map((entry) => (
            <section
              key={entry.version}
              aria-label={`${entry.version} · ${entry.date}`}
              className="py-5 first:pt-0 last:pb-0"
            >
              <h3 className="text-base font-semibold text-[var(--aria-ink)]">
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
            className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 py-2 text-sm font-semibold text-white transition-all duration-200 hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
          >
            知道了
          </button>
        </div>
      </div>
    </div>
  );
}
