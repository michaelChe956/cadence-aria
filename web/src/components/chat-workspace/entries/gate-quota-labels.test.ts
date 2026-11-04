// 块1 片6（G-2 最终裁决 §4.1 六条验收场景）：三个额度池呈现层的纯函数
// 测试。核心口径：三个额度分别命名；余量必须分别呈现两个账面事实，
// 绝不合成「共享余额」——后端未提供同域权威合并事实，现有代码也没有能
// 算出真实共享余额的公式（3−max(1,1)=2 但正确是 3−(1+1)=1）。
import { describe, expect, it } from "vitest";

import {
  GATE_QUOTA_HISTORY_UNAVAILABLE,
  GATE_QUOTA_LEDGERS_UNMERGED_NOTE,
  gateQuotaPresentation,
} from "./gate-quota-labels";

describe("gateQuotaPresentation（G-2 额度呈现）", () => {
  it("场景1：B1/B2 余量分别呈现两个账面事实＋未对齐说明，不合成共享余额", () => {
    const result = gateQuotaPresentation({
      b1Remaining: 2,
      b2Remaining: 3,
      acceptedTurns: 0,
      trigger: "native_human_required",
      flowKind: null,
    });

    expect(result.lines).toContain("平台自动重写还可做 2 次（编译面自动重写账本）");
    expect(result.lines).toContain("人工返修还可做 3 次（门预算账本）");
    expect(result.note).toBe(GATE_QUOTA_LEDGERS_UNMERGED_NOTE);
    expect(result.sharedBalance).toBeNull();

    const rendered = [...result.lines, result.note ?? ""].join(" ");
    // 不出现「共享余额」字样（共享权威尚不存在，呈现层无此概念）。
    expect(rendered).not.toContain("共享余额");
    // 不出现 2/3 之外的任何合并数字（「5 次」是 2+3 的合成痕迹）。
    expect(rendered).not.toContain("5 次");
    expect(rendered).not.toContain("共 5");
    expect(rendered).not.toContain("余额 5");
  });

  it("场景2：结构阻断路径的 repair_budget_exhausted 不渲染为「次数用完」，不补造 A 池计数", () => {
    const result = gateQuotaPresentation({
      b1Remaining: null,
      b2Remaining: null,
      acceptedTurns: null,
      trigger: "repair_budget_exhausted",
      flowKind: null,
    });

    const rendered = result.lines.join(" ");
    expect(rendered).toContain("本轮评审自动返修已停止，需要你决定下一步。");
    expect(rendered).not.toContain("用完");
    expect(rendered).not.toContain("用尽");
    expect(rendered).not.toContain("耗尽");
    // 不补造机械返修池（A 池）计数为 0。
    expect(rendered).not.toContain("0 次");
    expect(result.sharedBalance).toBeNull();
  });

  it("场景2b：SC 流（single_candidate）的停止标签按固定文案分流", () => {
    const result = gateQuotaPresentation({
      b1Remaining: null,
      b2Remaining: null,
      acceptedTurns: null,
      trigger: "repair_budget_exhausted",
      flowKind: "single_candidate",
    });

    const rendered = result.lines.join(" ");
    expect(rendered).toContain("本轮评审已停止自动返修，需要你来定下一步。");
    expect(rendered).not.toContain("用完");
    expect(rendered).not.toContain("用尽");
  });

  it("场景3：B1 因同环重复停止且余量为正时，停止原因与正余额同时保留", () => {
    const result = gateQuotaPresentation({
      b1Remaining: 2,
      b2Remaining: null,
      acceptedTurns: null,
      trigger: "repair_budget_exhausted",
      flowKind: null,
    });

    const rendered = result.lines.join(" ");
    // 停止事实（同环重复提前停，额度未耗尽）与正余额两行都在，不互相覆盖。
    expect(rendered).toContain("本轮评审自动返修已停止");
    expect(result.lines).toContain("平台自动重写还可做 2 次（编译面自动重写账本）");
    // 不因 trigger 是 exhausted 就把 B1 归零或隐藏。
    expect(rendered).not.toContain("0 次");
  });

  it("场景4：同门反馈受理后如实呈现新账面（M:3→2、C:0→1），不回填旧值", () => {
    const before = gateQuotaPresentation({
      b1Remaining: null,
      b2Remaining: 3,
      acceptedTurns: 0,
      trigger: "native_human_required",
      flowKind: null,
    });
    const after = gateQuotaPresentation({
      b1Remaining: null,
      b2Remaining: 2,
      acceptedTurns: 1,
      trigger: "native_human_required",
      flowKind: null,
    });

    expect(before.lines.join(" ")).toContain("人工返修还可做 3 次");
    expect(after.lines).toContain("人工返修还可做 2 次（门预算账本）");
    expect(after.lines).toContain("你已说明 1 轮");
    // 受理后的呈现不回填受理前的旧余量（重建/重连回填问题在快照层，
    // 本片只改呈现——呈现层的责任是只呈现传入的新事实）。
    expect(after.lines.join(" ")).not.toContain("3 次");
  });

  it("场景5：旧数据缺 gate-local 事实时保守呈现，不补默认值、不读成当前授权", () => {
    const result = gateQuotaPresentation({
      b1Remaining: null,
      b2Remaining: 3,
      acceptedTurns: null,
      trigger: "native_human_required",
      flowKind: "legacy",
    });

    expect(result.lines).toContain(GATE_QUOTA_HISTORY_UNAVAILABLE);
    // 缺 gate-local 事实时不得把卡片创建时的快照数字当当前余量呈现。
    expect(result.lines.join(" ")).not.toContain("还可做 3 次");
    expect(result.lines.join(" ")).not.toContain("还可做 0 次");
  });

  it("场景6：共享权威尚不存在——呈现层结构上不产生合并余额", () => {
    const result = gateQuotaPresentation({
      b1Remaining: 2,
      b2Remaining: 3,
      acceptedTurns: 1,
      trigger: "native_human_required",
      flowKind: null,
    });

    // sharedBalance 恒为 null：结构上杜绝「共享余额」被渲染出来。
    expect(result.sharedBalance).toBeNull();
    expect(
      [...result.lines, result.note ?? ""].join(" "),
    ).not.toContain("共享余额");
  });
});
