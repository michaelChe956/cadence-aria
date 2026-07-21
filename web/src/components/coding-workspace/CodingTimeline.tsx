import {
  Circle,
  Code,
  FlaskConical,
  GitBranch,
  GitPullRequest,
  SearchCode,
  ShieldCheck,
  UserCheck,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { CodingExecutionStage, CodingTimelineNode } from "../../api/types";
import type { PlanRepairSessionState } from "../../state/plan-repair-session";
import { PlanRepairTimelineGroup } from "./PlanRepairTimelineGroup";

export function CodingTimeline({
  nodes,
  activeNodeId,
  selectedNodeId,
  onSelectNode,
  planRepair = null,
}: {
  nodes: CodingTimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
  planRepair?: PlanRepairSessionState | null;
}) {
  const repairNodeIds = new Set(
    planRepair?.timelineNodes.map((node) => node.id) ?? [],
  );
  const parentNodes = nodes.filter((node) => !repairNodeIds.has(node.id));
  const timelineAnchorId = planRepair?.link.return_context.timeline_anchor_id ?? null;
  const hasTimelineAnchor = parentNodes.some((node) => node.id === timelineAnchorId);
  const showRepairAtEnd = Boolean(planRepair) && !hasTimelineAnchor;

  return (
    <nav
      aria-label="Coding Timeline"
      data-testid="coding-timeline"
      className="min-h-0 overflow-auto border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 md:border-b-0 md:border-r"
    >
      {nodes.length === 0 ? (
        <div className="rounded-md border border-[var(--aria-line)] bg-white p-3 text-sm text-[var(--aria-ink-muted)]">
          暂无 Timeline 节点
        </div>
      ) : (
        <div className="space-y-2">
          {parentNodes.map((node) => {
            const Icon = iconForStage(node.stage);
            const active = node.id === activeNodeId;
            const selected = node.id === selectedNodeId;
            const title = titleForStage(node);
            return (
              <div key={node.id} className="space-y-2">
                <button
                  type="button"
                  onClick={() => onSelectNode(node.id)}
                  aria-current={active ? "step" : undefined}
                  className={[
                    "block w-full rounded-md border bg-white px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
                    active || selected
                      ? "border-[var(--aria-primary)] ring-1 ring-[var(--aria-primary)]"
                      : "border-[var(--aria-line)] hover:border-[var(--aria-primary)]",
                  ].join(" ")}
                >
                  <div className="flex min-w-0 items-start gap-2">
                    <Icon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
                    <div className="min-w-0 flex-1">
                      <div className="flex min-w-0 items-center justify-between gap-2">
                        <span className="truncate text-sm font-semibold">{title}</span>
                        <span className="rounded bg-[var(--aria-panel-muted)] px-1.5 py-0.5 text-[11px] text-[var(--aria-ink-muted)]">
                          {node.status}
                        </span>
                      </div>
                      {node.summary ? (
                        <p className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">
                          {node.summary}
                        </p>
                      ) : null}
                    </div>
                  </div>
                </button>
                {planRepair && node.id === timelineAnchorId ? (
                  <PlanRepairTimelineGroup
                    nodes={planRepair.timelineNodes}
                    activeNodeId={activeNodeId}
                    selectedNodeId={selectedNodeId}
                    onSelectNode={onSelectNode}
                  />
                ) : null}
              </div>
            );
          })}
          {planRepair && showRepairAtEnd ? (
            <PlanRepairTimelineGroup
              nodes={planRepair.timelineNodes}
              activeNodeId={activeNodeId}
              selectedNodeId={selectedNodeId}
              onSelectNode={onSelectNode}
            />
          ) : null}
        </div>
      )}
    </nav>
  );
}

function titleForStage(node: CodingTimelineNode) {
  if (node.stage === "internal_pr_review") {
    return "GroupFinalReview";
  }
  return node.title;
}

function iconForStage(stage: CodingExecutionStage): LucideIcon {
  switch (stage) {
    case "worktree_prepare":
      return GitBranch;
    case "coding":
      return Code;
    case "testing":
      return FlaskConical;
    case "code_review":
      return SearchCode;
    case "review_request":
      return GitPullRequest;
    case "internal_pr_review":
      return ShieldCheck;
    case "final_confirm":
      return UserCheck;
    default:
      return Circle;
  }
}
