// LC 管理面板抽屉化：右侧滑入轻量抽屉外壳（自研，无第三方依赖）。
// 职责仅外壳：半透明遮罩 + 右侧面板 + 进出场过渡 + Esc/遮罩/关闭按钮三路关闭
// + 打开时焦点移入、卸载时归还 + Tab 循环圈闭 + body 滚动锁定；
// 面板内容（LogicalCodebaseManagementPanel）经 children 注入，外壳不改其结构。
// 视觉规范：w-[min(560px,92vw)]、200ms transform/opacity 过渡、motion-reduce 无过渡直切、
// hover 仅变色不位移、层级沿用项目浮层惯例 z-50（普通弹窗同级、系统守卫 z-[100] 之下）。
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { X } from "lucide-react";

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

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

type LogicalCodebaseManagementDrawerProps = {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
};

export function LogicalCodebaseManagementDrawer({
  open,
  onClose,
  children,
}: LogicalCodebaseManagementDrawerProps) {
  // mounted：是否渲染 DOM；shown：是否可视态（驱动遮罩淡入与面板滑入/退场）。
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(false);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

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

  // 挂载期间：焦点移入抽屉、锁定 body 滚动；卸载（含父层直接移除）时两者还原。
  useEffect(() => {
    if (!mounted) {
      return;
    }
    const active = document.activeElement;
    restoreFocusRef.current =
      active instanceof HTMLElement && panelRef.current?.contains(active) === false
        ? active
        : null;
    panelRef.current?.focus();
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
      const restoreTarget = restoreFocusRef.current;
      if (restoreTarget && document.contains(restoreTarget)) {
        restoreTarget.focus();
      }
    };
  }, [mounted]);

  if (!mounted) {
    return null;
  }

  // Tab 循环圈闭（事件仅来自抽屉内部时才处理——上层再叠弹窗时不劫持其键盘）。
  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      onClose();
      return;
    }
    if (event.key !== "Tab") {
      return;
    }
    const panel = panelRef.current;
    if (!panel) {
      return;
    }
    const focusables = Array.from(
      panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
    );
    if (focusables.length === 0) {
      event.preventDefault();
      panel.focus();
      return;
    }
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;
    const active = document.activeElement;
    if (event.shiftKey) {
      if (active === first || !panel.contains(active)) {
        event.preventDefault();
        last.focus();
      }
    } else if (active === last || !panel.contains(active)) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div
      data-testid="lc-management-drawer-layer"
      className="fixed inset-0 z-50"
      onKeyDown={handleKeyDown}
    >
      <div
        data-testid="lc-management-overlay"
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
        aria-label="逻辑代码库管理"
        data-testid="lc-management-drawer"
        tabIndex={-1}
        className={[
          "absolute inset-y-0 right-0 flex w-[min(560px,92vw)] flex-col outline-none",
          "border-l border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl",
          "transition-transform duration-200 ease-out motion-reduce:transition-none",
          shown ? "translate-x-0" : "translate-x-full",
        ].join(" ")}
      >
        <header className="flex shrink-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] px-3 py-2">
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
            逻辑代码库管理
          </h2>
          <button
            type="button"
            data-testid="lc-management-close"
            aria-label="关闭逻辑代码库管理"
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
