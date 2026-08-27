import { useEffect } from "react";
import type { CodingAttemptAddress } from "./api/types";
import { IssueLifecycleWorkbench } from "./components/lifecycle/IssueLifecycleWorkbench";
import { WhatsNewDialog } from "./components/whats-new/WhatsNewDialog";
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

  useEffect(() => {
    const previousScrollRestoration = window.history.scrollRestoration;
    window.history.scrollRestoration = "manual";
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
