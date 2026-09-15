// REQ-UI37-16 第 2、3 项的纯投影层：
// - 契约核对表从 PlanProjectionBundle.human_group_projection.contract_flow 派生，
//   满足判定以引擎编译期 missing_capabilities 为准（不前端重算交集）；
// - 缺口跳转目标从既有 gate_prompt / review_verdict 条目的 findings.contract_field 解析；
// - DEF-3 面板模型从 session_state.plan_repair 快照 + 本连接 timeline / turn 只读派生。
import type {
  PlanProjectionBundle,
  PlanRepairSessionSnapshot,
  ProjectionValidationReport,
} from "../api/types";
import type { ChatEntry } from "./chat-entries";
import type {
  HumanGateTurnState,
  TimelineNode,
} from "./workspace-ws-store-types";

export interface ChecklistFindingRef {
  code: string;
  severity: string | null;
  contractRef: string | null;
  capabilityRef: string | null;
  message: string;
}

export interface ContractChecklistRow {
  key: string;
  contractId: string;
  from: string;
  to: string;
  capability: string;
  satisfied: boolean;
  matchedFindings: ChecklistFindingRef[];
}

export interface ContractChecklist {
  rows: ContractChecklistRow[];
  gapCount: number;
  hasData: boolean;
}

export function selectContractChecklist(
  bundle: PlanProjectionBundle | null,
  findings: readonly ChecklistFindingRef[],
): ContractChecklist {
  if (!bundle) {
    return { rows: [], gapCount: 0, hasData: false };
  }
  const rows: ContractChecklistRow[] = [];
  for (const edge of bundle.human_group_projection.contract_flow) {
    const missing = new Set(edge.missing_capabilities);
    for (const capability of edge.required_capabilities) {
      const satisfied = !missing.has(capability);
      rows.push({
        key: `${edge.contract_id}::${capability}`,
        contractId: edge.contract_id,
        from: edge.from,
        to: edge.to,
        capability,
        satisfied,
        // 满足行不携带缺口 finding：validation findings 的 capabilityRef 通配（null）
        // 只允许命中未满足行，否则缺口提示会污染已满足能力（计划审查 M1 裁决）。
        matchedFindings: satisfied
          ? []
          : findings.filter(
              (finding) =>
                finding.contractRef === edge.contract_id &&
                (finding.capabilityRef === null ||
                  finding.capabilityRef === capability),
            ),
      });
    }
  }
  return {
    rows,
    gapCount: rows.filter((row) => !row.satisfied).length,
    hasData: true,
  };
}

export function checklistFindingsFromValidation(
  validation: ProjectionValidationReport | null,
): ChecklistFindingRef[] {
  return (validation?.findings ?? []).map((finding) => ({
    code: finding.code,
    severity: null,
    contractRef: finding.contract_ref,
    capabilityRef: null,
    message: finding.message,
  }));
}

export function checklistFindingsFromPlanRepair(
  snapshot: PlanRepairSessionSnapshot | null,
): ChecklistFindingRef[] {
  const contractFindings =
    snapshot?.validation?.contract_validation.findings ?? [];
  const projectionFindings =
    snapshot?.validation?.projection_validation.findings ?? [];
  return [
    ...contractFindings.map((finding) => ({
      code: finding.code,
      severity: finding.severity,
      contractRef: finding.contract_ref,
      capabilityRef: finding.capability_ref,
      message: finding.message,
    })),
    ...projectionFindings.map((finding) => ({
      code: finding.code,
      severity: null,
      contractRef: finding.contract_ref,
      capabilityRef: null,
      message: finding.message,
    })),
  ];
}

export function selectFindingTargetEntryId(
  chatEntries: readonly ChatEntry[],
  contractId: string,
): string | null {
  for (let index = chatEntries.length - 1; index >= 0; index -= 1) {
    const entry = chatEntries[index];
    if (entry.type !== "gate_prompt" && entry.type !== "review_verdict") {
      continue;
    }
    const findings = (entry.metadata as Record<string, unknown> | undefined)
      ?.findings;
    if (!Array.isArray(findings)) {
      continue;
    }
    const matched = findings.some(
      (finding) =>
        isRecord(finding) && finding["contract_field"] === contractId,
    );
    if (matched) {
      return entry.id;
    }
  }
  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

const PLAN_REPAIR_STAGE_LABELS: Record<
  PlanRepairSessionSnapshot["stage"],
  string
> = {
  triaging: "缺陷分诊",
  authoring_revision: "修订编写中",
  validating_contract: "契约校验中",
  generating_projections: "投影生成中",
  plan_review: "计划评审中",
  awaiting_confirmation: "等待最终确认",
  published: "已出版",
  amendment_conflict: "修订冲突",
  applying_amendment: "应用修订中",
  amendment_apply_failed: "修订应用失败",
  completed: "已完成",
  failed: "已失败",
};

export function planRepairStageLabel(
  stage: PlanRepairSessionSnapshot["stage"] | null,
): string {
  if (stage === null) {
    return "未知阶段";
  }
  return PLAN_REPAIR_STAGE_LABELS[stage] ?? "未知阶段";
}

export interface PlanRepairRound {
  nodeId: string;
  title: string;
  summary: string | null;
  status: TimelineNode["status"];
  startedAt: string | null;
  completedAt: string | null;
}

export interface PlanRepairFeedbackTurn {
  turnId: string;
  commandId: string | null;
  status: string;
  remainingBudget: number;
  openedAt: string;
}

export interface PlanRepairAmendmentSummary {
  id: string;
  previousPlanRevisionId: string;
  newPlanRevisionId: string;
  revisedWorkItemCount: number;
  contractDeltaCount: number;
  supersededCount: number;
  dependencyGraphChangeCount: number;
  resumeTarget: string | null;
}

export interface PlanRepairPanelModel {
  visible: boolean;
  stageLabel: string | null;
  requestStatus: string | null;
  defectClass: string | null;
  reasonCode: string | null;
  triggerFindingId: string | null;
  amendment: PlanRepairAmendmentSummary | null;
  rounds: PlanRepairRound[];
  feedbackTurn: PlanRepairFeedbackTurn | null;
  error: string | null;
}

const EMPTY_PANEL_MODEL: PlanRepairPanelModel = {
  visible: false,
  stageLabel: null,
  requestStatus: null,
  defectClass: null,
  reasonCode: null,
  triggerFindingId: null,
  amendment: null,
  rounds: [],
  feedbackTurn: null,
  error: null,
};

export function selectPlanRepairPanel(
  snapshot: PlanRepairSessionSnapshot | null,
  sessionId: string | null,
  timelineNodes: readonly TimelineNode[],
  humanGateTurn: HumanGateTurnState | null,
): PlanRepairPanelModel {
  if (
    !snapshot ||
    !sessionId ||
    snapshot.link.child_session_id !== sessionId
  ) {
    return EMPTY_PANEL_MODEL;
  }
  return {
    visible: true,
    stageLabel: planRepairStageLabel(snapshot.stage),
    requestStatus: snapshot.request.status,
    defectClass: snapshot.request.defect_class,
    reasonCode: snapshot.request.reason_code,
    triggerFindingId: snapshot.request.trigger_finding_id,
    amendment: amendmentSummary(snapshot.amendment),
    rounds: timelineNodes.map((node) => ({
      nodeId: node.node_id,
      title: node.title,
      summary: node.summary ?? null,
      status: node.status,
      startedAt: node.started_at ?? null,
      completedAt: node.completed_at ?? null,
    })),
    feedbackTurn: humanGateTurn
      ? {
          turnId: humanGateTurn.turn_id,
          commandId: humanGateTurn.command_id,
          status: humanGateTurn.status,
          remainingBudget: humanGateTurn.remaining_budget,
          openedAt: humanGateTurn.opened_at,
        }
      : null,
    error: snapshot.error,
  };
}

function amendmentSummary(
  amendment: PlanRepairSessionSnapshot["amendment"],
): PlanRepairAmendmentSummary | null {
  if (!amendment) {
    return null;
  }
  return {
    id: amendment.id,
    previousPlanRevisionId: amendment.previous_plan_revision_id,
    newPlanRevisionId: amendment.new_plan_revision_id,
    revisedWorkItemCount: Object.keys(amendment.revised_work_items).length,
    contractDeltaCount: amendment.contract_deltas.length,
    supersededCount: amendment.superseded_revisions.length,
    dependencyGraphChangeCount: amendment.dependency_graph_changes.length,
    resumeTarget: amendment.resume_target
      ? `${amendment.resume_target.logical_work_item_id} · ${amendment.resume_target.mode}`
      : null,
  };
}
