import { describe, expect, it } from "vitest";
import type { ChatEntry } from "../../state/chat-entries";
import { groupEntries } from "./message-grouping";

describe("groupEntries", () => {
  it("returns an empty list for empty entries", () => {
    expect(groupEntries([])).toEqual([]);
  });

  it("groups provider stream and execution events for the same node", () => {
    const items = groupEntries([
      makeEntry("stream-1", "provider_stream", "author", "生成中", "node-1"),
      makeEntry("event-1", "execution_event", "system", "读取文件", "node-1"),
    ]);

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ kind: "group" });
    if (items[0].kind !== "group") return;
    expect(items[0].group.primaryEntry?.id).toBe("stream-1");
    expect(items[0].group.inlineEvents.map((entry) => entry.id)).toEqual(["event-1"]);
  });

  it("keeps permission responses standalone", () => {
    const items = groupEntries([
      makeEntry("stream-1", "provider_stream", "author", "生成中", "node-1"),
      makeEntry("response-1", "permission_response", "user", "已允许 shell", "node-1"),
    ]);

    expect(items.map((item) => item.kind)).toEqual(["group", "entry"]);
    expect(items[1]).toMatchObject({
      kind: "entry",
      entry: expect.objectContaining({ id: "response-1" }),
    });
  });

  it("uses unique group ids when a standalone entry splits the same node", () => {
    const items = groupEntries([
      makeEntry("stream-1", "provider_stream", "author", "第一段", "node-1"),
      makeEntry("stage-1", "stage_change", "system", "阶段变更", "node-1"),
      makeEntry("stream-2", "provider_stream", "author", "第二段", "node-1"),
    ]);

    const ids = items
      .filter((item) => item.kind === "group")
      .map((item) => (item.kind === "group" ? item.group.id : ""));
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("keeps permission and choice requests as group interrupts", () => {
    const choiceRequest = makeEntry(
      "choice-1",
      "choice_request" as ChatEntry["type"],
      "system",
      "选择下一步",
      "node-1",
    );
    const items = groupEntries([
      makeEntry("stream-1", "provider_stream", "author", "生成中", "node-1"),
      makeEntry("permission-1", "permission_request", "system", "shell", "node-1"),
      choiceRequest,
    ]);

    expect(items).toHaveLength(1);
    if (items[0].kind !== "group") return;
    expect(items[0].group.interruptEntries.map((entry) => entry.id)).toEqual([
      "permission-1",
      "choice-1",
    ]);
  });

  it("groups coding roles by node and keeps analyst verdicts standalone", () => {
    const items = groupEntries([
      makeEntry("tester-stream", "provider_stream", "tester", "测试中", "coding_node_0002"),
      makeEntry("tester-tool", "execution_event", "tester", "run_command", "coding_node_0002"),
      makeEntry(
        "analyst-verdict",
        "analyst_verdict",
        "analyst",
        "测试仍失败",
        "coding_node_0003",
      ),
      makeEntry(
        "review-stream",
        "provider_stream",
        "code_reviewer",
        "审查中",
        "coding_node_0004",
      ),
    ]);

    expect(items.map((item) => item.kind)).toEqual(["group", "entry", "group"]);
    if (items[0].kind !== "group" || items[2].kind !== "group") return;
    expect(items[0].group.role).toBe("tester");
    expect(items[0].group.inlineEvents.map((entry) => entry.id)).toEqual(["tester-tool"]);
    expect(items[1]).toMatchObject({
      kind: "entry",
      entry: expect.objectContaining({ id: "analyst-verdict" }),
    });
    expect(items[2].group.role).toBe("code_reviewer");
  });
});

function makeEntry(
  id: string,
  type: ChatEntry["type"],
  role: ChatEntry["role"],
  content: string,
  nodeId?: string,
): ChatEntry {
  return {
    id,
    type,
    role,
    content,
    timestamp: "2026-05-26T10:00:00Z",
    node_id: nodeId,
  };
}
