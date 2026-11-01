import { act } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useCodingWorkspaceStore } from "./coding-workspace-store";
import {
  CODING_LOG_TAIL_LIMIT,
  codingLogNodeOptions,
  filterCodingLogLines,
  useCodingLogStore,
  type CodingLogEntryInput,
} from "./coding-log-store";

function entry(overrides: Partial<CodingLogEntryInput> = {}): CodingLogEntryInput {
  return {
    nodeId: "node_1",
    nodeTitle: "Coder",
    text: "hello",
    at: "2026-09-14T08:00:00.000Z",
    kind: "stream",
    ...overrides,
  };
}

describe("coding log store", () => {
  beforeEach(() => {
    useCodingLogStore.getState().reset();
    useCodingWorkspaceStore.getState().reset();
  });

  it("appends lines with strictly increasing ids and merges per flush", () => {
    act(() => {
      useCodingLogStore.getState().appendLines([entry()]);
      useCodingLogStore.getState().appendLines([entry({ text: "world", kind: "event" })]);
    });
    const lines = useCodingLogStore.getState().lines;
    expect(lines.map((line) => line.text)).toEqual(["hello", "world"]);
    expect(lines[0]?.id).toBeLessThan(lines[1]?.id ?? Number.POSITIVE_INFINITY);
  });

  it("keeps only the most recent tail of lines", () => {
    const batch = Array.from({ length: CODING_LOG_TAIL_LIMIT + 120 }, (_, index) =>
      entry({ text: `line-${index}` }),
    );
    act(() => {
      useCodingLogStore.getState().appendLines(batch);
    });
    const lines = useCodingLogStore.getState().lines;
    expect(lines).toHaveLength(CODING_LOG_TAIL_LIMIT);
    expect(lines[0]?.text).toBe(`line-120`);
    expect(lines[lines.length - 1]?.text).toBe(`line-${CODING_LOG_TAIL_LIMIT + 119}`);
  });

  it("filters lines by node and derives deduped node options in first-seen order", () => {
    act(() => {
      useCodingLogStore.getState().appendLines([
        entry({ nodeId: "node_1", nodeTitle: "Coder" }),
        entry({ nodeId: "node_2", nodeTitle: "Reviewer" }),
        entry({ nodeId: "node_1", nodeTitle: "Coder", text: "again" }),
        entry({ nodeId: null, nodeTitle: null, text: "global" }),
      ]);
    });
    const lines = useCodingLogStore.getState().lines;
    expect(filterCodingLogLines(lines, "node_1")).toHaveLength(2);
    expect(filterCodingLogLines(lines, null)).toHaveLength(4);
    expect(codingLogNodeOptions(lines)).toEqual([
      { nodeId: "node_1", nodeTitle: "Coder" },
      { nodeId: "node_2", nodeTitle: "Reviewer" },
    ]);
  });

  it("resets on demand", () => {
    act(() => {
      useCodingLogStore.getState().appendLines([entry()]);
    });
    act(() => {
      useCodingLogStore.getState().reset();
    });
    expect(useCodingLogStore.getState().lines).toEqual([]);
  });

  it("keeps log appends decoupled from the coding chat store", () => {
    const before = useCodingWorkspaceStore.getState().chatEntries;
    act(() => {
      useCodingLogStore.getState().appendLines([entry()]);
    });
    expect(useCodingWorkspaceStore.getState().chatEntries).toBe(before);
    expect(useCodingWorkspaceStore.getState().streamingContent).toBe(null);
  });
});
