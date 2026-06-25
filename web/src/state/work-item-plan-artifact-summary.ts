import type { WorkItemPlanArtifactPayload } from "../api/types";

export interface WorkItemPlanArtifactUpdateSummary {
  content: string;
  metadata: Record<string, unknown>;
}

export function workItemPlanArtifactUpdateSummary(
  artifact: WorkItemPlanArtifactPayload,
  version: number,
): WorkItemPlanArtifactUpdateSummary {
  const versionLabel = `内部版本 v${version}`;

  if (artifact.type === "outline_candidate") {
    const outline = artifact.payload.outline;
    const items = outline.work_item_outlines ?? outline.work_items ?? [];
    const round = artifact.payload.current_generation_round_id ?? outline.id ?? "未命名 round";
    return {
      content: `Outline 已更新 · ${round} · ${items.length} items`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Outline",
        object_id: round,
        status_label: outline.status ?? null,
      },
    };
  }

  if (artifact.type === "draft_candidate") {
    const record = artifact.payload.draft_record;
    return {
      content: `Draft 已更新 · ${record.outline_id} · ${record.draft_id}`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Draft",
        object_id: record.outline_id,
        object_title: record.candidate.title,
        draft_id: record.draft_id,
        status_label: record.status,
      },
    };
  }

  if (artifact.type === "batch_state") {
    return {
      content: `Batch Draft 已更新 · ${artifact.payload.batch_status}`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Batch Draft",
        object_id: artifact.payload.batch_id,
        status_label: artifact.payload.batch_status,
      },
    };
  }

  return {
    content: `Compile Report 已更新 · ${artifact.payload.status}`,
    metadata: {
      version,
      version_label: versionLabel,
      artifact_type: artifact.type,
      artifact_label: "Compile Report",
      object_id: artifact.payload.compile_id,
      status_label: artifact.payload.status,
    },
  };
}
