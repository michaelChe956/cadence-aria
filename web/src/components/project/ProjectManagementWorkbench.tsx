import { IssueLifecycleWorkbench } from "../lifecycle/IssueLifecycleWorkbench";
import type { ExecutionContext } from "../task/TaskManagementWorkbench";

export function ProjectManagementWorkbench({
  onOpenExecution: _onOpenExecution,
}: {
  onOpenExecution?: (context: ExecutionContext) => void;
}) {
  return <IssueLifecycleWorkbench />;
}
