// F-49 B6：「采纳建议为反馈」的格式化模板（常量与纯函数集中在 gate-prompt-copy.ts）。
// 对象=finding.message、建议动作=finding.required_action——门卡 metadata findings 的
// 结构化文本位就这两个（见 workspace-ws-store-types ReviewFinding，与 B3 渲染同源）；
// 建议缺失即字段不足，整条原文降级拼入。must_fix 不在拼接范围（处理路径不同），
// 由调用方过滤。
import { describe, expect, it } from "vitest";
import {
  GATE_ADOPT_FINDINGS_BUTTON_LABEL,
  GATE_ADOPT_FINDINGS_LEAD,
  GATE_ADOPT_FINDINGS_TRAIL,
  GATE_FEEDBACK_OPTIONAL_HINT,
  GATE_TERMINATE_BUTTON_LABEL,
  GATE_TERMINATE_CONFIRM_LABEL,
  GATE_TRIAGE_WHY_COPY,
  gateAdoptFindingsFeedback,
  gateWhyAdvisoryCopy,
  gateWhyRequiredCopy,
  isGateTitleSynonymousCopy,
} from "./gate-prompt-copy";

describe("gate-prompt-copy adopt findings (F-49 B6)", () => {
  it("formats structured findings as a single-line revision instruction", () => {
    const feedback = gateAdoptFindingsFeedback([
      { message: "复杂度说明", required_action: "补充复杂度说明" },
      { message: "验收标准", required_action: "补可度量判据" },
    ]);

    expect(GATE_ADOPT_FINDINGS_LEAD).toBe("按复评建议修订以下内容：");
    expect(GATE_ADOPT_FINDINGS_TRAIL).toBe("其余内容保持不变。");
    // 门卡反馈框是单行 input（HTML value 净化剥换行），模板按单行串接、所见即所交。
    expect(feedback).toBe(
      "按复评建议修订以下内容：复杂度说明：补充复杂度说明；验收标准：补可度量判据；其余内容保持不变。",
    );
  });

  it("degrades a finding without required_action to its original message", () => {
    expect(gateAdoptFindingsFeedback([{ message: "统一命名" }])).toBe(
      "按复评建议修订以下内容：统一命名；其余内容保持不变。",
    );
  });

  it("keeps the adopt button label copy stable", () => {
    expect(GATE_ADOPT_FINDINGS_BUTTON_LABEL).toBe("采纳建议为反馈");
  });
});

// F-50 裁决 1/2/3/4（第二批语义文案）：单一原因行（含下一步建议，「机械校验
// 0 error」并入前半句）；反馈与确认并行的帮助文案；终止按钮按门级作用域命名；
// content/summary 与标题同义时由组件侧去重。
describe("gate-prompt-copy F-50 semantics", () => {
  it("advisory reason line merges the 0-error fact as its first clause", () => {
    expect(gateWhyAdvisoryCopy(2)).toBe(
      "原因：机械校验 0 error，复评有 2 条建议（不阻断发布）；可直接确认，或提交反馈后再修订。",
    );
  });

  it("required reason line keeps the must-fix count and the feedback advice", () => {
    expect(gateWhyRequiredCopy(1)).toBe("原因：有 1 条必须处理项；建议先提交反馈。");
  });

  it("keeps the triage/intent reason line stable", () => {
    expect(GATE_TRIAGE_WHY_COPY).toBe(
      "原因：评审结果无法自动取舍；请选择确认当前版本或反馈修改。",
    );
  });

  it("keeps the optional-feedback help copy stable", () => {
    expect(GATE_FEEDBACK_OPTIONAL_HINT).toBe(
      "如需调整，请在反馈框填写修改意见；确认当前版本无需填写。",
    );
  });

  it("names the terminate buttons with the gate scope (abandon_human_gate)", () => {
    expect(GATE_TERMINATE_BUTTON_LABEL).toBe("终止此门");
    expect(GATE_TERMINATE_CONFIRM_LABEL).toBe("确认终止此门");
  });

  it("detects title-synonymous gate copy across spacing and synonyms", () => {
    for (const synonymous of [
      "需要人工确认",
      "等待人工确认",
      "需人工确认",
      "人工确认",
      "可确认当前版本",
      "需要判断 reviewer 意图",
      " 需要 人工确认 ",
    ]) {
      expect(isGateTitleSynonymousCopy(synonymous)).toBe(true);
    }
    expect(isGateTitleSynonymousCopy("复评仍有 2 条建议")).toBe(false);
    expect(isGateTitleSynonymousCopy("缺少上下文")).toBe(false);
  });
});
