import { useEffect } from "react";
import { CircleHelp } from "lucide-react";
import type { CodingAttemptAddress } from "./api/types";
import { IssueLifecycleWorkbench } from "./components/lifecycle/IssueLifecycleWorkbench";
import { WhatsNewDialog } from "./components/whats-new/WhatsNewDialog";
import { OnboardingWizard } from "./onboarding/OnboardingWizard";
import { useOnboarding } from "./onboarding/useOnboarding";
import { useWhatsNew } from "./whats-new/useWhatsNew";

export function AppShell({
  focusEntityKey,
  onDrawerFocusChange,
  onOpenWorkspace,
  onOpenCodingWorkspace,
}: {
  focusEntityKey?: string | null;
  onDrawerFocusChange?: (entityKey: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
  onOpenCodingWorkspace?: (address: CodingAttemptAddress) => void;
}) {
  const whatsNew = useWhatsNew();
  const onboarding = useOnboarding();

  useEffect(() => {
    const previousScrollRestoration = window.history.scrollRestoration;
    window.history.scrollRestoration = "manual";
    // v40 复验 #2：外壳（CockpitShell）接管视口高度后文档不再滚动，
    // 进入工作台的「回顶」重置作用到外壳滚动容器；window 重置保留作兜底
    //（直出/无外壳渲染场景）。
    document
      .querySelector('[data-testid="cockpit-shell-scroll"]')
      ?.scrollTo({ top: 0, left: 0 });
    window.scrollTo({ top: 0, left: 0, behavior: "auto" });

    return () => {
      window.history.scrollRestoration = previousScrollRestoration;
    };
  }, []);

  return (
    <div className="relative">
      {whatsNew.open && whatsNew.entries.length > 0 ? (
        <WhatsNewDialog entries={whatsNew.entries} onClose={whatsNew.close} />
      ) : null}
      {onboarding.open && onboarding.current ? (
        <OnboardingWizard
          steps={onboarding.steps}
          index={onboarding.index}
          onPrev={onboarding.prev}
          onNext={onboarding.next}
          onSkip={onboarding.skip}
          onClose={onboarding.close}
        />
      ) : null}
      <button
        type="button"
        data-testid="onboarding-help-trigger"
        aria-label="重新打开操作引导"
        onClick={onboarding.reopen}
        className="fixed bottom-4 left-4 z-40 inline-flex cursor-pointer items-center gap-1.5 rounded-full border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-xs font-semibold text-[var(--aria-ink)] shadow-lg transition-colors hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        <CircleHelp aria-hidden="true" className="h-4 w-4" />
        操作引导
      </button>
      <a
        href="/image-create"
        className="fixed bottom-4 right-4 z-40 rounded-full border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 py-2 text-sm font-semibold text-white shadow-lg transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
      >
        图片创作
      </a>
      <IssueLifecycleWorkbench
        focusEntityKey={focusEntityKey}
        onDrawerFocusChange={onDrawerFocusChange}
        onOpenWorkspace={onOpenWorkspace}
        onOpenCodingWorkspace={onOpenCodingWorkspace}
      />
    </div>
  );
}
