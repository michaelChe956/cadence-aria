import { CheckCircle2, Circle, LoaderCircle } from "lucide-react";
import type { WorkspaceSession } from "../../api/types";

export function WorkspaceFlowRail({
  workspaceType,
  status,
}: {
  workspaceType: WorkspaceSession["workspace_type"];
  status: WorkspaceSession["status"];
}) {
  const steps =
    workspaceType === "work_item"
      ? ["author plan", "confirm plan", "coding", "testing", "review", "final"]
      : ["prepare context", "author draft", "cross review", "revise", "human confirm"];

  return (
    <nav
      aria-label="Workspace 流程"
      className="min-h-0 overflow-auto border-r border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
    >
      <ol className="space-y-2">
        {steps.map((step, index) => {
          const Icon = firstStepIcon(index, status);
          return (
            <li
              key={step}
              className="flex items-center gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1.5 text-xs font-semibold text-[var(--aria-ink)]"
            >
              <Icon className="h-3.5 w-3.5 shrink-0 text-[var(--aria-primary)]" />
              <span>{step}</span>
            </li>
          );
        })}
      </ol>
      <p className="mt-3 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1 font-mono text-[11px] text-[var(--aria-ink-muted)]">
        {status}
      </p>
    </nav>
  );
}

function firstStepIcon(index: number, status: WorkspaceSession["status"]) {
  if (index !== 0) {
    return Circle;
  }

  return status === "running" ? LoaderCircle : CheckCircle2;
}
