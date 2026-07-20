import { describe, expect, it } from "vitest";
import { linkedWorkspaceAmendmentSnapshotFixture } from "../components/coding-workspace/plan-repair-test-fixtures";
import { useLinkedWorkspaceAmendmentStore } from "./linked-workspace-amendment-store";

describe("linked workspace amendment store", () => {
  it("moves from pending to ready only for the active repair child", () => {
    const store = useLinkedWorkspaceAmendmentStore.getState();
    store.reset("workspace_session_repair_0001");
    store.begin({
      entity_id: "story_spec_0001",
      workspace_type: "story",
      relation: "story_amendment",
    });

    expect(useLinkedWorkspaceAmendmentStore.getState().status).toBe("pending");
    expect(store.consume(snapshotFor("story_spec_0001"))).toBe(true);
    expect(useLinkedWorkspaceAmendmentStore.getState()).toMatchObject({
      status: "ready",
      snapshot: snapshotFor("story_spec_0001"),
      error: null,
    });
  });

  it("fails closed for a forged parent or relation/workspace mismatch", () => {
    const store = useLinkedWorkspaceAmendmentStore.getState();
    store.reset("workspace_session_repair_0001");
    store.begin({
      entity_id: "story_spec_0001",
      workspace_type: "story",
      relation: "story_amendment",
    });
    const snapshot = snapshotFor("story_spec_0001");

    expect(
      store.consume({
        ...snapshot,
        link: { ...snapshot.link, parent_session_id: "workspace_session_other" },
      }),
    ).toBe(false);
    expect(useLinkedWorkspaceAmendmentStore.getState()).toMatchObject({
      status: "error",
      snapshot: null,
    });
  });

  it("fails closed for an unsolicited response while idle", () => {
    const store = useLinkedWorkspaceAmendmentStore.getState();
    store.reset("workspace_session_repair_0001");

    expect(store.consume(snapshotFor("story_spec_0001"))).toBe(false);
    expect(useLinkedWorkspaceAmendmentStore.getState()).toMatchObject({
      status: "error",
      snapshot: null,
    });
  });

  it("rejects a same-type response for a different entity", () => {
    const store = useLinkedWorkspaceAmendmentStore.getState();
    store.reset("workspace_session_repair_0001");
    store.begin({
      entity_id: "story_spec_0001",
      workspace_type: "story",
      relation: "story_amendment",
    });

    expect(store.consume(snapshotFor("story_spec_0002"))).toBe(false);
    expect(useLinkedWorkspaceAmendmentStore.getState()).toMatchObject({
      status: "error",
      snapshot: null,
    });
  });

  it("rejects a second response after the pending request is consumed", () => {
    const store = useLinkedWorkspaceAmendmentStore.getState();
    store.reset("workspace_session_repair_0001");
    store.begin({
      entity_id: "story_spec_0001",
      workspace_type: "story",
      relation: "story_amendment",
    });
    expect(store.consume(snapshotFor("story_spec_0001"))).toBe(true);

    expect(store.consume(snapshotFor("story_spec_0001"))).toBe(false);
    expect(useLinkedWorkspaceAmendmentStore.getState()).toMatchObject({
      status: "error",
      snapshot: null,
    });
  });
});

function snapshotFor(entityId: string) {
  return {
    ...linkedWorkspaceAmendmentSnapshotFixture(),
    entity_id: entityId,
  };
}
