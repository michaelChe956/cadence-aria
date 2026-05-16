import { IssueLifecycleWorkbench } from "../lifecycle/IssueLifecycleWorkbench";
import type { ExecutionContext } from "../task/TaskManagementWorkbench";

export function ProjectManagementWorkbench({
  onOpenExecution: _onOpenExecution,
  onOpenWorkspace,
}: {
  onOpenExecution?: (context: ExecutionContext) => void;
  onOpenWorkspace?: (sessionId: string) => void;
}) {
  return <IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />;
}
