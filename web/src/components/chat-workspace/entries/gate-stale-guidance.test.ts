// REQ-HGC-04：过期门卡分流表（单一事实源）的单元测试。
//
// 钉住三件事：
// ① 分流以 **durable 会话状态**为主键（不凭「卡过期了」推断）；
// ② 非过期卡（actionBlockReason === null）的「终止此门」按钮**不得消失**
//    （owner 正在用的动作面）；
// ③ 无真实续跑入口时**不得**给出「继续自动化」假指引。
import { describe, expect, it } from "vitest";
import type { WorkspaceSessionStatus } from "../../../api/types";
import type { GateActionBlockReason } from "../../../state/workspace-cockpit-projection";
import {
  GATE_STALE_ADVANCED_COPY,
  GATE_STALE_COMPLETED_COPY,
  GATE_STALE_FAILED_COPY,
  GATE_STALE_STOPPED_CONTINUE_COPY,
  GATE_STALE_STOPPED_NO_ENTRY_COPY,
  GATE_STALE_TERMINATED_COPY,
  staleGateGuidance,
} from "./gate-stale-guidance";

const STALE_REASON: GateActionBlockReason = "phase_mismatch";

describe("staleGateGuidance（REQ-HGC-04 过期门卡分流）", () => {
  it("按 durable 会话状态逐条分流（六个状态 × 入口事实）", () => {
    // confirmed：历史记录，无需处理，也不给「终止此门」。
    const confirmed = staleGateGuidance("confirmed", STALE_REASON, true);
    expect(confirmed.copy).toBe(GATE_STALE_COMPLETED_COPY);
    expect(confirmed.action).toBe("none");
    expect(confirmed.showTerminate).toBe(false);

    // terminated：历史记录，无需处理，也不给「终止此门」。
    const terminated = staleGateGuidance("terminated", STALE_REASON, true);
    expect(terminated.copy).toBe(GATE_STALE_TERMINATED_COPY);
    expect(terminated.action).toBe("none");
    expect(terminated.showTerminate).toBe(false);

    // stopped_needs_human + 确有入口：指向底部输入框 +「继续自动化」。
    const stoppedWithEntry = staleGateGuidance("stopped_needs_human", STALE_REASON, true);
    expect(stoppedWithEntry.copy).toBe(GATE_STALE_STOPPED_CONTINUE_COPY);
    expect(stoppedWithEntry.copy).toContain("输入框");
    expect(stoppedWithEntry.copy).toContain("继续自动化");
    expect(stoppedWithEntry.action).toBe("continue");
    expect(stoppedWithEntry.showTerminate).toBe(false);

    // stopped_needs_human + 无入口（false/null）：不得给假指引。
    for (const available of [false, null] as const) {
      const stoppedNoEntry = staleGateGuidance("stopped_needs_human", STALE_REASON, available);
      expect(stoppedNoEntry.copy).toBe(GATE_STALE_STOPPED_NO_ENTRY_COPY);
      expect(stoppedNoEntry.copy).not.toContain("继续自动化");
      expect(stoppedNoEntry.copy).toContain("刷新页面");
      expect(stoppedNoEntry.action).toBe("none");
      expect(stoppedNoEntry.showTerminate).toBe(false);
    }

    // failed：指向「继续自动化」重试，并说明可附补充说明。
    const failed = staleGateGuidance("failed", STALE_REASON, true);
    expect(failed.copy).toBe(GATE_STALE_FAILED_COPY);
    expect(failed.copy).toContain("继续自动化");
    expect(failed.copy).toContain("说明");
    expect(failed.action).toBe("continue");
    expect(failed.showTerminate).toBe(false);

    // 其余（推进到别的门）：以最新门卡为准，「终止此门」是可选出口。
    const advanced = staleGateGuidance("running", STALE_REASON, true);
    expect(advanced.copy).toBe(GATE_STALE_ADVANCED_COPY);
    expect(advanced.copy).toContain("以页面最新的一张门卡为准");
    expect(advanced.action).toBe("terminate_optional");
    expect(advanced.showTerminate).toBe(true);
  });

  it("非过期卡保留「终止此门」（owner 正在用的按钮不得消失）", () => {
    for (const status of [
      "open",
      "running",
      "waiting_for_human",
      "confirmed",
      "terminated",
      "stopped_needs_human",
      "failed",
      "change_requested",
    ] as WorkspaceSessionStatus[]) {
      const guidance = staleGateGuidance(status, null, true);
      expect(guidance.showTerminate).toBe(true);
      expect(guidance.action).toBe("terminate_optional");
    }
  });

  it("无真实续跑入口时不指引去点「继续自动化」", () => {
    const guidance = staleGateGuidance("stopped_needs_human", STALE_REASON, null);
    expect(guidance.copy).not.toContain("继续自动化");
    expect(guidance.showTerminate).toBe(false);
  });

  it("确认完成的既有文案逐字不变（历史卡不回归）", () => {
    expect(GATE_STALE_COMPLETED_COPY).toBe("这一版已确认完成，此卡是历史记录，无需处理");
    expect(staleGateGuidance("confirmed", STALE_REASON, true).copy).toBe(
      GATE_STALE_COMPLETED_COPY,
    );
  });

  it("推进到别的门时终止不是唯一出路", () => {
    const guidance = staleGateGuidance("open", STALE_REASON, true);
    expect(guidance.showTerminate).toBe(true);
    expect(guidance.copy).toContain("以页面最新的一张门卡为准");
    expect(guidance.copy).not.toContain("只能终止此门");
  });

  it("durable 状态未知（null）时按「推进到别的门」兜底且保留终止出口", () => {
    const guidance = staleGateGuidance(null, STALE_REASON, true);
    expect(guidance.copy).toBe(GATE_STALE_ADVANCED_COPY);
    expect(guidance.showTerminate).toBe(true);
  });
});
