// REQ-HGC-04：过期门卡按 durable 停点分流的**单一事实源**。
//
// 过期门卡（该门已不是当前需处理的门）按会话的 **durable 状态**分流：
// - confirmed / terminated → 历史记录，无需处理（无「终止此门」按钮）；
// - stopped_needs_human → 指向页面底部输入框 +「继续自动化」——**仅当服务端
//   说该入口此刻真实存在**（WS `plan_continue_available === true`，与 HTTP
//   control 的 `allowed_actions` 含 `continue_plan` 同源）；否则改指刷新页面
//   看最新状态或到自动化控制面处理（**不得给假指引**）；
// - failed → 指向「继续自动化」重试（并说明可附补充说明）；
// - 其余（推进到别的门）→ 以页面最新的一张门卡为准，并保留「终止此门」作为
//   **可选**出口（不得表述为唯一出路）。
//
// ⚠️ 非过期门卡（`actionBlockReason === null`）不在本表管辖：其「终止此门」
// 按钮必须保留（owner 正在用的动作面不得消失）——故本表对 `null` 一律返回
// `showTerminate: true`。
//
// 文案里引用的按钮名与页面真实按钮逐字一致：「继续自动化」= 底部输入框的
// 续跑按钮文案；「终止此门」= `GATE_TERMINATE_BUTTON_LABEL`。
import type { WorkspaceSessionStatus } from "../../../api/types";
import type { GateActionBlockReason } from "../../../state/workspace-cockpit-projection";

/**
 * 已确认完成的历史卡文案——**逐字沿用既有表述**（REQ-HGC-04：会话已确认完成
 * 的历史卡不回归）。原定义在 `gate-next-step.tsx`，本模块为分流的单一事实源，
 * 由 `gate-next-step.tsx` 再导出（既有导入面零变化）。
 */
export const GATE_STALE_COMPLETED_COPY =
  "这一版已确认完成，此卡是历史记录，无需处理";

/** 会话已终止 → 历史记录，无需处理。 */
export const GATE_STALE_TERMINATED_COPY =
  "这次自动化已经终止了，这张卡是历史记录，不用再处理。";

/** 停在等 owner 回话 + **确有**「继续自动化」入口 → 指向底部输入框 + 续跑按钮。 */
export const GATE_STALE_STOPPED_CONTINUE_COPY =
  "自动化停在等你回话的地方，这张卡已经过期了。请在页面下方的输入框里写一两句说明，然后点『继续自动化』—— 它会带上你的说明接着跑，不用重新开始。";

/** 停在等 owner 回话 + **无真实续跑入口** → 不给假指引，改指刷新/控制面。 */
export const GATE_STALE_STOPPED_NO_ENTRY_COPY =
  "自动化停下来了，但这张卡已经过期。自动化当前不接受新的输入，请刷新页面看最新状态，或到自动化控制面处理这一轮。";

/** 本轮失败 → 指向「继续自动化」重试（可附补充说明）。 */
export const GATE_STALE_FAILED_COPY =
  "这一轮自动化跑失败了，这张卡已经过期了。请在页面下方点『继续自动化』重试；也可以先写一两句说明，一并带给 AI。";

/** 推进到别的门 → 以最新门卡为准；「终止此门」保留为可选出口（非唯一出路）。 */
export const GATE_STALE_ADVANCED_COPY =
  "会话已经走到下一阶段了，这张门卡不再接收操作。请以页面最新的一张门卡为准；如果这一轮确实要作废，可以点下面的『终止此门』。";

/** 分流给出的动作指向。 */
export type StaleGateAction =
  /** 指向底部输入框 +「继续自动化」。 */
  | "continue"
  /** 无可用动作（历史记录 / 无真实续跑入口）。 */
  | "none"
  /** 以最新门卡为准，「终止此门」作为可选出口保留。 */
  | "terminate_optional";

/** 分流结果（文案 + 动作指向 + 是否保留「终止此门」）。 */
export type StaleGateGuidance = {
  copy: string;
  action: StaleGateAction;
  showTerminate: boolean;
};

/**
 * 过期门卡分流（单一事实源）。
 *
 * @param sessionStatus durable 会话状态（主键；null=未知）。
 * @param actionBlockReason 门卡被阻断的原因（**必带**：区分「已过期」与
 * 「正在等你拍板」——后者不在本表管辖，「终止此门」必须保留）。
 * @param planContinueAvailable 服务端权威事实：底部「继续自动化」入口此刻
 * 是否真实存在（WS `plan_continue_available`；null=未知，按「无真实续跑
 * 入口」处理，绝不给假指引）。
 */
export function staleGateGuidance(
  sessionStatus: WorkspaceSessionStatus | null,
  actionBlockReason: GateActionBlockReason,
  planContinueAvailable: boolean | null,
): StaleGateGuidance {
  // 非过期卡（owner 正在拍板）：不在分流表管辖——「终止此门」必须保留。
  if (actionBlockReason === null) {
    return {
      copy: GATE_STALE_ADVANCED_COPY,
      action: "terminate_optional",
      showTerminate: true,
    };
  }
  switch (sessionStatus) {
    case "confirmed":
      return {
        copy: GATE_STALE_COMPLETED_COPY,
        action: "none",
        showTerminate: false,
      };
    case "terminated":
      return {
        copy: GATE_STALE_TERMINATED_COPY,
        action: "none",
        showTerminate: false,
      };
    case "stopped_needs_human":
      return planContinueAvailable === true
        ? {
            copy: GATE_STALE_STOPPED_CONTINUE_COPY,
            action: "continue",
            showTerminate: false,
          }
        : {
            copy: GATE_STALE_STOPPED_NO_ENTRY_COPY,
            action: "none",
            showTerminate: false,
          };
    case "failed":
      return {
        copy: GATE_STALE_FAILED_COPY,
        action: "continue",
        showTerminate: false,
      };
    default:
      return {
        copy: GATE_STALE_ADVANCED_COPY,
        action: "terminate_optional",
        showTerminate: true,
      };
  }
}
