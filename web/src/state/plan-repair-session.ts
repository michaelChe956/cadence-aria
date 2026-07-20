import type {
  CodingTimelineNode,
  CodingTimelineNodeStatus,
  CodingAttemptStatus,
  PlanAmendmentManifest,
  PlanRepairRequest,
  PlanRepairSessionSnapshot,
  TimelineNode,
  WorkItemRevisionHistoryDto,
  WorkspaceSessionLink,
} from "../api/types";

export type PlanRepairSessionState = Omit<PlanRepairSessionSnapshot, "timeline_nodes"> & {
  childSessionId: string;
  childTimelineNodes: TimelineNode[];
  timelineNodes: CodingTimelineNode[];
  history: WorkItemRevisionHistoryDto | null;
};

export type PlanRepairRequiredInput = {
  request: PlanRepairRequest;
  session_link: WorkspaceSessionLink | null;
};

type PlanRepairStoreSlice = {
  attemptId: string | null;
  status: CodingAttemptStatus | null;
  timelineNodes: CodingTimelineNode[];
  activePlanRepair: PlanRepairSessionState | null;
};

export function planRepairRequiredStateUpdate(
  state: PlanRepairStoreSlice,
  message: PlanRepairRequiredInput,
): Partial<PlanRepairStoreSlice> {
  if (!state.attemptId) {
    return {};
  }
  const activePlanRepair = repairSessionFromRequired(
    message,
    state.attemptId,
    state.activePlanRepair,
  );
  if (activePlanRepair === state.activePlanRepair) {
    return {};
  }
  return { activePlanRepair, status: "awaiting_plan_amendment" };
}

export function planRepairSnapshotStateUpdate(
  state: PlanRepairStoreSlice,
  snapshot: PlanRepairSessionSnapshot,
): Partial<PlanRepairStoreSlice> {
  if (!state.attemptId) {
    return {};
  }
  const activePlanRepair = updateRepairSessionSnapshot(
    state.activePlanRepair,
    snapshot,
    state.attemptId,
  );
  if (activePlanRepair === state.activePlanRepair || !activePlanRepair) {
    return {};
  }
  return {
    activePlanRepair,
    timelineNodes: replaceRepairTimelineNodes(
      state.timelineNodes,
      state.activePlanRepair?.timelineNodes ?? [],
      activePlanRepair.timelineNodes,
    ),
  };
}

export function planRepairTimelineNodeAddedStateUpdate(
  state: PlanRepairStoreSlice,
  childSessionId: string,
  node: TimelineNode,
): Partial<PlanRepairStoreSlice> {
  if (!state.attemptId) {
    return {};
  }
  const activePlanRepair = addRepairTimelineNode(
    state.activePlanRepair,
    childSessionId,
    node,
    state.attemptId,
  );
  if (activePlanRepair === state.activePlanRepair || !activePlanRepair) {
    return {};
  }
  return {
    activePlanRepair,
    timelineNodes: mergeRepairTimelineNodes(
      state.timelineNodes,
      activePlanRepair.timelineNodes,
    ),
  };
}

export function planRepairTimelineNodeUpdatedStateUpdate(
  state: PlanRepairStoreSlice,
  childSessionId: string,
  nodeId: string,
  status: TimelineNode["status"],
  summary?: string | null,
  completedAt?: string | null,
): Partial<PlanRepairStoreSlice> {
  const activePlanRepair = updateRepairTimelineNode(
    state.activePlanRepair,
    childSessionId,
    nodeId,
    status,
    summary,
    completedAt,
  );
  if (activePlanRepair === state.activePlanRepair || !activePlanRepair) {
    return {};
  }
  return {
    activePlanRepair,
    timelineNodes: mergeRepairTimelineNodes(
      state.timelineNodes,
      activePlanRepair.timelineNodes,
    ),
  };
}

export function planRepairHistoryStateUpdate(
  state: PlanRepairStoreSlice,
  childSessionId: string,
  history: WorkItemRevisionHistoryDto,
): Partial<PlanRepairStoreSlice> {
  const activePlanRepair = setRepairHistory(
    state.activePlanRepair,
    childSessionId,
    history,
  );
  return activePlanRepair === state.activePlanRepair ? {} : { activePlanRepair };
}

export function planRepairAmendmentStateUpdate(
  state: PlanRepairStoreSlice,
  amendment: PlanAmendmentManifest,
  childSessionId?: string,
): Partial<PlanRepairStoreSlice> {
  const activePlanRepair = setRepairAmendment(
    state.activePlanRepair,
    amendment,
    childSessionId,
  );
  return activePlanRepair === state.activePlanRepair ? {} : { activePlanRepair };
}

export function planRepairResumeStateUpdate(
  state: PlanRepairStoreSlice,
  amendmentId: string,
): Partial<PlanRepairStoreSlice> {
  if (!repairMatchesAmendment(state.activePlanRepair, amendmentId)) {
    return {};
  }
  const resumeMode = state.activePlanRepair?.amendment?.resume_target.mode;
  return {
    activePlanRepair: null,
    status:
      state.status === "awaiting_plan_amendment" && resumeMode !== "await_handoff"
        ? "running"
        : state.status,
  };
}

export function repairSessionFromSnapshot(
  snapshot: PlanRepairSessionSnapshot,
  attemptId: string,
  history: WorkItemRevisionHistoryDto | null = null,
): PlanRepairSessionState | null {
  if (!snapshotBelongsToAttempt(snapshot, attemptId)) {
    return null;
  }
  return normalizedRepairSession(snapshot, attemptId, history);
}

export function repairSessionFromRequired(
  message: PlanRepairRequiredInput,
  attemptId: string,
  current: PlanRepairSessionState | null,
): PlanRepairSessionState | null {
  const link = message.session_link;
  if (!link || !requestAndLinkBelongToAttempt(message.request, link, attemptId)) {
    return current;
  }
  if (
    current &&
    current.request.id === message.request.id &&
    current.childSessionId === link.child_session_id
  ) {
    if (isOlderRequest(message.request, current.request)) {
      return current;
    }
    return {
      ...current,
      request: message.request,
      link,
    };
  }
  return {
    request: message.request,
    link,
    stage: stageForRequest(message.request),
    projection: null,
    amendment: null,
    validation: null,
    impact: null,
    plan_review: null,
    package_identity: null,
    candidate_package_artifact_id: null,
    impact_scope_review: null,
    error: null,
    childSessionId: link.child_session_id,
    childTimelineNodes: [],
    timelineNodes: [],
    history: null,
  };
}

export function updateRepairSessionSnapshot(
  current: PlanRepairSessionState | null,
  snapshot: PlanRepairSessionSnapshot,
  attemptId: string,
): PlanRepairSessionState | null {
  if (
    !current ||
    snapshot.link.child_session_id !== current.childSessionId ||
    snapshot.request.id !== current.request.id ||
    isOlderRequest(snapshot.request, current.request) ||
    !snapshotBelongsToAttempt(snapshot, attemptId)
  ) {
    return current;
  }
  const next = normalizedRepairSession(snapshot, attemptId, current.history);
  return {
    ...next,
    childTimelineNodes: current.childTimelineNodes.reduce(
      (nodes, node) => upsertById(nodes, node, "node_id"),
      next.childTimelineNodes,
    ),
    timelineNodes: current.timelineNodes.reduce(
      (nodes, node) => upsertById(nodes, node, "id"),
      next.timelineNodes,
    ),
  };
}

export function addRepairTimelineNode(
  current: PlanRepairSessionState | null,
  childSessionId: string,
  node: TimelineNode,
  attemptId: string,
): PlanRepairSessionState | null {
  if (!current || current.childSessionId !== childSessionId) {
    return current;
  }
  return {
    ...current,
    childTimelineNodes: upsertById(current.childTimelineNodes, node, "node_id"),
    timelineNodes: upsertById(
      current.timelineNodes,
      codingTimelineNode(node, attemptId),
      "id",
    ),
  };
}

export function updateRepairTimelineNode(
  current: PlanRepairSessionState | null,
  childSessionId: string,
  nodeId: string,
  status: TimelineNode["status"],
  summary?: string | null,
  completedAt?: string | null,
): PlanRepairSessionState | null {
  if (!current || current.childSessionId !== childSessionId) {
    return current;
  }
  const existing = current.childTimelineNodes.find((node) => node.node_id === nodeId);
  if (!existing) {
    return current;
  }
  const childTimelineNodes = current.childTimelineNodes.map((node) =>
    node.node_id === nodeId
      ? {
          ...node,
          status,
          summary: summary ?? node.summary,
          completed_at: completedAt ?? node.completed_at,
        }
      : node,
  );
  return {
    ...current,
    childTimelineNodes,
    timelineNodes: current.timelineNodes.map((node) =>
      node.id === nodeId
        ? {
            ...node,
            status: codingTimelineStatus(status),
            summary: summary ?? node.summary,
            completed_at: completedAt ?? node.completed_at,
          }
        : node,
    ),
  };
}

export function setRepairAmendment(
  current: PlanRepairSessionState | null,
  amendment: PlanAmendmentManifest,
  childSessionId?: string,
): PlanRepairSessionState | null {
  if (
    !current ||
    (childSessionId !== undefined && current.childSessionId !== childSessionId) ||
    amendment.repair_request_id !== current.request.id ||
    amendment.previous_plan_revision_id !== current.request.base_plan_revision_id ||
    (current.request.amendment_id !== null && current.request.amendment_id !== amendment.id) ||
    (current.link.trigger.amendment_id !== "" &&
      current.link.trigger.amendment_id !== amendment.id)
  ) {
    return current;
  }
  return { ...current, amendment };
}

export function setRepairHistory(
  current: PlanRepairSessionState | null,
  childSessionId: string,
  history: WorkItemRevisionHistoryDto,
): PlanRepairSessionState | null {
  if (!current || current.childSessionId !== childSessionId) {
    return current;
  }
  return { ...current, history };
}

export function repairMatchesAmendment(
  current: PlanRepairSessionState | null,
  amendmentId: string,
) {
  if (!current) {
    return false;
  }
  return (
    current.amendment?.id === amendmentId ||
    current.request.amendment_id === amendmentId ||
    current.link.trigger.amendment_id === amendmentId
  );
}

export function mergeRepairTimelineNodes(
  parentNodes: CodingTimelineNode[],
  repairNodes: CodingTimelineNode[],
) {
  return repairNodes.reduce(
    (nodes, node) => upsertById(nodes, node, "id"),
    parentNodes,
  );
}

export function replaceRepairTimelineNodes(
  parentNodes: CodingTimelineNode[],
  previousRepairNodes: CodingTimelineNode[],
  nextRepairNodes: CodingTimelineNode[],
) {
  const previousIds = new Set(previousRepairNodes.map((node) => node.id));
  return mergeRepairTimelineNodes(
    parentNodes.filter((node) => !previousIds.has(node.id)),
    nextRepairNodes,
  );
}

function normalizedRepairSession(
  snapshot: PlanRepairSessionSnapshot,
  attemptId: string,
  history: WorkItemRevisionHistoryDto | null,
): PlanRepairSessionState {
  return {
    ...snapshot,
    childSessionId: snapshot.link.child_session_id,
    childTimelineNodes: snapshot.timeline_nodes,
    timelineNodes: snapshot.timeline_nodes.map((node) => codingTimelineNode(node, attemptId)),
    history,
  };
}

function snapshotBelongsToAttempt(snapshot: PlanRepairSessionSnapshot, attemptId: string) {
  return requestAndLinkBelongToAttempt(snapshot.request, snapshot.link, attemptId);
}

function requestAndLinkBelongToAttempt(
  request: PlanRepairRequest,
  link: WorkspaceSessionLink,
  attemptId: string,
) {
  return (
    link.relation === "plan_repair" &&
    request.trigger_attempt_id === attemptId &&
    link.parent_session_id === attemptId &&
    link.trigger.attempt_id === attemptId &&
    link.return_context.original_attempt_id === attemptId &&
    link.trigger.repair_request_id === request.id &&
    link.trigger.fingerprint === request.fingerprint &&
    link.trigger.base_plan_revision_id === request.base_plan_revision_id &&
    (request.amendment_id === null || link.trigger.amendment_id === request.amendment_id)
  );
}

function isOlderRequest(next: PlanRepairRequest, current: PlanRepairRequest) {
  return next.updated_at < current.updated_at;
}

function stageForRequest(request: PlanRepairRequest): PlanRepairSessionState["stage"] {
  switch (request.status) {
    case "open":
      return "triaging";
    case "in_progress":
      return "authoring_revision";
    case "awaiting_confirmation":
      return "awaiting_confirmation";
    case "published":
      return "published";
    case "applied":
      return "completed";
    case "cancelled":
    case "failed":
      return "failed";
  }
}

function codingTimelineNode(node: TimelineNode, attemptId: string): CodingTimelineNode {
  return {
    id: node.node_id,
    attempt_id: attemptId,
    stage: codingStageForTimelineNode(node),
    title: node.title,
    status: codingTimelineStatus(node.status),
    agent_role: codingAgentRoleForTimelineNode(node),
    summary: node.summary ?? null,
    started_at: node.started_at,
    completed_at: node.completed_at ?? null,
    artifact_refs: node.artifact_ref ? [node.artifact_ref] : [],
  };
}

function codingStageForTimelineNode(node: TimelineNode): CodingTimelineNode["stage"] {
  if (node.node_type.includes("review")) {
    return "code_review";
  }
  if (node.node_type === "human_confirm") {
    return "final_confirm";
  }
  if (node.node_type.includes("validation")) {
    return "testing";
  }
  return "coding";
}

function codingAgentRoleForTimelineNode(node: TimelineNode): CodingTimelineNode["agent_role"] {
  if (node.node_type.includes("review")) {
    return "reviewer";
  }
  if (node.node_type.includes("author") || node.node_type === "revision") {
    return "author";
  }
  return "system";
}

function codingTimelineStatus(status: TimelineNode["status"]): CodingTimelineNodeStatus {
  switch (status) {
    case "active":
      return "running";
    case "paused":
      return "blocked";
    case "completed":
    case "skipped":
      return "completed";
    case "failed":
      return "failed";
  }
}

function upsertById<T, K extends keyof T>(items: T[], item: T, key: K): T[] {
  const value = item[key];
  const index = items.findIndex((existing) => existing[key] === value);
  if (index === -1) {
    return [...items, item];
  }
  return items.map((existing, itemIndex) => (itemIndex === index ? item : existing));
}
