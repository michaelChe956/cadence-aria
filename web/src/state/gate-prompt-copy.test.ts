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
  gateAdoptFindingsFeedback,
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
