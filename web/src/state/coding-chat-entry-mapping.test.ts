import { describe, expect, it } from "vitest";
import type { CodingChatEntry } from "../api/types";
import { codingChatEntryToChatEntry } from "./coding-chat-entry-mapping";

function makeCodingEntry(overrides: Partial<CodingChatEntry>): CodingChatEntry {
  return {
    id: "coding_unit_run_0001_delivery_facts",
    attempt_id: "coding_attempt_0001",
    node_id: null,
    role: "system",
    entry_type: { type: "system_event", event_type: "delivery_facts", message: "交付事实(平台生成)" },
    content: "Work Item work_item_0001 交付完成",
    metadata: null,
    created_at: "2026-10-14T00:00:00Z",
    ...overrides,
  };
}

describe("codingChatEntryToChatEntry", () => {
  it("maps platform delivery facts frames to delivery_facts entries", () => {
    const entry = makeCodingEntry({
      metadata: {
        source: "platform_delivery_facts",
        unit_id: "coding_unit_0001",
        unit_run_id: "coding_unit_run_0001",
        logical_work_item_id: "work_item_0001",
        start_commit: "startsha",
        completion_commit: "completionsha",
        changed_files: ["src/lib.rs"],
        verification: {
          review_report_id: "code_review_report_0001",
          verdict: "approve",
          tested_evidence_refs: ["evidence://tests/cargo-test"],
        },
        handoff_revision_id: "handoff_revision_coding_unit_run_0001",
      },
    });

    const mapped = codingChatEntryToChatEntry(entry);

    expect(mapped.id).toBe("coding_unit_run_0001_delivery_facts");
    expect(mapped.type).toBe("delivery_facts");
    expect(mapped.role).toBe("system");
    expect(mapped.metadata?.source).toBe("platform_delivery_facts");
    expect(mapped.metadata?.completion_commit).toBe("completionsha");
    expect(mapped.metadata?.changed_files).toEqual(["src/lib.rs"]);
    expect(mapped.metadata?.handoff_revision_id).toBe("handoff_revision_coding_unit_run_0001");
  });

  it("keeps ordinary system events as execution_event entries", () => {
    const entry = makeCodingEntry({
      entry_type: { type: "system_event", event_type: "runner_started", message: "runner started" },
    });

    expect(codingChatEntryToChatEntry(entry).type).toBe("execution_event");
  });
});
