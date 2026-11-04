// 块1 片6（G-2 最终裁决）：三个额度池的呈现层单一事实源——只做「分别
// 命名 + 来源标注 + 两个账面事实分别呈现」，绝不合成「共享余额」：
// 后端尚未提供同域权威合并事实，现有代码也没有能算出真实共享余额的
// 公式（3−max(1,1)=2，但正确是 3−(1+1)=1）。共享余额仅在后端提供同域
// 权威事实时才可展示；当前 sharedBalance 恒为 null（结构上杜绝渲染）。
//
// 三个额度分别命名（MUST NOT 用「自动修复」统称——那是本次要消除的混称）：
// - B1 平台自动重写：candidate_compile_failure.auto_rewrite.remaining
//   （编译面自动重写账本）。
// - B2 人工返修：human_gate_snapshot.manual_repairs_remaining（门卡上
//   读创建时快照 remaining_budget，来源标注「门预算账本」）。
// - C 人工说明回合：human_gate_snapshot.accepted_feedback_turns
//   （「你已说明 N 轮」）。
// - A 机械返修池（max_repairs）不上界面额度名。
import { GATE_HUMAN_TRIGGER_LABELS } from "./gate-next-step";
import type { WorkItemPlanHumanGateSnapshot } from "../../../api/types";

/** B1 池界面名：平台自动重写。 */
export const GATE_QUOTA_NAME_AUTO_REWRITE = "平台自动重写";
/** B2 池界面名：人工返修。 */
export const GATE_QUOTA_NAME_MANUAL_REPAIR = "人工返修";
/** C 池界面名句式（「你已说明 N 轮」）。 */
export const GATE_QUOTA_ACCEPTED_TURNS_NAME = "你已说明";

/** B1 行来源标注（编译面自动重写账本）。 */
export const GATE_QUOTA_SOURCE_AUTO_REWRITE = "编译面自动重写账本";
/** B2 行来源标注（门预算账本）。 */
export const GATE_QUOTA_SOURCE_MANUAL_REPAIR = "门预算账本";

/** 6-B：两个账面事实的未对齐说明（只陈述事实，不承诺合并、不算余量）。 */
export const GATE_QUOTA_LEDGERS_UNMERGED_NOTE = "两处账本尚未合并";

/** 6-D：无法判定停止路径时的中性停止文案（成品）。 */
export const GATE_STOPPED_NEUTRAL_COPY = "本轮评审自动返修已停止，需要你决定下一步。";
/** 6-D：SC 流（flowKind === "single_candidate"）的固定停止标签。 */
export const GATE_STOPPED_SC_COPY = "本轮评审已停止自动返修，需要你来定下一步。";

/** 6-E：共池说明性文字（不承诺算术合并后的余量值、不宣称已并成一个池）。 */
export const GATE_QUOTA_SHARED_CAP_NOTE = "平台自动重写与你的反馈共用同一份额度上限";

/** 6-F：旧数据缺 gate-local 事实的保守呈现（不补默认值、不读成当前授权）。 */
export const GATE_QUOTA_HISTORY_UNAVAILABLE = "预算历史不可用／共享余额无法确认";

/** B2 池紧凑余量文案（无来源标注版，供收件箱/只读面板等紧凑呈现面）。 */
export function gateManualRepairRemainingCopy(remaining: number): string {
  return `${GATE_QUOTA_NAME_MANUAL_REPAIR}还可做 ${remaining} 次`;
}

/** C 池轮次行文案。 */
export function gateAcceptedTurnsCopy(turns: number): string {
  return `${GATE_QUOTA_ACCEPTED_TURNS_NAME} ${turns} 轮`;
}

/**
 * 6-D：repair_budget_exhausted 的停止标签按 flowKind 分流（MUST NOT 渲染
 * 为「次数已用完」——该 trigger 可由机械返修池耗尽、cycle initial/
 * verification 的结构封闭等不同路径产生）；其它 trigger 沿用门卡人话标签。
 */
export function gateStoppedTriggerLabel(trigger: string, flowKind: string | null): string {
  if (trigger === "repair_budget_exhausted") {
    return flowKind === "single_candidate" ? GATE_STOPPED_SC_COPY : GATE_STOPPED_NEUTRAL_COPY;
  }
  return (
    GATE_HUMAN_TRIGGER_LABELS[trigger as WorkItemPlanHumanGateSnapshot["trigger"]] ?? trigger
  );
}

/** 呈现层输入：三个池的只读事实（null = 该账本事实缺席，不猜）。 */
export interface GateQuotaPresentationInput {
  /** B1 平台自动重写余量（entry.metadata.candidate_compile_failure.auto_rewrite.remaining）。 */
  b1Remaining: number | null;
  /** B2 人工返修余量（门卡 remaining_budget 快照 ← manual_repairs_remaining）。 */
  b2Remaining: number | null;
  /** C 已接受人工说明回合数（gate-local；缺席 = 旧数据预算历史不可用）。 */
  acceptedTurns: number | null;
  /** 门触发原因（repair_budget_exhausted 按 6-D 分流）。 */
  trigger: string | null;
  /** 会话流凭据（"single_candidate" 触发 SC 停止标签；非 null = plan 流在场）。 */
  flowKind: string | null;
}

/**
 * 额度呈现纯函数：输出若干行文案 + 未对齐说明；sharedBalance 恒为 null
 * ——共享权威尚不存在，呈现层结构上不产生合并余额（6-C）。
 */
export function gateQuotaPresentation(input: GateQuotaPresentationInput): {
  lines: string[];
  note: string | null;
  sharedBalance: null;
} {
  const { b1Remaining, b2Remaining, acceptedTurns, trigger, flowKind } = input;
  const lines: string[] = [];
  if (trigger !== null) {
    lines.push(`为什么找你：${gateStoppedTriggerLabel(trigger, flowKind)}`);
  }
  if (acceptedTurns !== null && acceptedTurns > 0) {
    lines.push(gateAcceptedTurnsCopy(acceptedTurns));
  }
  // 6-F：plan 流在场（flowKind 非 null = 收到过带 flow 凭据的 SessionState）
  // 且 gate-local 轮次事实缺席 → 旧数据：B2 卡片快照可能不是当前授权，
  // 不呈现其数字、不补默认值，保守声明预算历史不可用。
  const ledgerHistoryUnavailable =
    acceptedTurns === null &&
    flowKind !== null &&
    (b2Remaining !== null || trigger !== null);
  if (b1Remaining !== null) {
    lines.push(
      `${GATE_QUOTA_NAME_AUTO_REWRITE}还可做 ${b1Remaining} 次（${GATE_QUOTA_SOURCE_AUTO_REWRITE}）`,
    );
  }
  if (!ledgerHistoryUnavailable && b2Remaining !== null) {
    lines.push(
      `${gateManualRepairRemainingCopy(b2Remaining)}（${GATE_QUOTA_SOURCE_MANUAL_REPAIR}）`,
    );
  }
  if (ledgerHistoryUnavailable) {
    lines.push(GATE_QUOTA_HISTORY_UNAVAILABLE);
  }
  const note =
    b1Remaining !== null && b2Remaining !== null && !ledgerHistoryUnavailable
      ? GATE_QUOTA_LEDGERS_UNMERGED_NOTE
      : null;
  return { lines, note, sharedBalance: null };
}
