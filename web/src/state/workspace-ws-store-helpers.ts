import type {
  PlanProjectionBundle,
  ProjectionValidationReport,
  WorkItemProjectionBundle,
  WorkItemPlanArtifactPayload,
  WorkItemPlanArtifactVersion,
  WorkItemPlanCandidateDto,
  WorkItemRevisionHistoryDto,
  WorkspaceProviderName,
} from "../api/types";
import type { ChatEntryRole } from "./chat-entries";
import type {
  ArtifactVersion,
  ArtifactVersionSummary,
  ExecutionEvent,
  TimelineNode,
  TimelineNodeDetail,
  WorkspaceArtifact,
  WorkItemPlanProjectionArtifacts,
} from "./workspace-ws-store-types";

const STAGE_ORDER = [
  "prepare_context",
  "running",
  "author_confirm",
  "cross_review",
  "human_confirm",
  "completed",
];
export const STREAMING_STAGES = new Set(["running", "cross_review", "revision"]);

export function visitedStagesFor(stage: string) {
  const index = STAGE_ORDER.indexOf(flowStageFor(stage));
  if (index === -1) {
    return [stage];
  }
  return STAGE_ORDER.slice(0, index + 1);
}

export function mergeVisitedStages(current: string[], stage: string) {
  return Array.from(new Set([...current, ...visitedStagesFor(stage)]));
}

function flowStageFor(stage: string) {
  if (stage === "review_decision" || stage === "revision") {
    return "cross_review";
  }
  return stage;
}

export function detailsForTimelineNodes(nodes: TimelineNode[], sessionId: string) {
  return nodes.reduce<Record<string, TimelineNodeDetail>>((details, node) => {
    details[node.node_id] = emptyNodeDetail(node.node_id, { sessionId, node });
    return details;
  }, {});
}

export function emptyNodeDetail(
  nodeId: string,
  options: { sessionId?: string | null; node?: TimelineNode } = {},
): TimelineNodeDetail {
  const node = options.node;
  return {
    node_id: nodeId,
    session_id: options.sessionId ?? "",
    node_type: node?.node_type ?? "author_run",
    status: node?.status ?? "active",
    agent_role: agentRoleFor(node),
    provider: node?.agent ? { name: node.agent, model: "" } : null,
    prompt: null,
    messages: [],
    streaming_content: "",
    execution_events: [],
    permission_events: [],
    verdict: null,
    artifact_ref: null,
    is_revision: node?.node_type === "revision",
    base_artifact_ref: null,
    started_at: node?.started_at ?? "",
    ended_at: node?.completed_at ?? null,
  };
}

export function normalizeWorkspaceArtifact(
  artifact: WorkspaceArtifact,
): {
  artifactMarkdown: string | null;
  workItemPlanCandidate: WorkItemPlanCandidateDto | null;
  workItemPlanArtifact: WorkItemPlanArtifactPayload | null;
} {
  if (artifact === null) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: null,
      workItemPlanArtifact: null,
    };
  }
  if (typeof artifact === "object" && "candidate" in artifact) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: artifact.candidate,
      workItemPlanArtifact: null,
    };
  }
  if (typeof artifact === "object" && "outline_candidate" in artifact) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: null,
      workItemPlanArtifact: {
        type: "outline_candidate",
        payload: artifact.outline_candidate,
      },
    };
  }
  if (typeof artifact === "object" && "context_blocker" in artifact) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: null,
      workItemPlanArtifact: {
        type: "context_blocker",
        payload: artifact.context_blocker,
      },
    };
  }
  if (typeof artifact === "object" && "draft_candidate" in artifact) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: null,
      workItemPlanArtifact: {
        type: "draft_candidate",
        payload: artifact.draft_candidate,
      },
    };
  }
  if (typeof artifact === "object" && "batch_state" in artifact) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: null,
      workItemPlanArtifact: {
        type: "batch_state",
        payload: artifact.batch_state,
      },
    };
  }
  if (typeof artifact === "object" && "compile_report" in artifact) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: null,
      workItemPlanArtifact: {
        type: "compile_report",
        payload: artifact.compile_report,
      },
    };
  }
  const projectionArtifact = workItemPlanProjectionArtifactFromRecord(artifact);
  if (projectionArtifact) {
    return {
      artifactMarkdown: null,
      workItemPlanCandidate: null,
      workItemPlanArtifact: projectionArtifact,
    };
  }
  if (typeof artifact === "object" && "markdown" in artifact) {
    return {
      artifactMarkdown: artifact.markdown,
      workItemPlanCandidate: null,
      workItemPlanArtifact: null,
    };
  }
  return {
    artifactMarkdown: typeof artifact === "string" ? artifact : null,
    workItemPlanCandidate: null,
    workItemPlanArtifact: null,
  };
}

export function workItemPlanVersionsFromSession(
  versions: ArtifactVersionSummary[],
  fullVersions: ArtifactVersion[],
  currentArtifact: WorkItemPlanArtifactPayload | null,
  activeNodeId: string | null,
  authorProvider: WorkspaceProviderName,
  reviewerProvider: WorkspaceProviderName | null,
): WorkItemPlanArtifactVersion[] {
  if (versions.length === 0 && fullVersions.length === 0) {
    return currentArtifact
      ? [
          {
            version: 0,
            generated_by: authorProvider,
            reviewed_by: reviewerProvider,
            review_verdict: null,
            confirmed_by: null,
            is_current: true,
            created_at: new Date().toISOString(),
            source_node_id: activeNodeId ?? "",
            artifact: currentArtifact,
          },
        ]
      : [];
  }

  const summariesByVersion = new Map(
    versions.map((version) => [version.version, version]),
  );
  const fullVersionsByVersion = new Map(
    fullVersions.map((version) => [version.version, version]),
  );
  const versionNumbers = Array.from(
    new Set([...summariesByVersion.keys(), ...fullVersionsByVersion.keys()]),
  );
  const currentVersion =
    versions.find((version) => version.is_current)?.version ??
    fullVersions.find((version) => version.is_current)?.version ??
    Math.max(...versionNumbers);

  const normalizedVersions: WorkItemPlanArtifactVersion[] = [];
  for (const versionNumber of versionNumbers) {
    const summary = summariesByVersion.get(versionNumber);
    const fullVersion = fullVersionsByVersion.get(versionNumber);
    const metadata = summary ?? fullVersion;
    if (!metadata) {
      continue;
    }
    normalizedVersions.push({
      version: metadata.version,
      generated_by: metadata.generated_by,
      reviewed_by: metadata.reviewed_by ?? null,
      review_verdict: metadata.review_verdict ?? null,
      confirmed_by: metadata.confirmed_by ?? null,
      is_current: metadata.is_current,
      created_at: metadata.created_at,
      source_node_id: metadata.source_node_id,
      artifact:
        workItemPlanProjectionArtifactFromRecord(fullVersion) ??
        (currentArtifact && versionNumber === currentVersion
          ? currentArtifact
          : null),
    });
  }
  return normalizedVersions.sort((left, right) => left.version - right.version);
}

export function workItemPlanProjectionArtifactsFromVersions(
  versions: WorkItemPlanArtifactVersion[],
  selectedPlanProjectionId?: string,
): WorkItemPlanProjectionArtifacts {
  const orderedVersions = [...versions].sort(
    (left, right) => left.version - right.version,
  );
  const planVersions = orderedVersions.filter(
    (version) => version.artifact?.type === "plan_projection",
  );
  const selectedPlanVersion = selectedPlanProjectionId
    ? planVersions.find(
        (version) =>
          version.artifact?.type === "plan_projection" &&
          version.artifact.payload.id === selectedPlanProjectionId,
      )
    : [...planVersions].sort(
        (left, right) =>
          Number(Boolean(right.is_current)) -
            Number(Boolean(left.is_current)) ||
          right.version - left.version,
      )[0];
  const planProjection =
    selectedPlanVersion?.artifact?.type === "plan_projection"
      ? selectedPlanVersion.artifact.payload
      : null;
  const sourceNodeId = selectedPlanVersion?.source_node_id ?? null;
  let history: WorkItemRevisionHistoryDto | null = null;
  let validation: ProjectionValidationReport | null = null;
  const workItemProjectionById = new Map<string, WorkItemProjectionBundle>();

  for (const version of orderedVersions) {
    if (sourceNodeId && version.source_node_id !== sourceNodeId) {
      continue;
    }
    switch (version.artifact?.type) {
      case "work_item_projection":
        workItemProjectionById.set(
          version.artifact.payload.id,
          version.artifact.payload,
        );
        break;
      case "work_item_revision_history":
        history = version.artifact.payload;
        break;
      case "projection_validation":
        validation = version.artifact.payload;
        break;
      default:
        break;
    }
  }

  const workItemProjections: WorkItemProjectionBundle[] = [];
  const missingWorkItemProjectionRefs: string[] = [];
  for (const projectionRef of planProjection?.work_item_projection_bundle_refs ?? []) {
    const projection = workItemProjectionById.get(projectionRef);
    if (projection) {
      workItemProjections.push(projection);
    } else {
      missingWorkItemProjectionRefs.push(projectionRef);
    }
  }

  return {
    planProjection,
    workItemProjections,
    history,
    validation,
    missingWorkItemProjectionRefs,
  };
}

export function emptyWorkItemPlanProjectionArtifacts(): WorkItemPlanProjectionArtifacts {
  return {
    planProjection: null,
    workItemProjections: [],
    history: null,
    validation: null,
    missingWorkItemProjectionRefs: [],
  };
}

export function upsertArtifactVersionSummary(
  versions: ArtifactVersionSummary[],
  version: number,
  replaceCurrent: boolean,
  fallback: {
    author: WorkspaceProviderName;
    activeNodeId: string;
    createdAt: string;
  },
): ArtifactVersionSummary[] {
  const existing = versions.find((item) => item.version === version);
  return [
    ...versions
      .filter((item) => item.version !== version)
      .map((item) => (replaceCurrent ? { ...item, is_current: false } : item)),
    {
      version,
      generated_by: existing?.generated_by ?? fallback.author,
      reviewed_by: existing?.reviewed_by ?? null,
      review_verdict: existing?.review_verdict ?? null,
      confirmed_by: existing?.confirmed_by ?? null,
      is_current: replaceCurrent ? true : existing?.is_current ?? false,
      created_at: existing?.created_at ?? fallback.createdAt,
      source_node_id: existing?.source_node_id ?? fallback.activeNodeId,
    },
  ].sort((left, right) => left.version - right.version);
}

export function upsertWorkItemPlanArtifactVersion(
  versions: WorkItemPlanArtifactVersion[],
  artifact: WorkItemPlanArtifactPayload,
  version: number,
  replaceCurrent: boolean,
  fallback: {
    author: WorkspaceProviderName;
    reviewer: WorkspaceProviderName | null;
    activeNodeId: string;
    createdAt: string;
  },
): WorkItemPlanArtifactVersion[] {
  const existing = versions.find((item) => item.version === version);
  return [
    ...versions
      .filter((item) => item.version !== version)
      .map((item) => (replaceCurrent ? { ...item, is_current: false } : item)),
    {
      version,
      generated_by: existing?.generated_by ?? fallback.author,
      reviewed_by: existing?.reviewed_by ?? fallback.reviewer,
      review_verdict: existing?.review_verdict ?? null,
      confirmed_by: existing?.confirmed_by ?? null,
      is_current: replaceCurrent ? true : existing?.is_current ?? false,
      created_at: existing?.created_at ?? fallback.createdAt,
      source_node_id: existing?.source_node_id ?? fallback.activeNodeId,
      artifact,
    },
  ].sort((left, right) => left.version - right.version);
}

export function workItemPlanProjectionArtifactFromRecord(
  value: unknown,
): WorkItemPlanArtifactPayload | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  if ("plan_projection" in value) {
    return {
      type: "plan_projection",
      payload: value.plan_projection as PlanProjectionBundle,
    };
  }
  if ("work_item_projection" in value) {
    return {
      type: "work_item_projection",
      payload: value.work_item_projection as WorkItemProjectionBundle,
    };
  }
  if ("work_item_revision_history" in value) {
    return {
      type: "work_item_revision_history",
      payload: value.work_item_revision_history as WorkItemRevisionHistoryDto,
    };
  }
  if ("projection_validation" in value) {
    return {
      type: "projection_validation",
      payload: value.projection_validation as ProjectionValidationReport,
    };
  }
  return null;
}

export function ensureNodeDetail(details: Record<string, TimelineNodeDetail>, nodeId: string) {
  const existing = details[nodeId];
  details[nodeId] = existing
    ? {
        ...existing,
        messages: [...existing.messages],
        execution_events: [...existing.execution_events],
        permission_events: [...existing.permission_events],
      }
    : emptyNodeDetail(nodeId);
  return details[nodeId];
}

function agentRoleFor(node?: TimelineNode): "author" | "reviewer" | null {
  if (
    node?.node_type === "author_run" ||
    node?.node_type === "revision" ||
    node?.node_type === "work_item_plan_outline_run" ||
    node?.node_type === "work_item_draft_run" ||
    node?.node_type === "work_item_batch_run"
  ) {
    return "author";
  }
  if (
    node?.node_type === "reviewer_run" ||
    node?.node_type === "work_item_plan_outline_review" ||
    node?.node_type === "work_item_draft_review" ||
    node?.node_type === "work_item_batch_review"
  ) {
    return "reviewer";
  }
  return null;
}

export function chatRoleForTimelineNode(node?: TimelineNode): ChatEntryRole | null {
  return agentRoleFor(node);
}

export function upsertEvent(events: ExecutionEvent[], event: ExecutionEvent) {
  const index = events.findIndex((existing) => existing.event_id === event.event_id);
  if (index === -1) {
    return [...events, event];
  }
  const next = [...events];
  next[index] = { ...next[index], ...event };
  return next;
}

export function normalizeTimelineNodeDetails(details: Record<string, TimelineNodeDetail>) {
  return Object.fromEntries(
    Object.entries(details).map(([nodeId, detail]) => [
      nodeId,
      {
        ...detail,
        execution_events: deduplicateExecutionEvents(detail.execution_events),
      },
    ]),
  );
}

function deduplicateExecutionEvents(events: ExecutionEvent[]) {
  return events.reduce<ExecutionEvent[]>((deduped, event) => {
    const index = deduped.findIndex((existing) => existing.event_id === event.event_id);
    if (index === -1) {
      deduped.push(event);
    } else {
      deduped[index] = { ...deduped[index], ...event };
    }
    return deduped;
  }, []);
}
