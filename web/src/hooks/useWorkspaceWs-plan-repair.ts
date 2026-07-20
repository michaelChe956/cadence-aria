import type {
  PlanAmendmentManifest,
  PlanRepairSessionSnapshot,
  WorkItemRevisionHistoryDto,
} from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useLinkedWorkspaceAmendmentStore } from "../state/linked-workspace-amendment-store";
import {
  repairSourceMatchesCurrent,
  type PlanRepairIdentitySource,
} from "../state/plan-repair-session";
import type { TimelineNode } from "../state/workspace-ws-store";
import type { WsServerMessage } from "./workspace-ws-message-handler";

export type PlanRepairSourceState = {
  hasSnapshot: boolean;
  source: PlanRepairIdentitySource | null;
};

export function aggregatePlanRepairChildMessage(
  msg: WsServerMessage,
  sessionId: string | null,
  sourceState: PlanRepairSourceState,
) {
  if (!sessionId) {
    return;
  }
  if (msg.type === "linked_workspace_amendment_created") {
    useLinkedWorkspaceAmendmentStore.getState().consume(msg.snapshot);
    return;
  }
  if (
    msg.type === "protocol_error" &&
    msg.code === "LINKED_WORKSPACE_AMENDMENT_INVALID"
  ) {
    const linkedStore = useLinkedWorkspaceAmendmentStore.getState();
    if (linkedStore.status === "pending") {
      linkedStore.fail(msg.message);
    }
    return;
  }
  const codingStore = useCodingWorkspaceStore.getState();
  switch (msg.type) {
    case "session_state": {
      sourceState.hasSnapshot = true;
      sourceState.source = null;
      const snapshot = msg.plan_repair as PlanRepairSessionSnapshot | null | undefined;
      if (!snapshot || snapshot.link.child_session_id !== sessionId) {
        return;
      }
      codingStore.updatePlanRepairSession(snapshot);
      const current = useCodingWorkspaceStore.getState();
      if (
        !current.attemptId ||
        !repairSourceMatchesCurrent(
          current.activePlanRepair,
          snapshot,
          current.attemptId,
        )
      ) {
        return;
      }
      sourceState.source = { request: snapshot.request, link: snapshot.link };
      const artifacts = structuredPlanRepairArtifacts(msg);
      if (artifacts.history) {
        codingStore.setPlanRepairHistory(sourceState.source, artifacts.history);
      }
      if (artifacts.amendment) {
        codingStore.setPlanAmendment(artifacts.amendment, sourceState.source);
      }
      break;
    }
    case "timeline_node_created": {
      const source = planRepairSourceForMessage(sourceState, sessionId);
      if (!source) return;
      codingStore.addPlanRepairTimelineNode(source, msg.node as TimelineNode);
      break;
    }
    case "timeline_node_updated": {
      const source = planRepairSourceForMessage(sourceState, sessionId);
      if (!source) return;
      codingStore.updatePlanRepairTimelineNode(
        source,
        msg.node_id as string,
        msg.status as TimelineNode["status"],
        msg.summary as string | null | undefined,
        msg.completed_at as string | null | undefined,
      );
      break;
    }
    case "artifact_update": {
      const source = planRepairSourceForMessage(sourceState, sessionId);
      if (!source) return;
      const history = msg.work_item_revision_history as
        | WorkItemRevisionHistoryDto
        | undefined;
      if (history) {
        codingStore.setPlanRepairHistory(source, history);
      }
      const amendment = msg.plan_amendment_manifest as
        | PlanAmendmentManifest
        | undefined;
      if (amendment) {
        codingStore.setPlanAmendment(amendment, source);
      }
      break;
    }
  }
}

function planRepairSourceForMessage(
  state: PlanRepairSourceState,
  sessionId: string,
): PlanRepairIdentitySource | null {
  if (state.hasSnapshot) {
    return state.source;
  }
  const current = useCodingWorkspaceStore.getState().activePlanRepair;
  if (!current || current.childSessionId !== sessionId) {
    return null;
  }
  return { request: current.request, link: current.link };
}

function structuredPlanRepairArtifacts(msg: WsServerMessage) {
  const versions = ((msg as { artifact_versions?: Array<{
    work_item_revision_history?: WorkItemRevisionHistoryDto;
    plan_amendment_manifest?: PlanAmendmentManifest;
  }> }).artifact_versions ?? []).slice().reverse();
  return {
    history: versions.find((version) => version.work_item_revision_history)
      ?.work_item_revision_history,
    amendment: versions.find((version) => version.plan_amendment_manifest)
      ?.plan_amendment_manifest,
  };
}
