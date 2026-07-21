import type { WorkItemPlanArtifactPayload } from "../../api/types";

export function projectionArtifactVersionGroup(
  artifact: WorkItemPlanArtifactPayload | null,
): { key: string; label: string } | null {
  switch (artifact?.type) {
    case "plan_projection":
      return { key: "plan-projection", label: "Plan Projection" };
    case "work_item_projection":
      return { key: "work-item-projection", label: "Work Item Projection" };
    case "work_item_revision_history":
      return { key: "revision-history", label: "Revision History" };
    case "projection_validation":
      return { key: "projection-validation", label: "Projection Validation" };
    default:
      return null;
  }
}

export function workItemPlanArtifactLabel(
  artifact: WorkItemPlanArtifactPayload,
): string {
  switch (artifact.type) {
    case "outline_candidate": {
      const outline = artifact.payload.outline;
      const itemCount = (outline.work_item_outlines ?? outline.work_items ?? []).length;
      return `Outline · ${itemCount} items`;
    }
    case "draft_candidate":
      return `${artifact.payload.draft_record.outline_id} / ${artifact.payload.draft_record.draft_id}`;
    case "batch_state":
      return `Batch / ${artifact.payload.batch_id}`;
    case "compile_report":
      return `Final Compile / ${artifact.payload.compile_id}`;
    case "context_blocker":
      return `Blocker / ${artifact.payload.context_blockers.length}`;
    case "plan_projection":
      return `Plan Projection / ${artifact.payload.plan_revision_id}`;
    case "work_item_projection":
      return `Work Item Projection / ${artifact.payload.work_item_revision_id}`;
    case "work_item_revision_history":
      return `Revision History / ${artifact.payload.entries.length}`;
    case "projection_validation":
      return `Projection Validation / ${artifact.payload.findings.length}`;
  }
}
