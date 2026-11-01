// C2 human-gate-convergence（REQ-HGC-03 场景 1/F-52 §三）：finding 双轨分类
// 透明——severity（用户语义）与 effective class（class_hint 优先的策略语义）
// 同时呈现；二者不一致时不得仅显示其一造成「建议不阻断」误导。
//
// REQ-WSC-12/13（bound-review-findings-to-work-item-scope）：finding 的 WI
// 范围 disposition 呈现与逐条「排除此项」——advisory（指向本 Work Item 范围
// 外的事实）只作「仅供参考」提示：不进「需要解决」组、不计数、不得呈现为
// 要求作者执行；排除身份一律用平台算的 findingKey（前端不自算）。
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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

describe("REQ-WSC-12 advisory disposition presentation", () => {
  it("keeps advisory findings out of the required group even when severity and class hint disagree", () => {
    render(
      <ReviewFindingGroups
        findings={[
          finding({
            severity: "must_fix",
            class_hint: "repairable",
            disposition: "advisory",
            message: "仓库里前端规范文档的缺口需回写",
          }),
        ]}
      />,
    );

    // 平台硬规则的前端镜像：disposition=advisory 即使 class_hint/severity 判
    // 阻断，也不进「需要解决」，只落「仅供参考（不阻断）」组。
    expect(groupTitles()).not.toContain("需要解决");
    expect(groupTitles()).toContain("仅供参考（不阻断）");
    expect(screen.getByTestId("finding-advisory-badge").textContent).toBe("仅供参考");
    expect(screen.getByTestId("review-finding").textContent).toContain(
      "仓库里前端规范文档的缺口需回写",
    );
  });

  it("renders the advisory required action as a hint, not an imperative for the author", () => {
    render(
      <ReviewFindingGroups
        findings={[
          finding({
            disposition: "advisory",
            required_action: "提示：文档当前状态先保持原样",
          }),
        ]}
      />,
    );

    const action = screen.getByTestId("advisory-required-action");
    expect(action.textContent).toContain("提示：文档当前状态先保持原样");
    expect(action.className).not.toContain("font-medium");
  });

  it("treats a missing disposition as actionable for legacy records", () => {
    render(<ReviewFindingGroups findings={[finding({ severity: "must_fix" })]} />);

    expect(groupTitles()).toEqual(["需要解决"]);
    expect(screen.queryByTestId("finding-advisory-badge")).toBeNull();
  });

  it("groups actionable findings separately from advisory ones on the same card", () => {
    render(
      <ReviewFindingGroups
        findings={[
          finding({ severity: "must_fix", disposition: "actionable", message: "验收标准缺口" }),
          finding({
            severity: "suggestion",
            disposition: "advisory",
            message: "其它文档的当前状态",
          }),
        ]}
      />,
    );

    expect(groupTitles()).toEqual(["需要解决", "仅供参考（不阻断）"]);
  });
});

describe("REQ-WSC-13 per-finding exclusion affordance", () => {
  it("offers one-click exclusion only for findings carrying a platform finding key", () => {
    const onExcludeFinding = vi.fn(() => true);
    render(
      <ReviewFindingGroups
        findings={[
          finding({ severity: "must_fix", findingKey: "abc123def4560123", message: "范围外事实" }),
          finding({ severity: "must_fix", message: "旧记录无 key" }),
        ]}
        onExcludeFinding={onExcludeFinding}
      />,
    );

    fireEvent.click(screen.getByTestId("finding-exclude-button"));
    expect(onExcludeFinding).toHaveBeenCalledWith(
      expect.objectContaining({ findingKey: "abc123def4560123" }),
    );
    // 旧记录无 findingKey 不渲染排除按钮（fail-closed：无法 durable 引用）。
    expect(screen.getAllByTestId("finding-exclude-button")).toHaveLength(1);
  });

  it("hides the exclude button entirely when no sender is wired (read-only consumers)", () => {
    render(<ReviewFindingGroups findings={[finding({ findingKey: "abc123def4560123" })]} />);

    expect(screen.queryByTestId("finding-exclude-button")).toBeNull();
  });

  it("replaces the button with an excluded badge once the owner has excluded the finding", () => {
    render(
      <ReviewFindingGroups
        findings={[finding({ findingKey: "abc123def4560123" })]}
        onExcludeFinding={vi.fn(() => true)}
        excludedFindingKeys={new Set(["abc123def4560123"])}
      />,
    );

    expect(screen.queryByTestId("finding-exclude-button")).toBeNull();
    expect(screen.getByTestId("finding-excluded-badge").textContent).toContain("已排除");
  });
});
