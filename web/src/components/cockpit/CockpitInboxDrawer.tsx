import { useEffect, type ReactNode } from "react";
import { Inbox, X } from "lucide-react";

export const COCKPIT_INBOX_DRAWER_ID = "cockpit-inbox-drawer";

/**
 * 待处理入口（挂在页头右侧）：常态只占一条提示条 + 计数徽标，
 * 不占主区域空间；有待处理项（通知）时以危险色提示「有了通知再处理」。
 */
export function CockpitInboxDrawerTrigger({
  count,
  open,
  onToggle,
}: {
  count: number;
  open: boolean;
  onToggle: () => void;
}) {
  const hasPending = count > 0;
  return (
    <button
      type="button"
      data-testid="cockpit-inbox-drawer-trigger"
      aria-expanded={open}
      aria-controls={COCKPIT_INBOX_DRAWER_ID}
      onClick={onToggle}
      className={[
        "inline-flex min-h-11 items-center gap-2 rounded-md border px-3 text-xs font-semibold transition-colors duration-200",
        hasPending
          ? "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-danger)]"
          : "border-[var(--aria-line-strong)] bg-white text-[var(--aria-ink-muted)]",
        "hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
      ].join(" ")}
    >
      <Inbox aria-hidden="true" className="h-4 w-4" />
      待处理
      {hasPending ? (
        <span
          data-testid="cockpit-inbox-drawer-count"
          className="aria-mono rounded-full bg-[var(--aria-danger)] px-1.5 text-[11px] font-semibold text-white"
        >
          {count}
        </span>
      ) : null}
    </button>
  );
}

/**
 * 待处理抽屉（右侧浮层）：默认收起，展开才占屏幕；
 * 收起仅以 CSS 隐藏、不卸载子树——收件箱的勾选/反馈等状态在收起后保留。
 */
export function CockpitInboxDrawer({
  open,
  onClose,
  children,
}: {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    if (!open) {
      return;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  return (
    <>
      {open ? (
        <div
          data-testid="cockpit-inbox-drawer-backdrop"
          aria-hidden="true"
          onClick={onClose}
          className="fixed inset-0 z-[94] bg-black/30"
        />
      ) : null}
      <aside
        id={COCKPIT_INBOX_DRAWER_ID}
        data-testid="cockpit-inbox-drawer"
        data-state={open ? "open" : "closed"}
        aria-label="待处理抽屉"
        className={[
          open ? "flex" : "hidden",
          "fixed inset-y-0 right-0 z-[95] w-[min(26rem,92vw)] flex-col border-l-2 border-[var(--aria-line-strong)] bg-[var(--aria-bg)] shadow-2xl",
        ].join(" ")}
      >
        <div className="flex min-h-11 shrink-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] px-3 py-2">
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">待处理</h2>
          <button
            type="button"
            onClick={onClose}
            className="inline-flex min-h-11 items-center gap-1 rounded-md px-3 text-xs font-semibold text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <X className="h-4 w-4" aria-hidden="true" />
            收起待处理抽屉
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto p-2">{children}</div>
      </aside>
    </>
  );
}
