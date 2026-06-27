import type {
  WorkItemPlanArtifactPayload,
  WorkItemPlanArtifactVersion,
  WorkItemPlanCandidateDto,
  WorkspaceProviderName,
} from "../api/types";
import type { ChatEntryRole } from "./chat-entries";
import type {
  ArtifactVersionSummary,
  ExecutionEvent,
  TimelineNode,
  TimelineNodeDetail,
  WorkspaceArtifact,
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
  if (typeof artifact === "object" && "markdown" in artifact) {
    return {
      artifactMarkdown: artifact.markdown,
      workItemPlanCandidate: null,
      workItemPlanArtifact: null,
    };
  }
  return {
    artifactMarkdown: artifact,
    workItemPlanCandidate: null,
    workItemPlanArtifact: null,
  };
}

export function workItemPlanVersionsFromSession(
  versions: ArtifactVersionSummary[],
  currentArtifact: WorkItemPlanArtifactPayload | null,
  activeNodeId: string | null,
  authorProvider: WorkspaceProviderName,
  reviewerProvider: WorkspaceProviderName | null,
): WorkItemPlanArtifactVersion[] {
  if (versions.length === 0) {
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

  const currentVersion =
    versions.find((version) => version.is_current)?.version ??
    Math.max(...versions.map((version) => version.version));

  return versions
    .map((version) => ({
      ...version,
      artifact:
        currentArtifact && version.version === currentVersion
          ? currentArtifact
          : null,
    }))
    .sort((left, right) => left.version - right.version);
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
