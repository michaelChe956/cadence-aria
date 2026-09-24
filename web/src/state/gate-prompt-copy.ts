// F-49 门卡引导与留档文案的单一事实源（controller 草案，常量集中便于按复验调整）。
// 门卡此前只有 4 字 trigger chip：无「为什么需要你」、无「建议确认还是反馈」，
// findings 只在「requiresTriage 且 0 条」时被用于一句提示——实测门卡 metadata 带
// 3 条 advisory 却完全不渲染（诊断 f49-gate-ui-diagnosis.md §2.2）。

/**
 * F-50 裁决 2：门卡唯一原因行（advisory-only，N = 本轮建议级 findings 数）。
 * 「机械校验 0 error」并入前半句作证据，下一步建议（可直接确认，或反馈后
 * 再修订）收在同一行——不再渲染独立的绿色建议行。
 */
export function gateWhyAdvisoryCopy(findingCount: number): string {
  return `原因：机械校验 0 error，复评有 ${findingCount} 条建议（不阻断发布）；可直接确认，或提交反馈后再修订。`;
}

/** F-50 裁决 2：存在必须处理项时的原因行（N = must_fix/blocking 条数）。 */
export function gateWhyRequiredCopy(requiredCount: number): string {
  return `原因：有 ${requiredCount} 条必须处理项；建议先提交反馈。`;
}

/**
 * F-50 裁决 1：triage intent 门的原因行——标题保留「需要判断 reviewer 意图」
 * 时，原因行解释为什么无法自动取舍并给出两条出路。
 */
export const GATE_TRIAGE_WHY_COPY =
  "原因：评审结果无法自动取舍；请选择确认当前版本或反馈修改。";

/**
 * F-50 裁决 3：反馈与确认并行——可直接确认，反馈是可选修订路径。帮助文案
 * 明示「反馈可选、确认无需填写」，消除「必须先反馈」的误读。
 */
export const GATE_FEEDBACK_OPTIONAL_HINT =
  "如需调整，请在反馈框填写修改意见；确认当前版本无需填写。";

/**
 * F-50 裁决 4：终止按钮按实际作用域命名——actions.terminate 发送
 * abandon_human_gate（门级 abandon，见 cockpit-action-routing.ts），故所有
 * 终止入口统一「终止此门」，与错误条/会话级动作不再同名歧义。
 */
export const GATE_TERMINATE_BUTTON_LABEL = "终止此门";
export const GATE_TERMINATE_CONFIRM_LABEL = "确认终止此门";

/** 与门卡标题同义的内容/摘要短语（归一化比较，去空白与标点）。 */
const GATE_TITLE_SYNONYMS: Record<string, true> = {
  需要人工确认: true,
  需要判断reviewer意图: true,
  人工确认: true,
  等待人工确认: true,
  需人工确认: true,
  等待确认: true,
  可确认当前版本: true,
};

/**
 * F-50 裁决 1：entry.content / summary 与标题同义（「等待人工确认」等人工
 * 介入同义句）时不渲染——单标题制下不让正文重复标题。独立事实不受影响。
 */
export function isGateTitleSynonymousCopy(text: string): boolean {
  const normalized = text
    .trim()
    .replace(/[\s，。；、·（）()：:—\-]/gu, "");
  return GATE_TITLE_SYNONYMS[normalized] === true;
}

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
