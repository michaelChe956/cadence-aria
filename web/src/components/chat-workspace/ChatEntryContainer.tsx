import { type ReactNode } from "react";
import type { ChatEntryRole } from "../../state/chat-entries";

interface ChatEntryContainerProps {
  role: ChatEntryRole;
  title: string;
  titleSuffix?: ReactNode;
  children: ReactNode;
  className?: string;
  /**
   * F-49 A7：显式替换 role 面板色。`className` 与 role 面板并列输出时胜负由 Tailwind
   * 输出顺序决定（同级同权重），实测 reviewer 的 green 会压掉作者的 amber——需要
   * 覆盖面板色时走本 prop，不靠权重轮盘。
   */
  panelClassName?: string;
  /**
   * F-50 裁决 9（颜色契约）：显式替换 role 标题色。system role 默认红标题会把
   * 门禁开放这类「待人工」流程状态染成错误色——门卡用本 prop 覆盖为 gate-open
   * 色，红色只留给错误/危险（与 panelClassName 同款显式覆盖纪律）。
   */
  titleClassName?: string;
  testId?: string;
  wide?: boolean;
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
  titleSuffix,
  children,
  className = "",
  panelClassName,
  titleClassName,
  testId,
  wide = false,
}: ChatEntryContainerProps) {
  const styles = ROLE_STYLES[role];

  return (
    <div className={`flex min-w-0 ${styles.wrapper}`}>
      <article
        data-testid={testId}
        className={[
          "w-full rounded-md border px-3 py-2 text-sm shadow-sm",
          role === "system" || wide ? "max-w-none" : "max-w-3xl",
          panelClassName ?? styles.panel,
          className,
        ]
          .filter(Boolean)
          .join(" ")}
      >
        <div className="flex min-w-0 items-center justify-between gap-3">
          <span
            className={`truncate text-xs font-semibold ${titleClassName ?? styles.title}`}
          >
            {title}
          </span>
          {titleSuffix}
        </div>
      <div className="mt-2 min-w-0">{children}</div>
      </article>
    </div>
  );
}
