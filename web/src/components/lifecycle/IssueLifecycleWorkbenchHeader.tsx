import { CircleHelp, Plus, RefreshCw } from "lucide-react";
import { useCockpitSettingsSlotRef } from "../cockpit/CockpitShell";

export function IssueLifecycleWorkbenchHeader({
  projectName,
  focusedIssueId,
  canCreateIssue,
  onShowAll,
  onRefresh,
  onCreateIssue,
  onReopenOnboarding,
}: {
  projectName?: string;
  focusedIssueId: string | null;
  canCreateIssue: boolean;
  onShowAll: () => void;
  onRefresh: () => void;
  onCreateIssue: () => void;
  /** UI-4：重开操作引导；未接线（如独立测试宿主）时不渲染该入口。 */
  onReopenOnboarding?: () => void;
}) {
  const settingsSlotRef = useCockpitSettingsSlotRef();

  return (
    <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
      <div className="min-w-0">
        <h1 className="truncate text-base font-semibold text-[var(--aria-ink)]">
          Issue 生命周期工作台
        </h1>
        <p className="truncate text-xs text-[var(--aria-ink-muted)]">
          {projectName ?? "未选择 Project"}
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {focusedIssueId ? (
          <button
            type="button"
            onClick={onShowAll}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
          >
            显示全部
          </button>
        ) : null}
        <button
          type="button"
          aria-label="刷新"
          onClick={onRefresh}
          className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)]"
        >
          <RefreshCw className="h-4 w-4" />
        </button>
        <button
          type="button"
          data-testid="onboarding-anchor-issue-create"
          disabled={!canCreateIssue}
          onClick={onCreateIssue}
          className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          <Plus className="mr-1 h-4 w-4" />
          新建 Issue
        </button>
        {onReopenOnboarding ? (
          <button
            type="button"
            data-testid="onboarding-help-trigger"
            aria-label="重新打开操作引导"
            title="重新打开操作引导"
            onClick={onReopenOnboarding}
            className="inline-flex h-11 w-11 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] transition-colors hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <CircleHelp aria-hidden="true" className="h-4 w-4" />
          </button>
        ) : null}
        <div
          data-testid="cockpit-settings-slot"
          ref={settingsSlotRef}
          className="flex items-center"
        />
      </div>
    </div>
  );
}
