import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  LifecycleCard,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import {
  defaultOpenCodingWorkspace,
  findCardInColumns,
  lifecycleCardKey,
} from "./IssueLifecycleWorkbenchParts";

afterEach(() => {
  vi.unstubAllGlobals();
});

it("encodes every default coding workspace path segment", () => {
  const assign = vi.fn();
  vi.stubGlobal("window", { location: { assign } });

  defaultOpenCodingWorkspace({
    projectId: "project/with space",
    issueId: "issue?#with space",
    attemptId: "coding attempt/%1",
  });

  expect(assign).toHaveBeenCalledWith(
    "/workbench/projects/project%2Fwith%20space/issues/issue%3F%23with%20space/coding/coding%20attempt%2F%251",
  );
});

function testCard(
  kind: "story_spec" | "design_spec" | "work_item_group",
  issueId: string,
  id: string,
): LifecycleCard {
  return {
    kind,
    issueId,
    id,
    title: `${issueId} ${kind}`,
    status: "confirmed",
    version: kind === "work_item_group" ? null : 1,
    preview: null,
    sourceIds: [],
    artifactVersions: [],
    ...(kind === "work_item_group"
      ? { childWorkItemIds: [], raw: {} }
      : { raw: {} }),
  } as unknown as LifecycleCard;
}

describe.each([
  ["story_spec", "story_spec_0001", "story_spec"],
  ["design_spec", "design_spec_0001", "design_spec"],
  ["work_item_group", "issue_work_item_plan_0001", "work_item"],
] as const)("%s composite identity", (kind, id, column) => {
  it("selects the matching issue and rejects a bare entity id", () => {
    const issueOne = testCard(kind, "issue_0001", id);
    const issueTwo = testCard(kind, "issue_0002", id);
    const columns = {
      issue: [],
      story_spec: column === "story_spec" ? [issueOne, issueTwo] : [],
      design_spec: column === "design_spec" ? [issueOne, issueTwo] : [],
      work_item: column === "work_item" ? [issueOne, issueTwo] : [],
    } as LifecycleColumns;

    expect(lifecycleCardKey(issueTwo)).toBe(`${kind}:issue_0002:${id}`);
    expect(findCardInColumns(columns, lifecycleCardKey(issueTwo))).toBe(
      issueTwo,
    );
    expect(findCardInColumns(columns, id)).toBeNull();
  });
});
