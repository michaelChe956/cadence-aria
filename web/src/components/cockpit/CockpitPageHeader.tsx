import type { JSX } from "react";
import { ArrowLeft, History } from "lucide-react";
import { CockpitInboxDrawerTrigger } from "./CockpitInboxDrawer";
import { useCockpitSettingsSlotRef } from "./CockpitShell";

/**
 * 驾驶舱页头：会话导航 + 审计/推进动作，右侧为入口区——
 * 待处理抽屉入口（计数提示）与「驾驶舱设置」宿主。设置入口由 shell portal
 * 注入此宿主，入口不再以 fixed 浮层压住 spec 抽屉与内容区（UI-A）。
 */
export function CockpitPageHeader({
  sessionId,
  watchWindow,
  parentSessionId,
  onBack,
  onOpenSession,
  onOpenAudit,
  canManualAdvance,
  onAdvance,
  inboxCount,
  inboxOpen,
  onToggleInbox,
}: {
  sessionId: string;
  watchWindow: string;
  parentSessionId: string | null;
  onBack: () => void;
  onOpenSession: (sessionId: string) => void;
  onOpenAudit: () => void;
  canManualAdvance: boolean;
  onAdvance: () => void;
  inboxCount: number;
  inboxOpen: boolean;
  onToggleInbox: () => void;
}): JSX.Element {
  const settingsSlotRef = useCockpitSettingsSlotRef();

  return (
    <header className="flex min-h-11 items-center gap-2 border-b border-[var(--aria-line)] px-3 py-1">
      <button
        type="button"
        onClick={onBack}
        className="btn-secondary h-11 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        <ArrowLeft className="h-4 w-4" aria-hidden="true" />
        返回
      </button>
      {parentSessionId !== null && parentSessionId !== sessionId ? (
        <button
          type="button"
          onClick={() => onOpenSession(parentSessionId)}
          className="btn-secondary h-11 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          返回父会话
        </button>
      ) : null}
      <span className="aria-mono text-xs text-[var(--aria-ink-muted)]">{sessionId}</span>
      <span className="text-xs text-[var(--aria-ink-muted)]">{watchWindow}</span>
      <button
        type="button"
        aria-label="操作审计"
        onClick={onOpenAudit}
        className="inline-flex min-h-11 items-center gap-1 rounded-md px-3 text-xs font-semibold text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        <History aria-hidden="true" className="h-4 w-4" />
        操作审计
      </button>
      {canManualAdvance ? (
        <button
          type="button"
          onClick={onAdvance}
          className="inline-flex min-h-11 items-center rounded-md border border-[var(--aria-line-strong)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          手动推进
        </button>
      ) : null}
      <div className="ml-auto flex items-center gap-2">
        <CockpitInboxDrawerTrigger
          count={inboxCount}
          open={inboxOpen}
          onToggle={onToggleInbox}
        />
        <div
          data-testid="cockpit-settings-slot"
          ref={settingsSlotRef}
          className="flex items-center"
        />
      </div>
    </header>
  );
}
