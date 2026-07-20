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

export type PlanRepairIdentitySource = Pick<
  PlanRepairSessionSnapshot,
  "request" | "link"
>;

type PlanRepairStoreSlice = {
  projectId: string | null;
  issueId: string | null;
  attemptId: string | null;
  status: CodingAttemptStatus | null;
  timelineNodes: CodingTimelineNode[];
  activePlanRepair: PlanRepairSessionState | null;
};

export function planRepairRequiredStateUpdate(
  state: PlanRepairStoreSlice,
  message: PlanRepairRequiredInput,
): Partial<PlanRepairStoreSlice> {
  if (!state.projectId || !state.issueId || !state.attemptId) {
    return {};
  }
  const activePlanRepair = repairSessionFromRequired(
    message,
    state.attemptId,
    state.activePlanRepair,
    repairParentRoute(state.projectId, state.issueId, state.attemptId),
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
  if (activePlanRepair === state.activePlanRepair) {
    return {};
  }
  return {
    activePlanRepair,
    timelineNodes: replaceRepairTimelineNodes(
      state.timelineNodes,
      state.activePlanRepair?.timelineNodes ?? [],
      activePlanRepair?.timelineNodes ?? [],
    ),
  };
}

export function planRepairTimelineNodeAddedStateUpdate(
  state: PlanRepairStoreSlice,
  source: PlanRepairIdentitySource,
  node: TimelineNode,
): Partial<PlanRepairStoreSlice> {
  if (!state.attemptId) {
    return {};
  }
  const activePlanRepair = addRepairTimelineNode(
    state.activePlanRepair,
    source,
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
  source: PlanRepairIdentitySource,
  nodeId: string,
  status: TimelineNode["status"],
  summary?: string | null,
  completedAt?: string | null,
): Partial<PlanRepairStoreSlice> {
  const activePlanRepair = updateRepairTimelineNode(
    state.activePlanRepair,
    source,
    nodeId,
    status,
    summary,
    completedAt,
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

export function planRepairHistoryStateUpdate(
  state: PlanRepairStoreSlice,
  source: PlanRepairIdentitySource,
  history: WorkItemRevisionHistoryDto,
): Partial<PlanRepairStoreSlice> {
  const activePlanRepair = setRepairHistory(
    state.activePlanRepair,
    source,
    history,
    state.attemptId,
  );
  return activePlanRepair === state.activePlanRepair ? {} : { activePlanRepair };
}

export function planRepairAmendmentStateUpdate(
  state: PlanRepairStoreSlice,
  amendment: PlanAmendmentManifest,
  source?: PlanRepairIdentitySource,
): Partial<PlanRepairStoreSlice> {
  const activePlanRepair = setRepairAmendment(
    state.activePlanRepair,
    amendment,
    source,
    state.attemptId,
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
  expectedRoute: string,
  history: WorkItemRevisionHistoryDto | null = null,
): PlanRepairSessionState | null {
  if (
    !snapshotBelongsToAttempt(snapshot, attemptId, expectedRoute) ||
    isTerminalRepairRequest(snapshot.request)
  ) {
    return null;
  }
  return normalizedRepairSession(snapshot, attemptId, history);
}

export function reconcileParentPlanRepair(
  current: PlanRepairSessionState | null,
  snapshot: PlanRepairSessionSnapshot | null | undefined,
  attemptId: string,
  attemptStatus: CodingAttemptStatus,
  expectedRoute: string,
): PlanRepairSessionState | null {
  const validCurrent =
    current && snapshotBelongsToAttempt(current, attemptId, expectedRoute)
      ? current
      : null;
  if (!snapshot) {
    return isPlanRepairBlockedAttempt(attemptStatus) ? validCurrent : null;
  }
  if (!snapshotBelongsToAttempt(snapshot, attemptId, expectedRoute)) {
    return validCurrent;
  }
  if (!validCurrent) {
    return repairSessionFromSnapshot(snapshot, attemptId, expectedRoute);
  }
  if (!sameRepairDurableIdentity(validCurrent, snapshot)) {
    return validCurrent;
  }
  if (isOlderRequest(snapshot.request, validCurrent.request)) {
    return validCurrent;
  }
  if (isTerminalRepairRequest(snapshot.request)) {
    return null;
  }
  return reconcileRepairSession(validCurrent, snapshot, attemptId);
}

function isPlanRepairBlockedAttempt(status: CodingAttemptStatus) {
  return (
    status === "awaiting_plan_amendment" ||
    status === "applying_plan_amendment" ||
    status === "amendment_apply_failed"
  );
}

export function repairSessionFromRequired(
  message: PlanRepairRequiredInput,
  attemptId: string,
  current: PlanRepairSessionState | null,
  expectedRoute: string,
): PlanRepairSessionState | null {
  const link = message.session_link;
  if (
    !link ||
    !requestAndLinkBelongToAttempt(message.request, link, attemptId, expectedRoute)
  ) {
    return current;
  }
  if (isTerminalRepairRequest(message.request)) {
    return current;
  }
  const nextIdentity = { request: message.request, link };
  if (current) {
    if (!sameRepairDurableIdentity(current, nextIdentity)) {
      return current;
    }
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
    !snapshotBelongsToAttempt(
      snapshot,
      attemptId,
      current.link.return_context.original_route,
    ) ||
    !sameRepairDurableIdentity(current, snapshot)
  ) {
    return current;
  }
  if (isOlderRequest(snapshot.request, current.request)) {
    return current;
  }
  if (isTerminalRepairRequest(snapshot.request)) {
    return null;
  }
  return reconcileRepairSession(current, snapshot, attemptId);
}

export function addRepairTimelineNode(
  current: PlanRepairSessionState | null,
  source: PlanRepairIdentitySource,
  node: TimelineNode,
  attemptId: string,
): PlanRepairSessionState | null {
  if (!current || !repairSourceMatchesCurrent(current, source, attemptId)) {
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
  source: PlanRepairIdentitySource,
  nodeId: string,
  status: TimelineNode["status"],
  summary?: string | null,
  completedAt?: string | null,
  attemptId?: string | null,
): PlanRepairSessionState | null {
  if (
    !attemptId ||
    !current ||
    !repairSourceMatchesCurrent(current, source, attemptId)
  ) {
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
  source: PlanRepairIdentitySource | undefined,
  attemptId: string | null,
): PlanRepairSessionState | null {
  if (
    !attemptId ||
    !current ||
    (source !== undefined && !repairSourceMatchesCurrent(current, source, attemptId)) ||
    amendment.repair_request_id !== current.request.id ||
    amendment.previous_plan_revision_id !== current.request.base_plan_revision_id ||
    current.request.amendment_id !== amendment.id ||
    current.link.trigger.amendment_id !== amendment.id
  ) {
    return current;
  }
  return { ...current, amendment };
}

export function setRepairHistory(
  current: PlanRepairSessionState | null,
  source: PlanRepairIdentitySource,
  history: WorkItemRevisionHistoryDto,
  attemptId: string | null,
): PlanRepairSessionState | null {
  if (
    !attemptId ||
    !current ||
    !repairSourceMatchesCurrent(current, source, attemptId)
  ) {
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
  const durableAmendmentId = current.request.amendment_id;
  return (
    durableAmendmentId !== null &&
    durableAmendmentId !== "" &&
    amendmentId === durableAmendmentId &&
    current.link.trigger.amendment_id === durableAmendmentId &&
    current.amendment?.id === durableAmendmentId &&
    current.amendment.repair_request_id === current.request.id &&
    current.amendment.previous_plan_revision_id ===
      current.request.base_plan_revision_id
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

function snapshotBelongsToAttempt(
  snapshot: PlanRepairIdentitySource,
  attemptId: string,
  expectedRoute: string,
) {
  return requestAndLinkBelongToAttempt(
    snapshot.request,
    snapshot.link,
    attemptId,
    expectedRoute,
  );
}

function requestAndLinkBelongToAttempt(
  request: PlanRepairRequest,
  link: WorkspaceSessionLink,
  attemptId: string,
  expectedRoute: string,
) {
  const amendmentId = request.amendment_id;
  return (
    link.relation === "plan_repair" &&
    link.id !== "" &&
    link.child_session_id !== "" &&
    request.id !== "" &&
    request.plan_id !== "" &&
    request.fingerprint !== "" &&
    request.base_plan_revision_id !== "" &&
    amendmentId !== null &&
    amendmentId !== "" &&
    request.trigger_attempt_id === attemptId &&
    link.parent_session_id === attemptId &&
    link.trigger.attempt_id === attemptId &&
    link.return_context.original_attempt_id === attemptId &&
    link.trigger.unit_run_id === request.trigger_unit_run_id &&
    link.return_context.original_unit_run_id === request.trigger_unit_run_id &&
    link.trigger.review_id === request.trigger_review_id &&
    link.trigger.finding_id === request.trigger_finding_id &&
    link.return_context.timeline_anchor_id === request.trigger_finding_id &&
    link.trigger.repair_request_id === request.id &&
    link.trigger.fingerprint === request.fingerprint &&
    link.trigger.base_plan_revision_id === request.base_plan_revision_id &&
    link.trigger.amendment_id === amendmentId &&
    link.return_context.original_route === expectedRoute
  );
}

export function repairSourceMatchesCurrent(
  current: PlanRepairSessionState | null,
  source: PlanRepairIdentitySource,
  attemptId: string,
) {
  return (
    current !== null &&
    snapshotBelongsToAttempt(
      source,
      attemptId,
      current.link.return_context.original_route,
    ) &&
    sameRepairDurableIdentity(current, source)
  );
}

function sameRepairDurableIdentity(
  left: PlanRepairIdentitySource,
  right: PlanRepairIdentitySource,
) {
  return (
    left.request.id === right.request.id &&
    left.request.plan_id === right.request.plan_id &&
    left.request.base_plan_revision_id === right.request.base_plan_revision_id &&
    left.request.trigger_attempt_id === right.request.trigger_attempt_id &&
    left.request.trigger_unit_run_id === right.request.trigger_unit_run_id &&
    left.request.trigger_review_id === right.request.trigger_review_id &&
    left.request.trigger_finding_id === right.request.trigger_finding_id &&
    left.request.amendment_id === right.request.amendment_id &&
    left.request.fingerprint === right.request.fingerprint &&
    left.link.id === right.link.id &&
    left.link.relation === right.link.relation &&
    left.link.parent_session_id === right.link.parent_session_id &&
    left.link.child_session_id === right.link.child_session_id &&
    left.link.trigger.attempt_id === right.link.trigger.attempt_id &&
    left.link.trigger.unit_run_id === right.link.trigger.unit_run_id &&
    left.link.trigger.review_id === right.link.trigger.review_id &&
    left.link.trigger.finding_id === right.link.trigger.finding_id &&
    left.link.trigger.repair_request_id === right.link.trigger.repair_request_id &&
    left.link.trigger.amendment_id === right.link.trigger.amendment_id &&
    left.link.trigger.fingerprint === right.link.trigger.fingerprint &&
    left.link.trigger.base_plan_revision_id === right.link.trigger.base_plan_revision_id &&
    left.link.return_context.original_attempt_id ===
      right.link.return_context.original_attempt_id &&
    left.link.return_context.original_unit_run_id ===
      right.link.return_context.original_unit_run_id &&
    left.link.return_context.timeline_anchor_id ===
      right.link.return_context.timeline_anchor_id &&
    left.link.return_context.original_route ===
      right.link.return_context.original_route
  );
}

function repairParentRoute(projectId: string, issueId: string, attemptId: string) {
  return `/workbench/projects/${projectId}/issues/${issueId}/coding/${attemptId}`;
}

function reconcileRepairSession(
  current: PlanRepairSessionState,
  snapshot: PlanRepairSessionSnapshot,
  attemptId: string,
): PlanRepairSessionState {
  const equalVersion = snapshot.request.updated_at === current.request.updated_at;
  const childTimelineNodes = reconcileChildTimelineNodes(
    current.childTimelineNodes,
    snapshot.timeline_nodes,
    snapshot.request.updated_at,
  );
  const stage =
    equalVersion && stageOrder(snapshot.stage) < stageOrder(current.stage)
      ? current.stage
      : snapshot.stage;
  return {
    ...snapshot,
    stage,
    projection: snapshot.projection ?? current.projection,
    amendment: snapshot.amendment ?? current.amendment,
    validation: snapshot.validation ?? current.validation,
    impact: snapshot.impact ?? current.impact,
    plan_review: snapshot.plan_review ?? current.plan_review,
    package_identity: snapshot.package_identity ?? current.package_identity,
    candidate_package_artifact_id:
      snapshot.candidate_package_artifact_id ?? current.candidate_package_artifact_id,
    impact_scope_review: snapshot.impact_scope_review ?? current.impact_scope_review,
    error: snapshot.error ?? current.error,
    childSessionId: snapshot.link.child_session_id,
    childTimelineNodes,
    timelineNodes: childTimelineNodes.map((node) => codingTimelineNode(node, attemptId)),
    history: current.history,
  };
}

function reconcileChildTimelineNodes(
  current: TimelineNode[],
  snapshot: TimelineNode[],
  snapshotWatermark: string,
) {
  const snapshotIds = new Set(snapshot.map((node) => node.node_id));
  return [
    ...snapshot,
    ...current.filter(
      (node) =>
        !snapshotIds.has(node.node_id) &&
        isLiveTimelineNode(node) &&
        node.started_at > snapshotWatermark,
    ),
  ];
}

function isLiveTimelineNode(node: TimelineNode) {
  return node.status === "active" || node.status === "paused";
}

function isTerminalRepairRequest(request: PlanRepairRequest) {
  return ["applied", "cancelled", "failed"].includes(request.status);
}

function stageOrder(stage: PlanRepairSessionState["stage"]) {
  return [
    "triaging",
    "authoring_revision",
    "validating_contract",
    "generating_projections",
    "plan_review",
    "awaiting_confirmation",
    "published",
    "amendment_conflict",
    "applying_amendment",
    "amendment_apply_failed",
    "completed",
    "failed",
  ].indexOf(stage);
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
