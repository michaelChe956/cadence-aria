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

// F-49 A7：结论卡的 amber 面板与 ChatEntryContainer 的 reviewer 角色面板（green）
// 同级同权重——最终由 Tailwind 输出顺序决定，dist 实测 green 胜出（.border-amber-200
// 早于 .border-green-200、.bg-amber-50 早于 .bg-green-50），作者写的 amber 被静默
// 覆盖，结论卡与门卡系（slate）撞色。面板必须由本组件显式指定，不留权重轮盘。
describe("ReviewVerdictEntry panel tone", () => {
  it("renders the amber review panel instead of the green reviewer role panel (F-49 A7)", () => {
    render(<ReviewVerdictEntry entry={verdictEntry({ verdict: "pass", summary: "通过" })} />);

    const panel = screen.getByTestId("review-verdict-entry");
    expect(panel.className).toContain("border-amber-200");
    expect(panel.className).toContain("bg-amber-50");
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
});
