import {
  enrollmentTargetMemberCountText,
  enrollmentTargetPlaceholder,
} from "./enrollment-target-labels";
import { STALE_DRIVER_LEASE_CODE } from "./protocol-error-copy";
import type {
  C1WaitingItem,
  CodingAttentionItemDto,
  CodingFinalConfirmInfoItem,
  LogicalCodebaseBootstrapNoticeDto,
  LogicalCodebaseBootstrapProjection,
  PlanConfirmedInfoItem,
} from "../api/types";
import {
  GATE_TRIGGER_LABELS,
  type GateProjection,
} from "./workspace-cockpit-gate-projection";
import type {
  CockpitInboxItem,
  CodingAttentionInboxProjection,
} from "./workspace-cockpit-inbox-types";

// 从 workspace-cockpit-projection.ts 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 收件箱条目工厂（C1 等待项 / LC 冷启动 / plan 与 coding 信息 / 待处理事项 / 身份与去重）。
/** C1/C2 等待项标题（按 kind 固定文案；身份字段进摘要行）。 */
const C1_WAITING_KIND_TITLES: Record<string, string> = {
  candidate_recovery: "候选门快照待恢复",
  lease_wait: "租约活跃，自动链等待中",
  lease_takeover: "死亡租约待确认接管",
  lease_unknown: "租约活性未知，停等人工",
  advance_retry_failed: "Failed advance 待显式重试",
  intent_blocked: "计划意图停等修订",
  generation_history: "存在旧代绑定（只读可查）",
  // C5 Task 6：project 级 repository 初始化失败等待项（网关恢复后可续）。
  repository_initialization_failed: "仓库初始化失败，等待恢复",
  // C2 Task 12：coding 链十类等待项。
  coding_completion_unconfirmed: "Coding 完成状态待确认",
  coding_already_running: "Coding 已在运行，请等待",
  coding_takeover_required: "Coding 死亡租约待确认接管",
  coding_lease_unknown: "Coding 租约活性未知，停等人工",
  coding_restart_available: "Coding 终态可显式重启",
  reviewer_configuration_missing: "Reviewer 配置缺失，停等补齐",
  verification_triage: "验证处理待人工裁决",
  policy_verification: "政策核验停等（fail-closed）",
  instruction_claim_interrupted: "返修指令消费中断，待同认领重放",
  large_candidate_blocked: "大候选超预算，停等分段返修",
};

export function c1TargetLabel(target: C1WaitingItem["target"]): string {
  if (!target) {
    return "target 未知";
  }
  switch (target.kind) {
    case "single_repository":
      return `单仓 ${enrollmentTargetPlaceholder(target.repository_id)}`;
    case "logical_codebase":
      return `逻辑代码库 ${enrollmentTargetPlaceholder(target.logical_codebase_id)}/${enrollmentTargetPlaceholder(target.logical_repository_id)}`;
    case "logical_codebase_members":
      return `逻辑代码库 ${enrollmentTargetPlaceholder(target.logical_codebase_id)}[${enrollmentTargetMemberCountText(target.logical_repository_ids)}]`;
    default:
      return "target 未知";
  }
}

/** F-A additive（owner 2026-10-25 裁定，cadence/notes/2026-10-25_UI复验发现）：
 * 等待项可操作性分类的本地 wire 扩展。false = 自动等待/只读信息（租约活跃
 * 到期自动恢复、当前绑定健在时的换代历史）——投 kind="info" 归「进度信息」
 * 分组，不进「需要人工处理」与待处理计数；true = 需要人工决策。wire 主镜像
 * 在 api/types/lifecycle.ts（不在本改动文件集内），按既有 additive 缺省约定
 * 在此本地扩展；旧响应缺失该字段按 true 解释（保持旧展示）。 */
export type ClassifiedC1WaitingItem = C1WaitingItem & { requires_human?: boolean };

/** durable C1WaitingItem → 收件箱条目（不进 countedInbox；动作走 C1 REST）。
 * C5 Task 6：project 级条目不传 issueId——id 直接采用后端稳定 id
 *（c1:project:{project_id}:repository_init:{operation_id}），不拼接
 * undefined issueId；issue 级条目维持 `c1:{issueId}:{item.id}` 前缀。
 * F-A：requires_human=false 的条目投 kind="info"——两个待处理抽屉
 *（CockpitInbox / WorkbenchInboxOverlay）按 kind 分组，info 落「进度信息」
 * 且不渲染 C1 恢复卡（无动作按钮）；需要人工的维持 kind="c1_recovery"。 */
export function c1WaitingItem(
  item: C1WaitingItem,
  projectId: string,
  issueId?: string,
): CockpitInboxItem {
  const identityParts = [
    item.plan_id ? `plan ${item.plan_id}` : null,
    item.session_id ? `session ${item.session_id}` : null,
    item.attempt_id ? `attempt ${item.attempt_id}` : null,
    item.gate_id ? `gate ${item.gate_id}` : null,
    c1TargetLabel(item.target),
  ].filter((part): part is string => Boolean(part));
  const summaryParts = [
    item.reason,
    identityParts.length > 0 ? identityParts.join(" · ") : null,
    item.possible_side_effect ? `可能副作用：${item.possible_side_effect}` : null,
    item.next_phase ? `下一阶段：${item.next_phase}` : null,
  ].filter((part): part is string => Boolean(part));
  const requiresHuman = (item as ClassifiedC1WaitingItem).requires_human ?? true;
  return {
    id: issueId !== undefined ? `c1:${issueId}:${item.id}` : item.id,
    kind: requiresHuman ? "c1_recovery" : "info",
    severity: item.actions.length > 0 ? 2 : 3,
    title: C1_WAITING_KIND_TITLES[item.kind] ?? "C1 恢复等待项",
    summary: summaryParts.join(" · "),
    triage: false,
    source: "c1_waiting",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    c1Info: {
      projectId,
      issueId: issueId ?? null,
      itemId: item.id,
      kind: item.kind,
      reason: item.reason,
      completedSteps: item.completed_steps,
      targetLabel: c1TargetLabel(item.target),
      planId: item.plan_id ?? null,
      sessionId: item.session_id ?? null,
      attemptId: item.attempt_id ?? null,
      gateId: item.gate_id ?? null,
      possibleSideEffect: item.possible_side_effect ?? null,
      actions: item.actions,
      nextPhase: item.next_phase ?? null,
      // C2 Task 12 additive：expected 版本与操作上下文（旧响应缺失按空）。
      expectedVersion: item.expected_version ?? null,
      actionContext: (item.action_context ?? []).map((action) => ({
        action: action.action,
        commandId: action.command_id,
        expectedVersion: action.expected_version,
      })),
      // C5 Task 6 additive：project 级条目操作身份（后端稳定 operation_id）。
      operationId: item.operation_id ?? null,
      diagnostics: item.diagnostics
        ? {
            failedStep: item.diagnostics.failed_step,
            reasonCode: item.diagnostics.reason_code,
            provider: item.diagnostics.provider ?? null,
            stderrSummary: item.diagnostics.stderr_summary ?? null,
            changedPaths: item.diagnostics.changed_paths ?? [],
            retryable: item.diagnostics.retryable ?? false,
          }
        : null,
    },
  };
}

/** C4 Task 9 步骤名的固定中文文案（与服务端 as_str 一一对应）。 */
const LC_BOOTSTRAP_STEP_LABELS: Record<string, string> = {
  identity: "身份",
  manifest_checkout: "清单/检出",
  rules_policy: "规则/政策",
  member_index: "成员索引",
  aggregate_index_active: "聚合索引激活",
};

/** C4 Task 9 步骤失败/等待原因的固定标题（未知 reason 回落到通用文案）。 */
const LC_BOOTSTRAP_REASON_TITLES: Record<string, string> = {
  identity_migration_failed: "逻辑代码库身份迁移失败，等待人工修复",
  aggregate_initialization_failed: "成员索引构建失败，等待显式重试",
  aggregate_index_failed: "聚合索引首建失败，等待显式重试",
  // codegraph 语义化（owner 直批）：空仓/成员无可索引代码文件时给出
  // 业务标题，不再让 JSON 解析天书上卡。
  aggregate_index_member_coverage_failed: "聚合索引失败:成员代码覆盖不足",
  bootstrap_waiting_for_human: "逻辑代码库冷启动等待人工动作",
  member_rules_missing: "成员规则材料缺失，provider 保持零启动",
};

/** 覆盖不足的成员清单提取——后端 detail 契约：
 * `... no indexable code file under included members [api, web]; ...`。 */
const MEMBER_COVERAGE_LIST_PATTERN = /members \[([^\]]*)\]/;
const LC_BOOTSTRAP_REASON_HINTS: Partial<
  Record<string, (detail: string) => string>
> = {
  aggregate_index_member_coverage_failed: (detail) => {
    const members = MEMBER_COVERAGE_LIST_PATTERN.exec(detail)?.[1];
    return members
      ? `无可索引代码文件的成员：${members} · 每成员至少 1 个可索引代码文件`
      : "每成员至少 1 个可索引代码文件";
  },
};

/** C4 Task 9：durable bootstrap notice → 只读收件箱条目（不进错误计数；
 * id 以稳定 notice key 派生，同 key 只保留一条，重放不重复计数）。 */
export function logicalCodebaseBootstrapItem(
  notice: LogicalCodebaseBootstrapNoticeDto,
  projection: Pick<
    LogicalCodebaseBootstrapProjection,
    "project_id" | "logical_codebase_id" | "membership_revision"
  >,
): CockpitInboxItem {
  const stepLabel = LC_BOOTSTRAP_STEP_LABELS[notice.step] ?? notice.step;
  const reasonHint = LC_BOOTSTRAP_REASON_HINTS[notice.reason_code]?.(notice.summary);
  const summaryParts = [
    reasonHint,
    notice.summary,
    `步骤：${stepLabel} · 对象 ${notice.object_id}`,
    notice.external_side_effect && notice.external_side_effect !== "none"
      ? `可能外部副作用：${notice.external_side_effect}`
      : null,
    notice.next_step
      ? `下一步：${LC_BOOTSTRAP_STEP_LABELS[notice.next_step] ?? notice.next_step}`
      : null,
  ].filter((part): part is string => Boolean(part));
  return {
    id: `lc-bootstrap:${projection.project_id}:${projection.logical_codebase_id}:${notice.key}`,
    kind: "lc_bootstrap",
    severity: 2,
    title: LC_BOOTSTRAP_REASON_TITLES[notice.reason_code]
      ?? `逻辑代码库冷启动等待：${notice.reason_code}`,
    summary: summaryParts.join(" · "),
    triage: false,
    source: "logical_codebase_bootstrap",
    createdAt: notice.created_at || null,
    gate: null,
    inlineError: null,
    choice: null,
    bootstrapInfo: {
      projectId: projection.project_id,
      logicalCodebaseId: projection.logical_codebase_id,
      noticeKey: notice.key,
      step: notice.step,
      objectId: notice.object_id,
      reasonCode: notice.reason_code,
      summary: notice.summary,
      externalSideEffect: notice.external_side_effect,
      allowedActions: notice.allowed_actions,
      nextStep: notice.next_step ?? null,
      membershipRevision: projection.membership_revision ?? null,
    },
  };
}

/** 裸 driver 抢走租约后的协议码——常量源头在 protocol-error-copy（F-50）。 */
export { STALE_DRIVER_LEASE_CODE } from "./protocol-error-copy";

export function isStaleDriverLeaseItem(item: CockpitInboxItem): boolean {
  return item.source === "protocol_error" && item.protocolErrorCode === STALE_DRIVER_LEASE_CODE;
}

/** REQ-WIGA-07：durable plan_confirmed_info → 只读 info 收件箱条目。 */
export function planConfirmedInfoItem(info: PlanConfirmedInfoItem): CockpitInboxItem {
  return {
    id: `${info.session_id}:info:${info.key}`,
    kind: "info",
    severity: 1,
    title: info.title,
    summary: `plan ${info.plan_id} 已确认 · 确认于 ${info.occurred_at}`,
    triage: false,
    source: "plan_confirmed_info",
    createdAt: info.occurred_at,
    gate: null,
    inlineError: null,
    choice: null,
    planInfo: {
      key: info.key,
      planId: info.plan_id,
      sessionId: info.session_id,
      occurredAt: info.occurred_at,
      title: info.title,
    },
  };
}

/** REQ-WIGA-07/R5：durable coding_final_confirm_info → 只读 info 收件箱条目。 */
export function codingFinalConfirmInfoItem(
  info: CodingFinalConfirmInfoItem,
): CockpitInboxItem {
  return {
    id: `${info.issue_id}:info:${info.key}`,
    kind: "info",
    severity: 1,
    title: info.title,
    summary: info.final_confirmed
      ? `编码执行已完成人工最终确认 · ${info.occurred_at}`
      : `等待人工最终确认 · ${info.occurred_at}`,
    triage: false,
    source: "coding_final_confirm_info",
    createdAt: info.occurred_at,
    gate: null,
    inlineError: null,
    choice: null,
    codingInfo: {
      projectId: info.project_id,
      issueId: info.issue_id,
      planId: info.plan_id,
      attemptId: info.attempt_id,
      key: info.key,
      occurredAt: info.occurred_at,
      finalConfirmed: info.final_confirmed,
    },
  };
}

/** 稳定身份：JSON.stringify([project_id, issue_id, key])——数组序列化
 * 避免跨 project/issue/key 分隔符冲突（与 recentCompletionIdentity 同构）。 */
export function codingAttentionIdentity(item: CodingAttentionItemDto): string {
  return JSON.stringify([item.project_id, item.issue_id, item.key]);
}

/**
 * UI-3（待处理统一）：coding choice 的**规范身份**——完整作用域
 * （project/issue/attempt）+ 来源类型 + choiceId 归一（JSON 数组序列化，
 * 与 codingAttentionIdentity 同构）。用于跨来源合并去重（lifecycle
 * coding_attention 目录行 vs 页面 snapshot 派生的就地作答卡）；通知身份
 * （codingAttentionIdentity）只管提醒去重，两者不混。
 */
export function canonicalCodingChoiceIdentity(input: {
  projectId: string;
  issueId: string;
  attemptId: string;
  choiceId: string;
}): string {
  return JSON.stringify([
    "coding_choice",
    input.projectId,
    input.issueId,
    input.attemptId,
    input.choiceId,
  ]);
}

/** 收件箱条目的 coding choice 规范身份；非 coding choice 条目返回 null。 */
export function inboxCodingChoiceIdentity(
  item: CockpitInboxItem,
): string | null {
  if (
    item.kind === "choice" &&
    item.choice !== null &&
    item.choice.source === "coding" &&
    item.choice.attemptAddress !== undefined &&
    item.choice.choiceId !== ""
  ) {
    return canonicalCodingChoiceIdentity({
      ...item.choice.attemptAddress,
      choiceId: item.choice.choiceId,
    });
  }
  if (
    item.kind === "coding_action" &&
    item.attentionInfo !== undefined &&
    item.attentionInfo !== null &&
    item.attentionInfo.attentionKind === "choice" &&
    item.attentionInfo.attemptId !== null &&
    item.attentionInfo.focus?.choice_id
  ) {
    return canonicalCodingChoiceIdentity({
      projectId: item.attentionInfo.projectId,
      issueId: item.attentionInfo.issueId,
      attemptId: item.attentionInfo.attemptId,
      choiceId: item.attentionInfo.focus.choice_id,
    });
  }
  return null;
}

/**
 * UI-3：同一 choice 双来源只显示一次——coding 就地作答卡（含逐题表单，
 * 可直接操作）优先，同规范身份的 coding_action 目录行丢弃（目录行只是
 * 指针，不是第二张卡）。其余条目原样保留。
 */
export function dedupeCodingChoiceInboxItems(
  items: readonly CockpitInboxItem[],
): CockpitInboxItem[] {
  const choiceCardIdentities = new Set<string>();
  for (const item of items) {
    if (item.kind === "choice") {
      const identity = inboxCodingChoiceIdentity(item);
      if (identity !== null) {
        choiceCardIdentities.add(identity);
      }
    }
  }
  if (choiceCardIdentities.size === 0) {
    return [...items];
  }
  return items.filter((item) => {
    // 只丢弃与作答卡同身份的 coding_action 目录行；choice 卡本身保留。
    if (item.kind !== "coding_action") {
      return true;
    }
    const identity = inboxCodingChoiceIdentity(item);
    return identity === null || !choiceCardIdentities.has(identity);
  });
}

/** P4-d：durable 待处理事项 → 收件箱条目（进 display/count，无 watched/
 * TTL/上限过滤）。kind 全量透传（choice/triage/blocked/stalled/
 * manual_recovery/final_confirm/plan_final_confirm 及未知未来 kind 均
 * 投 kind="coding_action"，不枚举过滤）；标题/摘要/时间取服务端 durable
 * 事实，前端不另造文案。id 前缀 `coding_attention:` 不携带 workspace
 * session（cockpitInboxItemSessionId 对其 fail-closed 返回 null，
 * 批量/就地动作不误路由）。 */
export function codingAttentionItem(item: CodingAttentionItemDto): CockpitInboxItem {
  const summaryParts = [
    item.summary,
    item.repository_name,
    item.logical_work_item_id,
  ].filter((part): part is string => Boolean(part));
  return {
    id: `coding_attention:${item.project_id}:${item.issue_id}:${item.key}`,
    kind: "coding_action",
    severity: 2,
    title: item.title,
    summary: summaryParts.join(" · "),
    triage: false,
    source: "coding_attention",
    createdAt: item.occurred_at,
    gate: null,
    inlineError: null,
    choice: null,
    attentionInfo: {
      identity: codingAttentionIdentity(item),
      attentionKind: item.kind,
      projectId: item.project_id,
      issueId: item.issue_id,
      planId: item.plan_id ?? null,
      attemptId: item.attempt_id ?? null,
      logicalWorkItemId: item.logical_work_item_id ?? null,
      repositoryName: item.repository_name ?? null,
      reasonCode: item.reason_code ?? null,
      occurredAt: item.occurred_at,
      target: item.target,
      focus: item.focus ?? null,
    },
  };
}

/** 收件箱条目 id（`${sessionId}:gate:${key}` 等形态）的会话归属；无会话前缀返回 null。 */
export function cockpitInboxItemSessionId(itemId: string): string | null {
  const separator = itemId.indexOf(":");
  if (separator <= 0) {
    return null;
  }
  const sessionId = itemId.slice(0, separator);
  return sessionId === "gate" ||
      sessionId === "hard_error" ||
      sessionId === "stopped" ||
      sessionId === "coding_attention"
    ? null
    : sessionId;
}

/**
 * REQ-PCG-01/02：门条标题按门种类给出动作语义；两新门不复用 legacy 逐段决策文案。
 */
export function gateInboxTitle(gate: GateProjection): string {
  if (gate.kind === "batch_confirm") {
    return "确认整组 Work Item Draft";
  }
  if (gate.kind === "compile_recovery") {
    return "Final Compile 恢复";
  }
  // F-50 fix round 标题去重：triage 短语只保留主区门卡卡头一处——抽屉门条目
  // 与非 triage 门同题；triage 细节由主区门卡头+原因行承载。
  return "需要人工确认";
}

export function gateInboxHeadline(gate: GateProjection): string | null {
  if (gate.kind === "batch_confirm") {
    return "等待整组 Work Item Draft 确认";
  }
  if (gate.kind === "compile_recovery") {
    return "Final Compile 中断，等待恢复动作";
  }
  // F-50 §3.5：触发原因独立成「原因：…」行（与门卡 gate-why 同构），
  // 不再用标题重复人工介入语义；无 trigger 即无独立事实，返回 null 让位
  // （parts 已 filter），不以「等待人工确认」与标题「需要人工确认」近义叠行
  // （fix1，k3 P2——story/design author_confirm 主流程 trigger 恒 null）。
  return gate.trigger ? `原因：${GATE_TRIGGER_LABELS[gate.trigger]}` : null;
}
