// owner 2026-10-30 真机反馈：门卡写着「✓ 可直接确认（需先处理预判）」「引擎判定
// 人工 + 已回复 1 轮 + 剩余修复 2 次」，owner「不知道如何往下继续」。本模块是门卡
// 的「现在请做 X」状态指引与机器文案人话化的单一事实源：
//
// - 后端 metadata 投递的机器短语（如「预判」「validation_reject」在前端做**片段级**
//   处理：已知语义映射替换、未解释术语只摘除命中片段——句中其余事实原样保留
//   （k3：整段兜底会把真实拒收原因一并丢掉，与 REQ-WSC-14「可读原因」相抵）。
// - 被拒原因不套黑话兜底（jargon: "keep"）：已知机器码仍映射，未解释术语逐字
//   保留——那是 owner 必读的事实，措辞美观让位于信息完整。
// - 门卡顶部一行按状态分支的明确指引（选什么 / 写什么 / 点哪个按钮）。
// - 「确认当前版本」的后果说明与禁用原因。
//
// 只读 state 层常量（gateActionBlockCopy 等），不改 state/api-types——那些是后端
// agent 的写面。
import type { WorkItemPlanHumanGateSnapshot } from "../../../api/types";
import {
  gateActionBlockCopy,
  type GateActionBlockReason,
} from "../../../state/workspace-cockpit-projection";

/**
 * 已确认语义的机器短语映射（后端 metadata → owner 能懂的说法）——只替换
 * 命中的片段，句中其余文字不动。
 */
const GATE_MACHINE_PHRASE_MAP: ReadonlyArray<{ machine: RegExp; human: string }> = [
  { machine: /validation_reject/g, human: "回复未通过校验" },
];

/**
 * 出现即视为「未解释的内部术语」——前端没有可靠语义可不猜，按片段摘除
 * （只删术语本身，不编造其含义）。「预判」来自 owner 2026-10-30 真机门卡
 * （后端运行时投递，本仓前后端均无此常量）。
 */
const GATE_JARGON_TOKENS: readonly string[] = ["预判"];

/** humanizeGateProse 对未解释术语的处置策略。 */
export type GateProseJargonPolicy =
  /** 默认：按片段摘除——卡面不出现黑话，句中其余事实保留。 */
  | "mask-fragment"
  /**
   * REQ-WSC-14「可读原因」：被拒原因专用——不摘除、不兜底，原文为准
   * （已知机器码仍映射；黑话逐字保留，由拒收块的红框语义承载）。
   */
  | "keep";

/**
 * 门卡 owner 可读正文的人话化（片段级）：已知机器短语映射为确认语义；未解释
 * 术语按策略处置。绝不整段替换——真实信息（拒收原因/结论）优先于措辞美观。
 */
export function humanizeGateProse(
  text: string,
  options?: { jargon?: GateProseJargonPolicy },
): string {
  let result = text;
  for (const { machine, human } of GATE_MACHINE_PHRASE_MAP) {
    result = result.replace(machine, human);
  }
  if (options?.jargon !== "keep") {
    for (const token of GATE_JARGON_TOKENS) {
      result = result.split(token).join("");
    }
  }
  return result;
}


/**
 * 阻断原因人话：terminal_stage/closed 沿用 state 层既有文案（页面级测试与收件箱
 * 同口径）；phase_mismatch 的「门相位与当前阶段不一致」是 0009 现场误导源，改为
 * 陈述事实的 owner 措辞（配合禁用按钮的原因呈现，不再无声消失）。
 */
export function gateBlockReasonHuman(reason: Exclude<GateActionBlockReason, null>): string {
  if (reason === "phase_mismatch") {
    return "会话已进入下一阶段，这扇门不再接收操作";
  }
  return gateActionBlockCopy(reason);
}

/** 「确认当前版本」按钮的后果说明（反馈被拒时它是明确出路）。 */
export const GATE_CONFIRM_CONSEQUENCE_REJECTED = "确认后将用当前候选继续，不再等你的修改";

/** 元数据行（gate-meta）的触发原因人话标签——计数保留，措辞 owner 化。 */
export const GATE_HUMAN_TRIGGER_LABELS: Record<
  WorkItemPlanHumanGateSnapshot["trigger"],
  string
> = {
  native_human_required: "引擎自动评审拿不准，需要你来定",
  repeated_fingerprint: "同一问题又出现了，需要你来定",
  verification_new_findings: "复验又发现新问题，需要你来定",
  repair_budget_exhausted: "自动修复次数已用完，需要你来定",
};

/** 指引行前缀（owner 第一眼定位「现在该做什么」）。 */
export const GATE_NEXT_STEP_PREFIX = "现在请做：";

export const GATE_NEXT_STEP_REJECTED =
  "你上一条回复没被采纳（原因见下方红框）：请改写后在下方重新提交，或直接点「确认当前版本」用当前候选继续";

export const GATE_NEXT_STEP_REVISION_RUNNING =
  "引擎正在按你的反馈修订并复评，请稍候；期间也可直接点「确认当前版本」用当前候选继续";

export const GATE_NEXT_STEP_BATCH_CONFIRM =
  "确认发布后整组 Work Item 将进入执行阶段，此门关闭";

export const GATE_NEXT_STEP_CONTEXT_BLOCKER =
  "请在下方输入缺失的上下文后发送即可继续；无法补充时终止此门";

export const GATE_NEXT_STEP_OWNER_QUESTION =
  "请从上面「需要你决定一件事」里选一个选项（把选项名原文作为补充指令发回），或直接写一两句说明后提交";

export function gateNextStepRequiredCopy(requiredCount: number): string {
  return `还有 ${requiredCount} 条必须处理：请在下方写明要怎么改后提交；不归你管的条目可逐条「排除此项」，或直接点「确认当前版本」继续`;
}

/**
 * 「仅建议」分支指引（无必须处理项）：advisory（范围外事实）与建议级
 * actionable（severity=suggestion 等非阻断）共用——两者都不阻断、都可经
 * 「采纳建议为反馈」处置，但计数归因各自独立（见 GateNextStepInput）。
 */
export const GATE_NEXT_STEP_ADVISORY_ONLY =
  "没有必须处理项：直接点「确认当前版本」即可继续；想采纳建议就点「采纳建议为反馈」再提交";

export const GATE_NEXT_STEP_VERDICT_ONLY =
  "确认当前版本即可继续；要改就在下方写一两句说明后提交";

export function gateExcludedSuffix(excludedCount: number): string {
  return `；你已排除 ${excludedCount} 条，下一轮不会再报为需处理`;
}

/** 指引分支输入（全部由门卡既有事实派生，零新增状态机）。 */
export type GateNextStepInput = {
  resolved: boolean;
  /** 反馈被拒（rejectedFeedback/rejectionReason 等 metadata 任一在场）。 */
  rejected: boolean;
  /** 修订进行中（typed turn busy/open）。 */
  revisionInProgress: boolean;
  batchConfirm: boolean;
  contextBlocker: boolean;
  actionBlockReason: Exclude<GateActionBlockReason, null> | null;
  /** 终止是否仍可用（阻断指引的出路措辞据此分支）。 */
  terminateAvailable: boolean;
  /** 带人话裁决（owner_question + ≥2 选项）。 */
  ownerQuestion: boolean;
  requiredFindings: number;
  /**
   * disposition=advisory（范围外事实）条数——「advisory」只认这个口径（k3：
   * 不得用「总数−必处理数」差值冒充，那会把建议级 actionable 计成 advisory）。
   */
  advisoryFindings: number;
  /** actionable 但非阻断（建议级）条数——不冒充 advisory，指引与 advisory 分支同句。 */
  optionalFindings: number;
  excludedFindings: number;
};

/**
 * 门卡「现在请做 X」：按状态取一句明确的下一步动作（不能只复述状态）。
 * 优先级：已收口不指引 → 被拒（owner 最需要知道出路）→ 修订中 → 批次确认 →
 * 上下文阻断 → 动作被阻断 → 人话裁决 → 必须处理项 → 仅建议（advisory 或
 * 建议级 actionable）→ 只有结论。
 */
export function gateNextStepGuidance(input: GateNextStepInput): string | null {
  if (input.resolved) {
    return null;
  }
  if (input.rejected) {
    return GATE_NEXT_STEP_REJECTED;
  }
  if (input.revisionInProgress) {
    return GATE_NEXT_STEP_REVISION_RUNNING;
  }
  if (input.batchConfirm) {
    return GATE_NEXT_STEP_BATCH_CONFIRM;
  }
  if (input.contextBlocker) {
    return GATE_NEXT_STEP_CONTEXT_BLOCKER;
  }
  if (input.actionBlockReason !== null) {
    return input.terminateAvailable
      ? `${gateBlockReasonHuman(input.actionBlockReason)}；此时只能终止此门后重新发起`
      : `${gateBlockReasonHuman(input.actionBlockReason)}；此卡不再接收操作，请以页面最新状态为准`;
  }
  let base: string;
  if (input.ownerQuestion) {
    base = GATE_NEXT_STEP_OWNER_QUESTION;
  } else if (input.requiredFindings > 0) {
    base = gateNextStepRequiredCopy(input.requiredFindings);
  } else if (input.advisoryFindings > 0 || input.optionalFindings > 0) {
    base = GATE_NEXT_STEP_ADVISORY_ONLY;
  } else {
    base = GATE_NEXT_STEP_VERDICT_ONLY;
  }
  if (input.excludedFindings > 0) {
    base += gateExcludedSuffix(input.excludedFindings);
  }
  return base;
}

/** 门卡顶部的「现在请做 X」行（置于原因行之前，owner 第一眼可扫到）。 */
export function GateNextStepGuidance({ copy }: { copy: string }) {
  return (
    <div
      data-testid="gate-next-step"
      className="text-sm font-semibold text-slate-900"
    >
      <span className="text-amber-700">{GATE_NEXT_STEP_PREFIX}</span>
      {copy}
    </div>
  );
}
