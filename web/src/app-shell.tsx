import { useEffect } from "react";
import type { CodingAttemptAddress } from "./api/types";
import { IssueLifecycleWorkbench } from "./components/lifecycle/IssueLifecycleWorkbench";

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
      focusEntityKey={focusEntityKey}
      onDrawerFocusChange={onDrawerFocusChange}
      onOpenWorkspace={onOpenWorkspace}
      onOpenCodingWorkspace={onOpenCodingWorkspace}
    />
  );
}
