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
    panel: "border-gray-200 bg-gray-50",
    title: "text-gray-600",
  },
  author: {
    wrapper: "justify-start",
    panel: "border-blue-200 bg-blue-50",
    title: "text-blue-600",
  },
  coder: {
    wrapper: "justify-start",
    panel: "border-blue-200 bg-blue-50",
    title: "text-blue-600",
  },
  tester: {
    wrapper: "justify-start",
    panel: "border-purple-200 bg-purple-50",
    title: "text-purple-600",
  },
  analyst: {
    wrapper: "justify-start",
    panel: "border-amber-200 bg-amber-50",
    title: "text-amber-600",
  },
  reviewer: {
    wrapper: "justify-start",
    panel: "border-green-200 bg-green-50",
    title: "text-green-600",
  },
  code_reviewer: {
    wrapper: "justify-start",
    panel: "border-green-200 bg-green-50",
    title: "text-green-600",
  },
  internal_reviewer: {
    wrapper: "justify-start",
    panel: "border-indigo-200 bg-indigo-50",
    title: "text-indigo-600",
  },
  system: {
    wrapper: "justify-center",
    panel: "border-dashed border-red-200 bg-red-50",
    title: "text-red-500",
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
