// 右侧管理抽屉外壳（自研，无第三方依赖）：LC 管理抽屉与 Provider 管理抽屉共用。
// 职责仅外壳：半透明遮罩 + 右侧面板 + 进出场过渡 + Esc/遮罩/关闭按钮三路关闭
// + 打开时焦点移入、卸载时归还 + Tab 循环圈闭 + body 滚动锁定；
// 面板内容经 children 注入，外壳不改其结构。
// 行为内核复用 ./DialogShell 的 useModalBehavior（Esc/焦点圈闭与归还/滚动锁），
// 与居中弹窗外壳同一规范；视觉形态是右侧滑入面板而非居中卡片，故不复用
// DialogShell 布局。escOnlyWhenFocused：与其它浮层并存时 Esc 只在焦点位于
// 本层内才关闭（避免一次 Esc 连关多层）。
// 视觉规范：w-[min(560px,92vw)]、200ms transform/opacity 过渡、motion-reduce
// 无过渡直切、hover 仅变色不位移、层级沿用项目浮层惯例 z-50。
import { useEffect, useRef, useState, type ReactNode } from "react";
import { X } from "lucide-react";
import { useModalBehavior } from "./DialogShell";

const TRANSITION_MS = 200;

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

// 跨两帧再切可视态：真实浏览器里同帧加 class 不会触发 transform 过渡；
// jsdom 无 rAF 时退化为短定时器（测试环境只关心状态切到位）。
function onNextFrame(callback: () => void): void {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => requestAnimationFrame(() => callback()));
    return;
  }
  setTimeout(callback, 16);
}

export type ManagementDrawerProps = {
  open: boolean;
  onClose: () => void;
  /** role=dialog 的可访问名；同时作为抽屉标题文案与关闭按钮的可访问名前缀。 */
  label: string;
  /** data-testid 前缀：层 `${testIdPrefix}-drawer-layer`、面板 `${testIdPrefix}-drawer`、
   *  遮罩 `${testIdPrefix}-overlay`、关闭按钮 `${testIdPrefix}-close`。 */
  testIdPrefix: string;
  children: ReactNode;
};

export function ManagementDrawer({
  open,
  onClose,
  label,
  testIdPrefix,
  children,
}: ManagementDrawerProps) {
  // mounted：是否渲染 DOM；shown：是否可视态（驱动遮罩淡入与面板滑入/退场）。
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(false);
  const panelRef = useRef<HTMLDivElement | null>(null);
  useModalBehavior({
    active: mounted,
    onClose,
    closeDisabled: false,
    containerRef: panelRef,
    escOnlyWhenFocused: true,
  });

  // open → 挂载并在下一帧进入可视态；!open → 播退场过渡后卸载（reduced motion 直接卸载）。
  useEffect(() => {
    if (open) {
      setMounted(true);
      onNextFrame(() => setShown(true));
      return;
    }
    setShown(false);
    const timer = setTimeout(
      () => setMounted(false),
      prefersReducedMotion() ? 0 : TRANSITION_MS,
    );
    return () => clearTimeout(timer);
  }, [open]);

  if (!mounted) {
    return null;
  }

  return (
    <div
      data-testid={`${testIdPrefix}-drawer-layer`}
      className="fixed inset-0 z-50"
    >
      <div
        data-testid={`${testIdPrefix}-overlay`}
        aria-hidden="true"
        onClick={(event) => {
          if (event.target === event.currentTarget) {
            onClose();
          }
        }}
        className={`absolute inset-0 bg-black/35 transition-opacity duration-200 motion-reduce:transition-none ${
          shown ? "opacity-100" : "opacity-0"
        }`}
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        data-testid={`${testIdPrefix}-drawer`}
        tabIndex={-1}
        className={[
          "absolute inset-y-0 right-0 flex w-[min(560px,92vw)] flex-col outline-none",
          "border-l border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl",
          "transition-transform duration-200 ease-out motion-reduce:transition-none",
          shown ? "translate-x-0" : "translate-x-full",
        ].join(" ")}
      >
        <header className="flex shrink-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] px-3 py-2">
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">{label}</h2>
          <button
            type="button"
            data-testid={`${testIdPrefix}-close`}
            aria-label={`关闭${label}`}
            onClick={onClose}
            className="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors duration-200 motion-reduce:transition-none hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </header>
        {/* 内容区独立滚动：抽屉高度锁定视口，长面板不再撑长工作台页面。 */}
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
          {children}
        </div>
      </div>
    </div>
  );
}
