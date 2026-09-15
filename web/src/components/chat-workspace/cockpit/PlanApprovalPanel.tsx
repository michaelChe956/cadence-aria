// web/src/components/chat-workspace/cockpit/PlanApprovalPanel.tsx
// 计划审批视图容器：轮次差异 / 契约核对 / SC 子会话（DEF-3）三个只读子视图。
// 数据源选择：plan-repair 子会话快照优先（子会话场景），退回 plan projection artifacts。
// H2（计划双审裁决）：观测态可能缺少 projection/versions 字段——可选链 + 空默认，不白屏。
import { useMemo, useState } from "react";
import {
  checklistFindingsFromPlanRepair,
  checklistFindingsFromValidation,
  selectContractChecklist,
  selectFindingTargetEntryId,
  selectPlanRepairPanel,
} from "../../../state/plan-approval-projection";
import type { WorkspaceWsState } from "../../../state/workspace-ws-store-types";
import { ContractChecklistView } from "./ContractChecklistView";
import { PlanRepairReadOnlyPanel } from "./PlanRepairReadOnlyPanel";
import { RevisionDiffView } from "./RevisionDiffView";

export interface PlanApprovalPanelProps {
  sessionId: string | null;
  state: WorkspaceWsState;
  onJumpToEntry: (entryId: string) => void;
  artifactContentCache: Readonly<Record<number, string>>; // 页面层经 numericContentCacheValues 转换
  loadVersionMarkdown: ((version: number) => Promise<string>) | null;
  onCacheVersionMarkdown: ((version: number, markdown: string) => void) | null;
}

type ApprovalTab = "diff" | "checklist" | "repair";

export function PlanApprovalPanel({
  sessionId,
  state,
  onJumpToEntry,
  artifactContentCache,
  loadVersionMarkdown,
  onCacheVersionMarkdown,
}: PlanApprovalPanelProps) {
  const [activeTab, setActiveTab] = useState<ApprovalTab>("diff");

  const repairBundle = state.planRepair?.projection ?? null;
  const bundle = repairBundle ?? state.workItemPlanProjectionArtifacts?.planProjection ?? null;
  const validation = state.workItemPlanProjectionArtifacts?.validation ?? null;
  const findings = useMemo(
    () => [
      ...checklistFindingsFromValidation(validation),
      ...checklistFindingsFromPlanRepair(state.planRepair),
    ],
    [validation, state.planRepair],
  );
  const checklist = useMemo(
    () => selectContractChecklist(bundle, findings),
    [bundle, findings],
  );
  const repairModel = useMemo(
    () =>
      selectPlanRepairPanel(
        state.planRepair,
        sessionId,
        state.timelineNodes,
        state.humanGateTurn,
      ),
    [state.planRepair, sessionId, state.timelineNodes, state.humanGateTurn],
  );

  return (
    <div
      data-testid="cockpit-plan-approval-panel"
      className="flex min-h-0 flex-1 flex-col gap-2 overflow-hidden p-2"
    >
      <div className="flex flex-wrap items-center gap-1" role="tablist" aria-label="计划审批视图">
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "diff"}
          data-testid="plan-approval-tab-diff"
          onClick={() => setActiveTab("diff")}
          className="inline-flex min-h-11 items-center rounded-md px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          轮次差异
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "checklist"}
          data-testid="plan-approval-tab-checklist"
          onClick={() => setActiveTab("checklist")}
          className="inline-flex min-h-11 items-center gap-1 rounded-md px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          契约核对
          {checklist.gapCount > 0 ? (
            <span
              data-testid="plan-approval-gap-badge"
              className="aria-chip aria-num border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)] text-[var(--aria-topo-node-failed-fg)]"
            >
              缺口 {checklist.gapCount}
            </span>
          ) : null}
        </button>
        {repairModel.visible ? (
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "repair"}
            data-testid="plan-approval-tab-repair"
            onClick={() => setActiveTab("repair")}
            className="inline-flex min-h-11 items-center rounded-md px-3 text-xs font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            子会话
          </button>
        ) : null}
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {activeTab === "diff" ? (
          <RevisionDiffView
            sessionId={sessionId}
            versions={state.artifactVersions ?? []}
            contentCache={artifactContentCache}
            loadVersionMarkdown={loadVersionMarkdown}
            onCacheVersionMarkdown={onCacheVersionMarkdown}
          />
        ) : activeTab === "checklist" ? (
          <ContractChecklistView
            checklist={checklist}
            findingTargetFor={(row) =>
              selectFindingTargetEntryId(state.chatEntries, row.contractId)
            }
            onJumpToFinding={onJumpToEntry}
          />
        ) : (
          <PlanRepairReadOnlyPanel model={repairModel} />
        )}
      </div>
    </div>
  );
}
