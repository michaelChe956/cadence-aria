import { describe, expect, it } from "vitest";
import { selectLatestReviewReport } from "./workspace-ws-selectors";
import type { WorkspaceWsState, TimelineNode } from "./workspace-ws-store-types";
import type { ChatEntry } from "./chat-entries";

function stateWithEntries(
  entries: ChatEntry[],
  timelineNodes: TimelineNode[] = [],
): WorkspaceWsState {
  return { chatEntries: entries, timelineNodes } as unknown as WorkspaceWsState;
}

function timelineNode(
  node_id: string,
  node_type: TimelineNode["node_type"],
  status: TimelineNode["status"] = "completed",
): TimelineNode {
  return {
    node_id,
    node_type,
    status,
    stage: node_type === "revision" ? "revision" : "cross_review",
    title: node_type,
    started_at: "2026-08-17T03:00:00.000Z",
    completed_at: "2026-08-17T03:00:00.000Z",
    provider_config_snapshot: {
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    },
  };
}

function revisionEntry(): ChatEntry {
  return {
    id: "revision:stream",
    type: "provider_stream",
    role: "author",
    content: "已按反馈产出新版本",
    timestamp: "2026-08-17T03:00:00.000Z",
    node_id: "revision",
  };
}

function reviewVerdictEntry(metadata?: Record<string, unknown>): ChatEntry {
  return {
    id: "review-verdict:1",
    type: "review_verdict",
    role: "reviewer",
    content: "遗漏边界写入范围",
    timestamp: "2026-08-17T03:00:00.000Z",
    node_id: "reviewer",
    metadata,
  };
}

describe("selectLatestReviewReport", () => {
  it("含 findings metadata 时返回完整报告（含可选建议明细）", () => {
    const state = stateWithEntries([
      reviewVerdictEntry({
        summary: "遗漏边界写入范围",
        comments: "需要覆盖所有 match。",
        verdict: "revise",
        findings: [
          {
            severity: "must_fix",
            message: "ProviderName 扩展遗漏 match 分支",
            evidence: "src/types.rs:86",
            impact: "新增 provider 时遗漏运行时映射。",
            required_action: "纳入写入范围。",
          },
          {
            severity: "suggestion",
            message: "可选：补充空输入边界说明",
            evidence: "spec.md:12",
            impact: "可读性",
            required_action: "补一行说明。",
          },
        ],
      }),
    ]);
    const report = selectLatestReviewReport(state);
    expect(report).toContain("[review_summary]");
    expect(report).toContain("遗漏边界写入范围");
    expect(report).toContain("[review_comments]");
    expect(report).toContain("需要覆盖所有 match。");
    expect(report).toContain("[review_findings]");
    // 必须修复明细
    expect(report).toContain("ProviderName 扩展遗漏 match 分支");
    expect(report).toContain("src/types.rs:86");
    expect(report).toContain("纳入写入范围。");
    // 可选建议明细也必须带上
    expect(report).toContain("suggestion");
    expect(report).toContain("可选：补充空输入边界说明");
    expect(report).toContain("spec.md:12");
  });

  it("无 findings metadata 时 fallback 到消息 content", () => {
    const state = stateWithEntries([reviewVerdictEntry(undefined)]);
    expect(selectLatestReviewReport(state)).toBe("遗漏边界写入范围");
  });

  it("最新 review 后已有完成的 revision 时不返回旧报告", () => {
    const state = stateWithEntries(
      [reviewVerdictEntry(), revisionEntry()],
      [
        timelineNode("reviewer", "reviewer_run"),
        timelineNode("revision", "revision"),
      ],
    );

    expect(selectLatestReviewReport(state)).toBeUndefined();
  });

  it("最新 review 后尚无 revision 时仍返回报告", () => {
    const state = stateWithEntries(
      [revisionEntry(), reviewVerdictEntry()],
      [
        timelineNode("revision", "revision"),
        timelineNode("reviewer", "reviewer_run"),
      ],
    );

    expect(selectLatestReviewReport(state)).toBe("遗漏边界写入范围");
  });

  it("无 review_verdict entry 时返回 undefined", () => {
    expect(selectLatestReviewReport(stateWithEntries([]))).toBeUndefined();
  });
});
