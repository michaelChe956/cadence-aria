import { useRef, useState, type ReactNode } from "react";
import { Maximize2, Minimize2, X } from "lucide-react";
import { useModalBehavior } from "./DialogShell";

/**
 * UI-1/UI-3 共享宽抽屉壳：右侧锚定、蒙层点击关闭、可切全屏；小屏（<md）
 * 默认铺满视口。行为内核复用 useModalBehavior（Esc/焦点圈闭与归还/滚动锁），
 * 与 DialogShell 同一规范；视觉形态是宽抽屉而非居中卡片，故不复用 DialogShell
 * 布局。escOnlyWhenFocused：与工作台实体抽屉等浮层并存时，Esc 只在焦点位于
 * 本层内才关闭（避免一次 Esc 连关多层）。
 */
export function WorkbenchOverlayDrawer({
  label,
  title,
  description,
  onClose,
  testId,
  headerExtra,
  children,
}: {
  /** role=dialog 的可访问名。 */
  label: string;
  title: string;
  description?: string;
  onClose: () => void;
  testId: string;
  headerExtra?: ReactNode;
  children: ReactNode;
}) {
  const [fullscreen, setFullscreen] = useState(false);
  const containerRef = useRef<HTMLElement | null>(null);
  useModalBehavior({
    active: true,
    onClose,
    closeDisabled: false,
    containerRef,
    escOnlyWhenFocused: true,
  });

  return (
    <div className="fixed inset-0 z-50" data-testid={`${testId}-layer`}>
      <div
        aria-hidden="true"
        onClick={onClose}
        className="absolute inset-0 bg-black/35"
      />
      <section
        ref={(node) => {
          containerRef.current = node;
        }}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        data-testid={testId}
        className={`absolute right-0 top-0 flex h-full min-h-0 flex-col border-l border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl ${
          fullscreen
            ? "w-screen"
            : "w-screen md:w-[min(72rem,calc(100vw-3rem))]"
        }`}
      >
        <header className="flex h-11 shrink-0 items-center gap-2 border-b border-[var(--aria-line)] px-4 text-xs">
          <span className="shrink-0 font-semibold text-[var(--aria-ink)]">
            {title}
          </span>
          {description ? (
            <span
              data-testid={`${testId}-description`}
              className="min-w-0 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]"
            >
              {description}
            </span>
          ) : null}
          <div className="ml-auto flex shrink-0 items-center gap-1">
            {headerExtra}
            <button
              type="button"
              aria-label={fullscreen ? "退出全屏" : "全屏"}
              data-testid={`${testId}-fullscreen`}
              onClick={() => setFullscreen((value) => !value)}
              className="inline-flex h-7 w-7 items-center justify-center rounded-md text-[var(--aria-ink-muted)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)]"
            >
              {fullscreen ? (
                <Minimize2 className="h-4 w-4" aria-hidden="true" />
              ) : (
                <Maximize2 className="h-4 w-4" aria-hidden="true" />
              )}
            </button>
            <button
              type="button"
              aria-label={`关闭${label}`}
              data-testid={`${testId}-close`}
              onClick={onClose}
              className="inline-flex h-7 w-7 items-center justify-center rounded-md text-[var(--aria-ink-muted)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)]"
            >
              <X className="h-4 w-4" aria-hidden="true" />
            </button>
          </div>
        </header>
        {/* 内容区不套滚动：总览视图自带滚动容器，详情页（CodingWorkspacePage）
            自管内部滚动，双滚动壳会让两态都出现滚动错位。 */}
        <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      </section>
    </div>
  );
}
