import { Check, GitBranch, Play, Square, TriangleAlert, X } from "lucide-react";
import type { ReactNode } from "react";
import type { RevisionPath } from "../../api/types";

interface StageActionsBarProps {
  stage: string;
  onStartGeneration?: () => void;
  onAbort?: () => void;
  onConfirm?: () => void;
  onRequestChange?: () => void;
  onTerminate?: () => void;
  onSelectRevisionPath?: (path: RevisionPath) => void;
}

export function StageActionsBar({
  stage,
  onStartGeneration,
  onAbort,
  onConfirm,
  onRequestChange,
  onTerminate,
  onSelectRevisionPath,
}: StageActionsBarProps) {
  return (
    <div
      data-testid="stage-actions-bar"
      className="flex min-h-12 flex-wrap items-center justify-end gap-2 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2"
    >
      {stage === "prepare_context" && onStartGeneration ? (
        <ActionButton variant="primary" onClick={onStartGeneration} icon={<Play className="h-4 w-4" />}>
          开始生成
        </ActionButton>
      ) : null}

      {(stage === "running" || stage === "cross_review" || stage === "revision") && onAbort ? (
        <ActionButton variant="danger" onClick={onAbort} icon={<Square className="h-4 w-4" />}>
          中止
        </ActionButton>
      ) : null}

      {stage === "review_decision" && onSelectRevisionPath ? (
        <ActionButton
          variant="secondary"
          onClick={() => onSelectRevisionPath("revise")}
          icon={<GitBranch className="h-4 w-4" />}
        >
          选择修订路径
        </ActionButton>
      ) : null}
      {stage === "review_decision" && onAbort ? (
        <ActionButton variant="danger" onClick={onAbort} icon={<Square className="h-4 w-4" />}>
          中止
        </ActionButton>
      ) : null}

      {stage === "human_confirm" && onConfirm ? (
        <ActionButton variant="success" onClick={onConfirm} icon={<Check className="h-4 w-4" />}>
          确认
        </ActionButton>
      ) : null}
      {stage === "human_confirm" && onRequestChange ? (
        <ActionButton
          variant="warning"
          onClick={onRequestChange}
          icon={<TriangleAlert className="h-4 w-4" />}
        >
          要求修改
        </ActionButton>
      ) : null}
      {stage === "human_confirm" && onTerminate ? (
        <ActionButton variant="danger" onClick={onTerminate} icon={<X className="h-4 w-4" />}>
          终止
        </ActionButton>
      ) : null}
    </div>
  );
}

function ActionButton({
  variant,
  onClick,
  icon,
  children,
}: {
  variant: "primary" | "secondary" | "success" | "warning" | "danger";
  onClick: () => void;
  icon: ReactNode;
  children: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`inline-flex h-8 items-center justify-center gap-2 rounded-md border px-3 text-sm font-semibold ${variantClass(
        variant,
      )}`}
    >
      {icon}
      {children}
    </button>
  );
}

function variantClass(variant: "primary" | "secondary" | "success" | "warning" | "danger") {
  if (variant === "primary") {
    return "border-[var(--aria-primary)] bg-[var(--aria-primary)] text-white";
  }
  if (variant === "success") {
    return "border-emerald-600 bg-emerald-600 text-white";
  }
  if (variant === "warning") {
    return "border-amber-200 bg-amber-50 text-amber-800";
  }
  if (variant === "danger") {
    return "border-red-200 bg-red-50 text-red-700";
  }
  return "border-[var(--aria-line)] bg-white text-[var(--aria-ink)]";
}
