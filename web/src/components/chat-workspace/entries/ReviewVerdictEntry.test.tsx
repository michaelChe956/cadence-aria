import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import { ReviewVerdictEntry } from "./ReviewVerdictEntry";

function verdictEntry(metadata: Record<string, unknown>): ChatEntry {
  return {
    id: "timeline_node_009:review-verdict",
    type: "review_verdict",
    role: "reviewer",
    content: "Review Round 4",
    timestamp: "2026-09-24T05:29:45.514Z",
    metadata,
  };
}

// F-50 视觉 v2 fix round：结论卡纳入视觉规格——面板色仍由本组件显式指定
// （F-49 A7 的「不留权重轮盘」纪律保留），但按 §1 规则改中性白底+琥珀左线
// （琥珀不再做整卡底色，复用门卡同一 GATE_CARD_CLASS 常量）。
describe("ReviewVerdictEntry panel tone", () => {
  it("结论卡与门卡同一视觉常量：中性白底+琥珀左线，不再整卡琥珀底", () => {
    render(<ReviewVerdictEntry entry={verdictEntry({ verdict: "pass", summary: "通过" })} />);

    const panel = screen.getByTestId("review-verdict-entry");
    expect(panel.className).toContain("bg-white");
    expect(panel.className).toContain("border-l-4");
    expect(panel.className).toContain("border-l-amber-500/60");
    expect(panel.className).not.toContain("bg-amber-50");
    expect(panel.className).not.toContain("border-green-200");
    expect(panel.className).not.toContain("bg-green-50");
  });

  it("keeps the severity colouring of the findings inside the panel (F-49 A7)", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({
          verdict: "revise",
          findings: [{ severity: "must_fix", message: "缺少验证命令" }],
        })}
      />,
    );

    expect(screen.getByText("需要解决")).toBeInTheDocument();
    expect(screen.getByText("高 · 必须修复")).toBeInTheDocument();
  });

  // F-50 fix round 标题去重：triage 场景「需要判断 reviewer 意图」只保留门卡卡头
  // 一处——结论卡自称「审核结论待人工分诊」，不再与门卡头同文案。
  it("triage 结论卡标题不与门卡头同文案（去重）", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({ verdict: "needs_human", review_gate: "user_triage_required" })}
      />,
    );

    expect(screen.getByText("审核结论待人工分诊")).toBeInTheDocument();
    expect(screen.queryByText("需要判断 reviewer 意图")).not.toBeInTheDocument();
  });
});
