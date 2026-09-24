// C2 human-gate-convergence（REQ-HGC-03 场景 1/F-52 §三）：finding 双轨分类
// 透明——severity（用户语义）与 effective class（class_hint 优先的策略语义）
// 同时呈现；二者不一致时不得仅显示其一造成「建议不阻断」误导。
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ReviewFindingGroups } from "./finding-list";

const finding = (overrides: Record<string, unknown>) => ({
  severity: "suggestion",
  message: "CT-001 能力缺口",
  ...overrides,
});

function groupTitles() {
  return Array.from(screen.queryAllByTestId("review-finding-group-title")).map(
    (node) => node.textContent,
  );
}

describe("REQ-HGC-03 双轨分类显示", () => {
  it("shows effective class as primary with severity annotation when they disagree", () => {
    render(
      <ReviewFindingGroups
        findings={[finding({ severity: "suggestion", class_hint: "repairable" })]}
      />,
    );

    // 有效分类主显：需处理（repairable），进入阻断组——不再落入「可选建议」。
    expect(groupTitles()).toContain("需要解决");
    expect(groupTitles()).not.toContain("可选建议");
    const row = screen.getByTestId("review-finding");
    expect(row.textContent).toContain("需处理");
    // severity 标注同时在行上呈现（「建议」不得被吞）。
    expect(row.textContent).toContain("建议");
  });

  it("maps human_required to its own primary label and blocking group", () => {
    render(
      <ReviewFindingGroups
        findings={[finding({ severity: "suggestion", class_hint: "human_required" })]}
      />,
    );

    expect(groupTitles()).toContain("需要解决");
    const row = screen.getByTestId("review-finding");
    expect(row.textContent).toContain("需人工");
    expect(row.textContent).toContain("建议");
  });

  it("keeps advisory hints in the optional group without inventing blocking", () => {
    render(
      <ReviewFindingGroups
        findings={[finding({ severity: "suggestion", class_hint: "advisory" })]}
      />,
    );

    expect(groupTitles()).toContain("可选建议");
    expect(groupTitles()).not.toContain("需要解决");
  });

  it("falls back to severity when no class hint is present", () => {
    render(
      <ReviewFindingGroups
        findings={[finding({ severity: "must_fix" }), finding({ severity: "suggestion" })]}
      />,
    );

    expect(groupTitles()).toEqual(["需要解决", "可选建议"]);
    const rows = screen.getAllByTestId("review-finding");
    expect(rows[0].textContent).toContain("必须修复");
  });
});
