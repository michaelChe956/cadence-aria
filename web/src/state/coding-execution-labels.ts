import type {
  CodingAttemptStatus,
  CodingExecutionStage,
  PushStatus,
} from "../api/types";

/**
 * FE-1（设计 Q5 冻结）：coding attempt 状态 / 执行 stage / 推送状态三组展示标签
 * 的单一定义点。原先是 PlanGroupProjectionPanel 的私有常量；FE-2 的逐仓 PR 面板
 * 与 issue 交付面板复用同一份文案，避免出现第二套漂移的标签表。
 *
 * 完整性由左侧 `Record<值域, string>` 注解在编译期保证：值域新增成员而标签缺失
 * 会直接编译失败。对外导出放宽为 `Record<string, string>`，因为 DTO 侧字段是
 * `string | null`（服务端只承诺 snake_case 字符串事实），调用方以 `?? 原值` 回落。
 */

/** skipped_not_configured 的统一文案：本地交付，与「已推送」必须可区分（设计 Q1）。 */
export const LOCAL_DELIVERY_LABEL = "本地交付(无远端)";

const ATTEMPT_STATUS_LABELS_BY_VALUE: Record<CodingAttemptStatus, string> = {
  created: "已创建",
  running: "执行中",
  waiting_for_human: "等待人工",
  blocked: "已阻塞",
  awaiting_manual_recovery: "等待人工恢复",
  awaiting_plan_amendment: "等待计划修订",
  applying_plan_amendment: "应用计划修订",
  amendment_apply_failed: "修订应用失败",
  completed: "已完成",
  failed: "已失败",
  aborted: "已中止",
};

const STAGE_LABELS_BY_VALUE: Record<CodingExecutionStage, string> = {
  prepare_context: "准备上下文",
  worktree_prepare: "准备 Worktree",
  coding: "编码",
  code_review: "代码评审",
  review_request: "评审请求",
  internal_pr_review: "内部 PR 评审",
  final_confirm: "最终确认",
};

const PUSH_STATUS_LABELS_BY_VALUE: Record<PushStatus, string> = {
  pushed: "已推送",
  not_pushed: "未推送",
  failed: "推送失败",
  skipped_not_configured: LOCAL_DELIVERY_LABEL,
};

export const ATTEMPT_STATUS_LABELS: Record<string, string> = {
  ...ATTEMPT_STATUS_LABELS_BY_VALUE,
};

export const STAGE_LABELS: Record<string, string> = {
  ...STAGE_LABELS_BY_VALUE,
};

export const PUSH_STATUS_LABELS: Record<string, string> = {
  ...PUSH_STATUS_LABELS_BY_VALUE,
};
