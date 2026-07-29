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
        object_title: record.candidate.canonical_contract_candidate.identity.title,
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

  if (artifact.type === "context_blocker") {
    const firstBlocker = artifact.payload.context_blockers[0];
    const blockerCount = artifact.payload.context_blockers.length;
    return {
      content: `Context Blocker 已更新 · ${blockerCount} blockers`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Context Blocker",
        object_id: firstBlocker?.code ?? "context_blocker",
        status_label: "blocked",
      },
    };
  }

  if (artifact.type === "compile_report") {
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

  if (artifact.type === "plan_projection") {
    return {
      content: `Plan Projection 已发布 · ${artifact.payload.plan_revision_id}`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Plan Projection",
        object_id: artifact.payload.id,
        status_label: "published",
      },
    };
  }

  if (artifact.type === "work_item_projection") {
    return {
      content: `Work Item Projection 已发布 · ${artifact.payload.work_item_revision_id}`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Work Item Projection",
        object_id: artifact.payload.id,
        status_label: "published",
      },
    };
  }

  if (artifact.type === "work_item_revision_history") {
    return {
      content: `Revision History 已发布 · ${artifact.payload.entries.length} entries`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Revision History",
        object_id: "work_item_revision_history",
        status_label: "published",
      },
    };
  }

  return {
    content: `Projection Validation 已发布 · ${artifact.payload.findings.length} findings`,
    metadata: {
      version,
      version_label: versionLabel,
      artifact_type: artifact.type,
      artifact_label: "Projection Validation",
      object_id: "projection_validation",
      status_label: artifact.payload.findings.length === 0 ? "valid" : "invalid",
    },
  };
}
