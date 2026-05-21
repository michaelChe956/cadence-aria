import { type ReactNode } from "react";
import type { ChatEntryRole } from "../../state/chat-entries";

interface ChatEntryContainerProps {
  role: ChatEntryRole;
  title: string;
  children: ReactNode;
  className?: string;
  testId?: string;
}

const ROLE_STYLES: Record<
  ChatEntryRole,
  { wrapper: string; panel: string; title: string }
> = {
  user: {
    wrapper: "justify-end",
    panel: "border-blue-200 bg-blue-50",
    title: "text-blue-700",
  },
  author: {
    wrapper: "justify-start",
    panel: "border-[var(--aria-line)] bg-white",
    title: "text-[var(--aria-primary)]",
  },
  reviewer: {
    wrapper: "justify-start",
    panel: "border-amber-200 bg-amber-50",
    title: "text-amber-700",
  },
  system: {
    wrapper: "justify-center",
    panel: "border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)]",
    title: "text-[var(--aria-ink-muted)]",
  },
};

export function ChatEntryContainer({
  role,
  title,
  children,
  className = "",
  testId,
}: ChatEntryContainerProps) {
  const styles = ROLE_STYLES[role];

  return (
    <div className={`flex min-w-0 ${styles.wrapper}`}>
      <article
        data-testid={testId}
        className={[
          "w-full rounded-md border px-3 py-2 text-sm shadow-sm",
          role === "system" ? "max-w-none" : "max-w-3xl",
          styles.panel,
          className,
        ]
          .filter(Boolean)
          .join(" ")}
      >
        <div className="flex min-w-0 items-center justify-between gap-3">
          <span className={`truncate text-xs font-semibold ${styles.title}`}>{title}</span>
        </div>
        <div className="mt-2 min-w-0">{children}</div>
      </article>
    </div>
  );
}
