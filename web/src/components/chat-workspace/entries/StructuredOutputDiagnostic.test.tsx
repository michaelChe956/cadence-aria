import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StructuredOutputDiagnosticView } from "./StructuredOutputDiagnostic";

// F-50 视觉 v2（f50-ui-visual-spec-v2 §2）：卡内告警子卡改左线式——琥珀只上
// 4px 左线与图标，正文回 slate 系，不再琥珀底叠琥珀底；与正式门卡（决策卡）
// 视觉区分（告警=信息，门卡=决策）。
describe("StructuredOutputDiagnostic 视觉 v2", () => {
  const diagnostic = {
    code: "structured_output_parse_failed",
    repair_succeeded: false,
    repair_attempted: true,
    message: "invalid structured output json",
    raw_output_preview: "{oops",
  } as const;

  it("失败态：左线式（border-l-4 amber）+透明底，正文 slate，不再是琥珀底叠琥珀底", () => {
    render(
      <StructuredOutputDiagnosticView diagnostic={diagnostic} comments={null} />,
    );

    const alert = screen.getByRole("alert");
    expect(alert.className).toContain("border-l-4");
    expect(alert.className).toContain("border-l-amber-500/60");
    expect(alert.className).toContain("bg-transparent");
    expect(alert.className).not.toContain("bg-amber-50");
    const title = screen.getByText("结构化审核结果解析失败");
    expect(title.className).toContain("text-slate-900");
    expect(title.className).not.toContain("text-amber-950");
    // 图标保留琥珀语义。
    expect(alert.querySelector("svg.lucide-triangle-alert")?.getAttribute("class")).toContain(
      "text-amber-600",
    );
  });

  it("修复成功态维持 emerald 语义（成功不是告警，不改左线式）", () => {
    render(
      <StructuredOutputDiagnosticView
        diagnostic={{ ...diagnostic, repair_succeeded: true }}
        comments={null}
      />,
    );

    const success = screen.getByText("结构化输出已自动修复").closest("div")?.parentElement;
    expect(success?.className).toContain("border-emerald-200");
  });
});
