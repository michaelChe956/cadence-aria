import { create } from "zustand";
import type {
  LinkedWorkspaceAmendmentTarget,
  LinkedWorkspaceSessionSnapshot,
} from "../api/types";

export type LinkedWorkspaceAmendmentStatus =
  | "idle"
  | "pending"
  | "ready"
  | "error";

type LinkedWorkspaceAmendmentState = {
  parentSessionId: string | null;
  target: LinkedWorkspaceAmendmentTarget | null;
  snapshot: LinkedWorkspaceSessionSnapshot | null;
  status: LinkedWorkspaceAmendmentStatus;
  error: string | null;
};

type LinkedWorkspaceAmendmentActions = {
  reset: (parentSessionId: string | null) => void;
  begin: (target: LinkedWorkspaceAmendmentTarget) => void;
  fail: (message: string) => void;
  consume: (snapshot: LinkedWorkspaceSessionSnapshot) => boolean;
};

const initialState: LinkedWorkspaceAmendmentState = {
  parentSessionId: null,
  target: null,
  snapshot: null,
  status: "idle",
  error: null,
};

export const useLinkedWorkspaceAmendmentStore = create<
  LinkedWorkspaceAmendmentState & LinkedWorkspaceAmendmentActions
>((set, get) => ({
  ...initialState,
  reset: (parentSessionId) => {
    set({ ...initialState, parentSessionId });
  },
  begin: (target) => {
    if (!get().parentSessionId) {
      set({
        target: null,
        snapshot: null,
        status: "error",
        error: "缺少当前 Repair Child Session，无法发起关联修订。",
      });
      return;
    }
    set({ target, snapshot: null, status: "pending", error: null });
  },
  fail: (message) => {
    set({ snapshot: null, status: "error", error: message });
  },
  consume: (snapshot) => {
    const state = get();
    const error = linkedWorkspaceSnapshotError(state, snapshot);
    if (error) {
      set({ snapshot: null, status: "error", error });
      return false;
    }
    set({ snapshot, status: "ready", error: null });
    return true;
  },
}));

function linkedWorkspaceSnapshotError(
  state: LinkedWorkspaceAmendmentState,
  snapshot: LinkedWorkspaceSessionSnapshot,
): string | null {
  const parentSessionId = state.parentSessionId;
  if (!parentSessionId) {
    return "缺少当前 Repair Child Session，拒绝关联修订响应。";
  }
  const expectedRoute = `/workbench/workspace/${parentSessionId}`;
  if (
    snapshot.link.parent_session_id !== parentSessionId ||
    snapshot.link.return_context.original_route !== expectedRoute
  ) {
    return "关联修订响应不属于当前 Repair Child Session。";
  }
  if (!linkedWorkspaceIdentityIsComplete(snapshot)) {
    return "关联修订响应缺少权威身份字段。";
  }
  if (!relationMatchesWorkspace(snapshot.link.relation, snapshot.workspace_type)) {
    return "关联修订关系与 Workspace 类型不匹配。";
  }
  if (
    state.target &&
    (state.target.relation !== snapshot.link.relation ||
      state.target.workspace_type !== snapshot.workspace_type)
  ) {
    return "关联修订响应与当前请求目标不匹配。";
  }
  if (
    snapshot.selected_timeline_node_id &&
    !snapshot.timeline_nodes.some(
      (node) => node.node_id === snapshot.selected_timeline_node_id,
    )
  ) {
    return "关联修订响应引用了不存在的 Timeline 节点。";
  }
  if (
    snapshot.human_confirm_state === "blocked_provider_unavailable" ||
    snapshot.human_confirm_state === "terminated"
  ) {
    return "关联修订 Child Workspace 尚未处于可打开状态。";
  }
  return null;
}

function linkedWorkspaceIdentityIsComplete(
  snapshot: LinkedWorkspaceSessionSnapshot,
): boolean {
  const { link } = snapshot;
  return [
    link.id,
    link.parent_session_id,
    link.child_session_id,
    link.trigger.attempt_id,
    link.trigger.unit_run_id,
    link.trigger.finding_id,
    link.trigger.repair_request_id,
    link.trigger.amendment_id,
    link.trigger.fingerprint,
    link.trigger.base_plan_revision_id,
    link.return_context.original_attempt_id,
    link.return_context.original_unit_run_id,
    link.return_context.timeline_anchor_id,
  ].every((value) => value.trim().length > 0);
}

function relationMatchesWorkspace(
  relation: string,
  workspaceType: string,
): boolean {
  return (
    (relation === "story_amendment" && workspaceType === "story") ||
    (relation === "design_amendment" && workspaceType === "design")
  );
}
