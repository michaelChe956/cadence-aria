import { Check, GitBranch, Layers, Pause, Play, RefreshCw, RotateCcw, UserRound } from "lucide-react";
import type { ReactElement, ReactNode } from "react";
import type {
  WorkItemPlanArtifactPayload,
  WorkItemPlanCompileRecoveryAction,
} from "../../api/types";
import { DraftValidationFailureNotice } from "./DraftValidationFailureNotice";

export interface WorkItemPlanStagedPanelProps {
  activeNodeType: string | null;
  artifact: WorkItemPlanArtifactPayload | null;
  // L2 退役（T5/REQ-RET-02）：legacy 逐段决策回调（outline 确认/生成模式/outline
  // 返修/draft/batch）随消息族删除——各分支退役；compile recovery 为 SC compile
  // 链保留面，回调保持必选。
  onCompileRecoveryAction: (action: WorkItemPlanCompileRecoveryAction) => void;
}

export function WorkItemPlanStagedPanel({
  activeNodeType,
  artifact,
  onCompileRecoveryAction,
}: WorkItemPlanStagedPanelProps) {
  if (!activeNodeType) {
    return null;
  }

  if (activeNodeType === "work_item_plan_compile_recovery") {
    const compileReport = artifact?.type === "compile_report" ? artifact.payload : null;
    const rollbackAllowed = compileReport?.plan_commit_state !== "committed";
    return (
      <PanelShell title="Compile Recovery" testId="work-item-plan-staged-panel">
        <ActionButton icon={<Play />} onClick={() => onCompileRecoveryAction("continue")}>
          继续
        </ActionButton>
        {rollbackAllowed ? (
          <ActionButton
            icon={<RotateCcw />}
            onClick={() => onCompileRecoveryAction("abort_and_rollback")}
          >
            放弃并回滚
          </ActionButton>
        ) : null}
        <ActionButton icon={<UserRound />} onClick={() => onCompileRecoveryAction("human_triage")}>
          转人工
        </ActionButton>
      </PanelShell>
    );
  }

  if (activeNodeType.startsWith("work_item_")) {
    return (
      <div
        data-testid="work-item-plan-staged-panel"
        className="border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-3 text-sm text-[var(--aria-ink-muted)]"
      >
        系统处理中... <span className="font-mono text-xs">{activeNodeType}</span>
      </div>
    );
  }

  return null;
}

function PanelShell({
  title,
  testId,
  children,
}: {
  title: string;
  testId: string;
  children: ReactNode;
}) {
  return (
    <div
      data-testid={testId}
      className="flex min-w-0 flex-wrap items-center gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-3"
    >
      <span className="mr-1 text-xs font-semibold text-[var(--aria-ink-muted)]">{title}</span>
      {children}
    </div>
  );
}

function ActionButton({
  icon,
  onClick,
  children,
}: {
  icon: ReactElement;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="inline-flex h-8 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-2.5 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel)]"
    >
      {icon}
      <span>{children}</span>
    </button>
  );
}
