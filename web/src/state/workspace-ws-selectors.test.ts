import { describe, expect, it } from "vitest";
import { selectLatestReviewReport } from "./workspace-ws-selectors";
import type { WorkspaceWsState } from "./workspace-ws-store-types";
import type { ChatEntry } from "./chat-entries";

function stateWithEntries(entries: ChatEntry[]): WorkspaceWsState {
  return { chatEntries: entries } as unknown as WorkspaceWsState;
}

function reviewVerdictEntry(metadata?: Record<string, unknown>): ChatEntry {
  return {
    id: "review-verdict:1",
    type: "review_verdict",
    role: "reviewer",
    content: "遗漏边界写入范围",
    timestamp: "2026-08-17T03:00:00.000Z",
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

  it("无 review_verdict entry 时返回 undefined", () => {
    expect(selectLatestReviewReport(stateWithEntries([]))).toBeUndefined();
  });
});
