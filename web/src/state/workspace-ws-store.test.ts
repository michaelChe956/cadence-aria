import { beforeEach, describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";

describe("workspace ws store", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("clears partial streaming content when an active run is aborted", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("prepare_context");

    expect(useWorkspaceStore.getState().streamingContent).toBe("");
  });

  it("keeps streaming content while the stage remains running", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("running");

    expect(useWorkspaceStore.getState().streamingContent).toBe("partial output");
  });
});
