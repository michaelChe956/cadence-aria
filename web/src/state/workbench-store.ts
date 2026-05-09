import type { WebEvent, WebWorkspaceProjection } from "../api/types";

export type WorkbenchTab = "overview" | "inputs" | "run" | "outputs" | "diff";

export type WorkbenchSnapshot = {
  projection: WebWorkspaceProjection | null;
  selectedNodeId: string | null;
  selectedTab: WorkbenchTab;
  selectedArtifactRef: string | null;
  selectedTurnId: string | null;
  events: WebEvent[];
};

export function createWorkbenchStore() {
  const snapshot: WorkbenchSnapshot = {
    projection: null,
    selectedNodeId: null,
    selectedTab: "overview",
    selectedArtifactRef: null,
    selectedTurnId: null,
    events: [],
  };

  return {
    snapshot,
    setProjection(projection: WebWorkspaceProjection) {
      snapshot.projection = projection;
      snapshot.selectedNodeId = projection.selected_node_context.node_id;
    },
    selectNode(nodeId: string) {
      snapshot.selectedNodeId = nodeId;
    },
    selectTab(tab: WorkbenchTab) {
      snapshot.selectedTab = tab;
    },
    restoreSelectionFromSearch(params: URLSearchParams) {
      snapshot.selectedNodeId = params.get("node");
      snapshot.selectedTab = (params.get("tab") as WorkbenchTab | null) ?? "overview";
      snapshot.selectedArtifactRef = params.get("artifact");
      snapshot.selectedTurnId = params.get("turn");
    },
    toSearchParams() {
      const params = new URLSearchParams();
      if (snapshot.selectedNodeId) params.set("node", snapshot.selectedNodeId);
      if (snapshot.selectedTab) params.set("tab", snapshot.selectedTab);
      if (snapshot.selectedArtifactRef) params.set("artifact", snapshot.selectedArtifactRef);
      if (snapshot.selectedTurnId) params.set("turn", snapshot.selectedTurnId);
      return params;
    },
    pushEvent(event: WebEvent) {
      snapshot.events = [...snapshot.events.slice(-199), event];
    },
  };
}
