// 居中弹窗统一外壳（自研，无第三方依赖）：web/ 内全部自绘 dialog 共用。
// 规范移植自 LogicalCodebaseManagementDrawer（项目唯一已修好的浮层先例）：
// - 层 fixed inset-0 + 蒙层 absolute inset-0 bg-black/35（父层 fixed → 恒铺满视口，
//   页面滚动后蒙层不再错位/消失）；层经 createPortal 挂 document.body，
//   免疫 transform/filter 等祖先把 fixed 包含块劫持走（图片创作会话抽屉踩过的坑）；
// - 打开时 body overflow 锁定 + 内容滚动区 overscroll-contain（滚轮不穿透到页面）；
// - Esc / 点蒙层 / X 三路关闭（closeDisabled 时全部抑制）；打开焦点移入卡片、
//   卸载归还触发元素；Tab 循环圈闭（焦点不在本层时不劫持——上层再叠弹窗的场景）；
// - 200ms opacity/transform 过场 + motion-reduce 无过渡直切；hover 只变色不位移；
// - 标题栏形态对齐抽屉 header：text-sm font-semibold + h-7 w-7 X 按钮。
import {
  useEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";

const TRANSITION_MS = 200;

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

// 跨两帧再切可视态：真实浏览器里同帧加 class 不会触发 transform/opacity 过渡；
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

type ModalBehaviorOptions = {
  active: boolean;
  onClose: (() => void) | null;
  closeDisabled: boolean;
  containerRef: RefObject<HTMLElement | null>;
  /**
   * UI-1：默认 false（既有语义不变）——Esc 一律关闭本层。true 时仅当焦点
   * 在本容器内才响应 Esc：用于与其它浮层并存的常驻层（/workbench 抽屉），
   * 上层浮层持有焦点时把 Esc 留给它，避免一次 Esc 连关多层。
   */
  escOnlyWhenFocused?: boolean;
};

// 浮层行为内核（body 锁 / Esc / 焦点移入归还 / Tab 圈闭）：
// DialogShell 与页面内联抽屉（CodingWorkspacePage 辅助面板）共用，
// 行为规范与 LogicalCodebaseManagementDrawer 一致。
export function useModalBehavior({
  active,
  onClose,
  closeDisabled,
  containerRef,
  escOnlyWhenFocused = false,
}: ModalBehaviorOptions) {
  const onCloseRef = useRef(onClose);
  const closeDisabledRef = useRef(closeDisabled);
  onCloseRef.current = onClose;
  closeDisabledRef.current = closeDisabled;

  useEffect(() => {
    if (!active) {
      return;
    }
    const container = containerRef.current;
    const restoreTarget =
      document.activeElement instanceof HTMLElement &&
      container?.contains(document.activeElement) === false
        ? document.activeElement
        : null;
    container?.focus();
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    // window 级监听而非层内 React onKeyDown：页面内联抽屉没有统一的层节点可挂。
    // 焦点不在本容器内（上层又叠了弹窗）时不处理，避免劫持其键盘。
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        if (escOnlyWhenFocused) {
          const scope = containerRef.current;
          if (scope && !scope.contains(document.activeElement)) {
            return;
          }
        }
        if (!closeDisabledRef.current) {
          onCloseRef.current?.();
        }
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const scope = containerRef.current;
      if (!scope || !scope.contains(document.activeElement)) {
        return;
      }
      const focusables = Array.from(
        scope.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      );
      if (focusables.length === 0) {
        event.preventDefault();
        scope.focus();
        return;
      }
      const first = focusables[0]!;
      const last = focusables[focusables.length - 1]!;
      const current = document.activeElement;
      if (event.shiftKey) {
        if (current === first) {
          event.preventDefault();
          last.focus();
        }
      } else if (current === last) {
        event.preventDefault();
        first.focus();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = previousOverflow;
      if (restoreTarget && document.contains(restoreTarget)) {
        restoreTarget.focus();
      }
    };
  }, [active, containerRef]);
}

export type DialogShellProps = {
  /** role=dialog 的可访问名（e2e 契约，迁移时保持原名）。 */
  label: string;
  /** null = 不渲染 X 按钮，Esc/蒙层也不关（宿主自管关闭时用）。 */
  onClose: (() => void) | null;
  /** 提交/初始化进行中等不可中断场景：X 禁用，Esc 与点蒙层均不关闭。 */
  closeDisabled?: boolean;
  closeLabel?: string;
  title?: ReactNode;
  icon?: ReactNode;
  description?: ReactNode;
  /** 卡片宽度 class（max-w-*），默认 max-w-lg。 */
  widthClass?: string;
  /** 层级 class（z-50 / z-[80]…），默认 z-50。 */
  layerClass?: string;
  /** 卡片 data-testid。 */
  testId?: string;
  /** 可选外部卡片 ref：宿主需要在特定时机（如长流程中）把焦点拉回弹窗卡片时使用。 */
  dialogRef?: RefObject<HTMLElement | null>;
  children: ReactNode;
};

export function DialogShell({
  label,
  onClose,
  closeDisabled = false,
  closeLabel,
  title,
  icon,
  description,
  widthClass = "max-w-lg",
  layerClass = "z-50",
  testId,
  dialogRef,
  children,
}: DialogShellProps) {
  // 挂载即显示（宿主条件渲染控制开合）：进场跨两帧切可视态驱动蒙层淡入与卡片缩放归位。
  const [shown, setShown] = useState(false);
  const cardRef = useRef<HTMLElement | null>(null);
  useModalBehavior({
    active: true,
    onClose,
    closeDisabled,
    containerRef: cardRef,
  });

  useEffect(() => {
    onNextFrame(() => setShown(true));
  }, []);

  return createPortal(
    <div
      className={`fixed inset-0 ${layerClass}`}
      {...(testId ? { "data-testid": `${testId}-layer` } : {})}
    >
      <div
        aria-hidden="true"
        onClick={closeDisabled || !onClose ? undefined : onClose}
        className={`absolute inset-0 bg-black/35 transition-opacity duration-200 motion-reduce:transition-none ${
          shown ? "opacity-100" : "opacity-0"
        }`}
      />
      {/* 定位壳不拦截蒙层点击；卡片恢复交互。 */}
      <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-4">
        <section
          ref={(node) => {
            cardRef.current = node;
            if (dialogRef) {
              dialogRef.current = node;
            }
          }}
          role="dialog"
          aria-modal="true"
          aria-label={label}
          tabIndex={-1}
          {...(testId ? { "data-testid": testId } : {})}
          className={[
            "pointer-events-auto flex max-h-[85vh] w-full flex-col overflow-hidden",
            `rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl ${widthClass}`,
            "transition-[opacity,transform] duration-200 ease-out motion-reduce:transition-none",
            shown
              ? "scale-100 opacity-100"
              : "translate-y-2 scale-[0.98] opacity-0",
          ].join(" ")}
        >
          <header className="flex shrink-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] px-4 py-3">
            <div className="flex min-w-0 items-center gap-2.5">
              {icon ? (
                <span
                  aria-hidden="true"
                  className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                >
                  {icon}
                </span>
              ) : null}
              <div className="min-w-0">
                <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
                  {title ?? label}
                </h2>
                {description ? (
                  <p className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">
                    {description}
                  </p>
                ) : null}
              </div>
            </div>
            {onClose ? (
              <button
                type="button"
                aria-label={closeLabel ?? `关闭${label}`}
                onClick={onClose}
                disabled={closeDisabled}
                className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors duration-200 motion-reduce:transition-none hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:cursor-not-allowed disabled:opacity-60"
              >
                <X className="h-4 w-4" aria-hidden="true" />
              </button>
            ) : null}
          </header>
          {/* 内容区独立滚动 + overscroll-contain：滚到边界不链到页面（滚轮穿透根因）。 */}
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain p-4">
            {children}
          </div>
        </section>
      </div>
    </div>,
    document.body,
  );
}
