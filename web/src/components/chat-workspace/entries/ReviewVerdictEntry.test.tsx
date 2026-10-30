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

// REQ-WSC-11（owner 可读的人工裁决）：needs_human 结论先给用户看得懂的问题与
// 选项（label + consequence + 一句指引），技术 findings 折叠进「技术细节」；
// pass/revise 呈现必须逐字不变。
const ownerGuidanceFindings = [
  {
    severity: "suggestion",
    message:
      "WI-004 的 write_policy.exclusive_scopes 仍声明 scripts/dev.sh，位于仓库根文件区",
    evidence: "exclusive_write_scopes=[scripts/dev.sh]",
    required_action: "由人工裁决 WI-004 归属模型：清空 exclusive_scopes，或把 scripts/ 纳入可绑定写范围",
    owner_question: "WI-004 还要不要继续负责 scripts/dev.sh 的改动？",
    owner_options: [
      { label: "不负责，只做验证", consequence: "WI-004 变成纯验证项，脚本改动另行安排" },
      { label: "继续负责", consequence: "把 scripts/ 纳入可写入范围，后续改动都记在 WI-004 上" },
    ],
  },
];

describe("ReviewVerdictEntry owner-readable needs_human", () => {
  it("needs_human 渲染人话问题、选项与后果，并给出作答指引", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({
          verdict: "needs_human",
          review_gate: "user_triage_required",
          summary: "需要人工确认",
          findings: ownerGuidanceFindings,
        })}
      />,
    );

    expect(screen.getByTestId("owner-question")).toHaveTextContent(
      "WI-004 还要不要继续负责 scripts/dev.sh 的改动？",
    );
    const options = screen.getAllByTestId("owner-option");
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveTextContent("不负责，只做验证");
    expect(options[0]).toHaveTextContent("WI-004 变成纯验证项，脚本改动另行安排");
    expect(options[1]).toHaveTextContent("继续负责");
    expect(options[1]).toHaveTextContent("把 scripts/ 纳入可写入范围，后续改动都记在 WI-004 上");
    expect(screen.getByTestId("owner-option-guidance")).toHaveTextContent(
      "把选项名原文作为补充指令发回去",
    );
  });

  it("技术 findings 折叠进「技术细节」，不在主区原样透传", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({
          verdict: "needs_human",
          review_gate: "user_triage_required",
          findings: ownerGuidanceFindings,
        })}
      />,
    );

    const details = screen.getByTestId("review-technical-details");
    expect(details.tagName).toBe("DETAILS");
    expect(details).not.toHaveAttribute("open");
    expect(details).toHaveTextContent("技术细节");
    expect(details).toHaveTextContent("exclusive_write_scopes=[scripts/dev.sh]");
    expect(details).toHaveTextContent("由人工裁决 WI-004 归属模型");
    // 主区（details 之外）不能出现技术原文。
    expect(screen.getByTestId("review-verdict-entry")).toHaveTextContent(
      "WI-004 还要不要继续负责",
    );
  });

  it("pass 结论（不携带人话字段）不出现人话裁决区，findings 直接展示（零回归）", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({
          verdict: "pass",
          summary: "通过",
          findings: [{ severity: "suggestion", message: "补充 README 截图" }],
        })}
      />,
    );

    expect(screen.queryByTestId("owner-question")).not.toBeInTheDocument();
    expect(screen.queryByTestId("review-technical-details")).not.toBeInTheDocument();
    expect(screen.getByText("可选建议")).toBeInTheDocument();
  });

  it("revise 结论不出现人话裁决区，findings 直接展示（零回归）", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({
          verdict: "revise",
          summary: "需要返修",
          findings: [{ severity: "must_fix", message: "缺少验证命令" }],
        })}
      />,
    );

    expect(screen.queryByTestId("owner-question")).not.toBeInTheDocument();
    expect(screen.queryByTestId("review-technical-details")).not.toBeInTheDocument();
    expect(screen.getByText("缺少验证命令")).toBeInTheDocument();
  });
});

describe("ReviewVerdictEntry owner guidance visibility (F2)", () => {
  it("宣称 needs_human 被 blocking finding 归一成 revise 时，人话区仍可见", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({
          verdict: "revise",
          review_gate: "requires_revision",
          summary: "需要返修",
          findings: ownerGuidanceFindings,
        })}
      />,
    );

    expect(screen.getByTestId("owner-question")).toBeInTheDocument();
    expect(screen.getAllByTestId("owner-option")).toHaveLength(2);
    expect(screen.getByTestId("review-technical-details")).toBeInTheDocument();
  });

  it("只有 1 个选项时按 fail-closed 下限不渲染人话区（与服务端同口径）", () => {
    render(
      <ReviewVerdictEntry
        entry={verdictEntry({
          verdict: "needs_human",
          findings: [
            {
              severity: "suggestion",
              message: "m",
              owner_question: "要不要继续？",
              owner_options: [{ label: "继续", consequence: "保持当前拆分" }],
            },
          ],
        })}
      />,
    );

    expect(screen.queryByTestId("owner-guidance")).not.toBeInTheDocument();
  });
});
