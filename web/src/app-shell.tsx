import { useEffect } from "react";
import { IssueLifecycleWorkbench } from "./components/lifecycle/IssueLifecycleWorkbench";

export function AppShell({
  onOpenWorkspace,
}: {
  onOpenWorkspace?: (sessionId: string) => void;
}) {
  useEffect(() => {
    const previousScrollRestoration = window.history.scrollRestoration;
    window.history.scrollRestoration = "manual";
    window.scrollTo({ top: 0, left: 0, behavior: "auto" });

    return () => {
      window.history.scrollRestoration = previousScrollRestoration;
    };
  }, []);

  return <IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />;
}
