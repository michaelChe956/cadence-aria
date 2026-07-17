import type { HumanPresentationRevision } from "../api/types";
import type { WorkspaceWsState } from "./workspace-ws-store-types";

export function humanPresentationRevisionsFromSession(
  revisions: HumanPresentationRevision[] | undefined,
) {
  return Object.fromEntries(
    (revisions ?? []).flatMap((revision) => {
      const sourceProjectionBundleId = humanPresentationSourceBundleId(revision);
      return sourceProjectionBundleId ? [[sourceProjectionBundleId, revision]] : [];
    }),
  );
}

export function beginHumanPresentationSave(
  prev: WorkspaceWsState,
  sourceProjectionBundleId: string,
) {
  return {
    humanPresentationSaveStates: {
      ...prev.humanPresentationSaveStates,
      [sourceProjectionBundleId]: { saving: true, error: null },
    },
  };
}

export function completeHumanPresentationSave(
  prev: WorkspaceWsState,
  revision: HumanPresentationRevision,
) {
  const sourceProjectionBundleId = humanPresentationSourceBundleId(revision);
  if (!sourceProjectionBundleId) {
    return {};
  }
  return {
    humanPresentationRevisions: {
      ...prev.humanPresentationRevisions,
      [sourceProjectionBundleId]: revision,
    },
    humanPresentationSaveStates: {
      ...prev.humanPresentationSaveStates,
      [sourceProjectionBundleId]: { saving: false, error: null },
    },
  };
}

export function failHumanPresentationSave(
  prev: WorkspaceWsState,
  sourceProjectionBundleId: string,
  message: string,
) {
  return {
    humanPresentationSaveStates: {
      ...prev.humanPresentationSaveStates,
      [sourceProjectionBundleId]: { saving: false, error: message },
    },
  };
}

export function humanPresentationSourceBundleId(
  revision: HumanPresentationRevision,
) {
  return (
    revision.source_plan_projection_bundle_id ??
    revision.source_work_item_projection_bundle_id
  );
}
