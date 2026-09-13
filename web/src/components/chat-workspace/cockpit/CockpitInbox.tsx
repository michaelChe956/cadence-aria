import { AlertTriangle, ClipboardList, CircleAlert } from "lucide-react";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";

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

export function CockpitInbox({ items }: { items: CockpitInboxItem[] }) {
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
        items.map((item) => {
          const Glyph = KIND_GLYPH[item.kind];
          return (
            <article
              key={item.id}
              data-testid={`cockpit-inbox-item-${item.kind}`}
              className={[
                "flex min-h-11 items-start gap-2 rounded-lg border-2 px-3 py-2",
                KIND_CLASS[item.kind],
              ].join(" ")}
            >
              <Glyph className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-semibold text-[var(--aria-ink)]">{item.title}</p>
                <p className="mt-1 break-words text-xs leading-4 text-[var(--aria-ink-muted)]">
                  {item.summary}
                </p>
              </div>
            </article>
          );
        })
      )}
    </section>
  );
}
