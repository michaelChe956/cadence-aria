import { Check, GitBranch, Layers, Pause, Play, RefreshCw, RotateCcw, UserRound } from "lucide-react";
import type { ReactElement, ReactNode } from "react";
import type {
  WorkItemBatchDecision,
  WorkItemDraftDecision,
  WorkItemGenerationMode,
  WorkItemPlanArtifactPayload,
  WorkItemPlanCompileRecoveryAction,
} from "../../api/types";

export interface WorkItemPlanStagedPanelProps {
  activeNodeType: string | null;
  artifact: WorkItemPlanArtifactPayload | null;
  onAcceptOutline: () => void;
  onSelectMode: (mode: WorkItemGenerationMode) => void;
  onRequestOutlineRevision: () => void;
  onDraftDecision: (outlineId: string, decision: WorkItemDraftDecision) => void;
  onBatchDecision: (
    decision: WorkItemBatchDecision,
    feedback?: string,
    firstAffectedOutlineId?: string,
  ) => void;
  onCompileRecoveryAction: (action: WorkItemPlanCompileRecoveryAction) => void;
}

export function WorkItemPlanStagedPanel({
  activeNodeType,
  artifact,
  onAcceptOutline,
  onSelectMode,
  onRequestOutlineRevision,
  onDraftDecision,
  onBatchDecision,
  onCompileRecoveryAction,
}: WorkItemPlanStagedPanelProps) {
  if (!activeNodeType) {
    return null;
  }

  if (activeNodeType === "work_item_plan_outline_confirm") {
    return (
      <PanelShell title="Outline 确认" testId="work-item-plan-staged-panel">
        <ActionButton icon={<Check />} onClick={onAcceptOutline}>
          接受 Outline
        </ActionButton>
        <ActionButton icon={<RefreshCw />} onClick={onRequestOutlineRevision}>
          重写 Outline
        </ActionButton>
      </PanelShell>
    );
  }

  if (activeNodeType === "work_item_generation_mode") {
    return (
      <PanelShell title="生成模式" testId="work-item-plan-staged-panel">
        <ActionButton icon={<GitBranch />} onClick={() => onSelectMode("serial")}>
          逐个生成
        </ActionButton>
        <ActionButton icon={<Layers />} onClick={() => onSelectMode("batch")}>
          自动生成
        </ActionButton>
        <ActionButton icon={<RefreshCw />} onClick={onRequestOutlineRevision}>
          返回 Outline 返修
        </ActionButton>
      </PanelShell>
    );
  }

  if (activeNodeType === "work_item_draft_confirm") {
    const draftPayload = artifact?.type === "draft_candidate" ? artifact.payload : null;
    const outlineId = draftPayload?.draft_record.outline_id ?? "";
    return (
      <PanelShell title="Draft 确认" testId="work-item-plan-staged-panel">
        {draftPayload?.can_accept ? (
          <ActionButton icon={<Check />} onClick={() => onDraftDecision(outlineId, "accept")}>
            接受
          </ActionButton>
        ) : null}
        <ActionButton icon={<RefreshCw />} onClick={() => onDraftDecision(outlineId, "rewrite")}>
          重写
        </ActionButton>
        <ActionButton icon={<Pause />} onClick={() => onDraftDecision(outlineId, "pause")}>
          暂停
        </ActionButton>
      </PanelShell>
    );
  }

  if (activeNodeType === "work_item_batch_confirm") {
    const batchPayload = artifact?.type === "batch_state" ? artifact.payload : null;
    const firstAffectedOutlineId = batchPayload?.failure_summary[0]?.outline_id;
    return (
      <PanelShell title="Batch 确认" testId="work-item-plan-staged-panel">
        <ActionButton icon={<Check />} onClick={() => onBatchDecision("accept_all")}>
          接受全部
        </ActionButton>
        <ActionButton icon={<RefreshCw />} onClick={() => onBatchDecision("rewrite_batch")}>
          整组重写
        </ActionButton>
        <ActionButton icon={<Pause />} onClick={() => onBatchDecision("pause")}>
          暂停
        </ActionButton>
        {firstAffectedOutlineId ? (
          <ActionButton
            icon={<GitBranch />}
            onClick={() =>
              onBatchDecision("downgrade_to_serial", undefined, firstAffectedOutlineId)
            }
          >
            降级串行
          </ActionButton>
        ) : null}
      </PanelShell>
    );
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
