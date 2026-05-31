import { useEffect } from "react";
import { IssueLifecycleWorkbench } from "./components/lifecycle/IssueLifecycleWorkbench";

export function AppShell({
  focusEntityId,
  onDrawerFocusChange,
  onOpenWorkspace,
  onOpenCodingWorkspace,
}: {
  focusEntityId?: string | null;
  onDrawerFocusChange?: (entityId: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
  onOpenCodingWorkspace?: (attemptId: string) => void;
}) {
  useEffect(() => {
    const previousScrollRestoration = window.history.scrollRestoration;
    window.history.scrollRestoration = "manual";
    window.scrollTo({ top: 0, left: 0, behavior: "auto" });

    return () => {
      window.history.scrollRestoration = previousScrollRestoration;
    };
  }, []);

  return (
    <IssueLifecycleWorkbench
      focusEntityId={focusEntityId}
      onDrawerFocusChange={onDrawerFocusChange}
      onOpenWorkspace={onOpenWorkspace}
      onOpenCodingWorkspace={onOpenCodingWorkspace}
    />
  );
}
