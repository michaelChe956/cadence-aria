// F-49 门卡引导与留档文案的单一事实源（controller 草案，常量集中便于按复验调整）。
// 门卡此前只有 4 字 trigger chip：无「为什么需要你」、无「建议确认还是反馈」，
// findings 只在「requiresTriage 且 0 条」时被用于一句提示——实测门卡 metadata 带
// 3 条 advisory 却完全不渲染（诊断 f49-gate-ui-diagnosis.md §2.2）。

/** B1：advisory-only 轮次的「为什么需要你」首行（N = 本轮 findings 条数）。 */
export function gateWhyAdvisoryCopy(findingCount: number): string {
  return `复评仍有 ${findingCount} 条 findings（均为建议级，不阻断发布）；引擎不做自动取舍，由你确认采纳或反馈修改。`;
}

/** B1：存在必须处理项时的首行（N = must_fix/blocking 条数）。 */
export function gateWhyRequiredCopy(requiredCount: number): string {
  return `存在 ${requiredCount} 条必须处理项，建议先提交反馈`;
}

/** B2：advisory-only 且可确认时的建议行。 */
export const GATE_ADVISORY_CONFIRM_HINT =
  "机械校验 0 error——可直接确认；如需采纳建议请提交反馈";

/** B3：门卡内 findings 折叠列表的开关文案（N = 本轮 findings 条数）。 */
export function gateFindingsToggleLabel(findingCount: number): string {
  return `本轮 findings ${findingCount} 条`;
}

/** A4：反馈提交被门面接受后的成功反馈。 */
export const GATE_FEEDBACK_SUBMITTED_NOTE = "反馈已提交";

/** B5：留档卡徽章。 */
export const GATE_ARCHIVE_BADGE_LABEL = "已留档";

/** B5：留档摘要截断长度（草案「摘要前 50 字」）。 */
export const GATE_ARCHIVE_FEEDBACK_SNIPPET_MAX = 50;

/** B5：旧轮留档文案（提交过反馈的轮次）。 */
export function gateArchiveFeedbackCopy(round: number, feedback: string): string {
  return `第 ${round} 轮已提交反馈：${feedback.trim().slice(0, GATE_ARCHIVE_FEEDBACK_SNIPPET_MAX)}`;
}

/**
 * B5：旧轮留档文案（未记录到反馈文本的轮次，如另一连接提交——本连接没有本地
 * 提交记录）。不谎报「已提交反馈」。
 */
export function gateArchiveClosedCopy(round: number): string {
  return `第 ${round} 轮已收口`;
}
