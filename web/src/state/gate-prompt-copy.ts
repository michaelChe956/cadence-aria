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

/**
 * B4：反馈已提交、引擎正在跑门内修订 provider。
 * 草案原为「（第 N/3 轮）」，但前端无单调轮次事实源——门每次重开预算按默认值重置
 * （REQ-CG-02 重建公式 ⇒ `remaining_budget` 非单调），turn 计数不在 session_state 内，
 * 补序号属新增可见事实面（契约增量），故先不绑序号。
 */
export const GATE_REVISION_RUNNING_NOTE = "已提交，正在按反馈修订";

/** B4：门内修订已完成，引擎正在复评（复评结论随后以 review 结论卡呈现）。 */
export const GATE_REVISION_REVIEWING_NOTE = "修订完成，正在复评";

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

/** B6：「采纳建议为反馈」按钮文案。 */
export const GATE_ADOPT_FINDINGS_BUTTON_LABEL = "采纳建议为反馈";

/** B6：采纳文本开头一句（模板见 gateAdoptFindingsFeedback）。 */
export const GATE_ADOPT_FINDINGS_LEAD = "按复评建议修订以下内容：";

/** B6：采纳文本收尾一句（模板见 gateAdoptFindingsFeedback）。 */
export const GATE_ADOPT_FINDINGS_TRAIL = "其余内容保持不变。";

/**
 * B6：单条 advisory finding 的采纳句。对象=message、建议动作=required_action
 * （门卡 metadata findings 的结构化文本位就这两个，与 B3 渲染同源）；建议缺失即
 * 字段不足，整条原文降级拼入。
 */
export function gateAdoptFindingLine(finding: {
  message: string;
  required_action?: string;
}): string {
  const subject = finding.message.trim();
  const action = finding.required_action?.trim() ?? "";
  if (subject && action) {
    return `${subject}：${action}；`;
  }
  return `${subject || action}；`;
}

/**
 * B6：advisory findings → 反馈输入框的修订指令文本（只填入不提交，用户可继续
 * 编辑）。must_fix 不在拼接范围（处理路径不同），由调用方过滤后传入。
 *
 * 单行串接：门卡反馈框是单行 <input>（HTML value 净化会剥掉换行），开头/收尾
 * 两句按前后缀直接串接，保证所见即所交。
 */
export function gateAdoptFindingsFeedback(
  findings: ReadonlyArray<{ message: string; required_action?: string }>,
): string {
  const items = findings
    .map(gateAdoptFindingLine)
    .filter((line) => line !== "；");
  return `${GATE_ADOPT_FINDINGS_LEAD}${items.join("")}${GATE_ADOPT_FINDINGS_TRAIL}`;
}
