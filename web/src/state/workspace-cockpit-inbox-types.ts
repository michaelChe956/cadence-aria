import type {
  C1WaitingItem,
  ChoiceQuestion,
  CodingAttentionItemDto,
  LcFreshOperationDto,
  LcResumeRecoveryDto,
} from "../api/types";
import type { CodingAttemptAddress } from "../api/types/coding";
import type { GateProjection } from "./workspace-cockpit-gate-projection";

// 从 workspace-cockpit-projection.ts 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 收件箱条目/投影的类型面（无运行时代码）。
export type CockpitInboxKind =
  | "gate"
  | "stopped"
  | "hard_error"
  | "choice"
  | "info"
  | "c1_recovery"
  | "sc_failed"
  | "lc_bootstrap"
  | "lc_resume_recovery"
  | "coding_action";

/**
 * P0 1.3（REQ-WIGA-05）Task 11：驾驶舱 choice 就地作答投影——workspace 侧
 * 来自 session_state `pending_choice_requests`（含完整逐题结构），coding 侧
 * 来自 attempt snapshot 的 open choice gate（页面按需拉取）。status 是本地
 * 命令状态与服务端 pending 列表的合并视图：open=可作答；
 * submitting/resolving=202 已受理保卡复查；delivered=回执已送达（卡片移除）；
 * expired=410 旧 run 失效（不路由到新 run）。
 */
export interface ChoiceInboxProjection {
  sessionId: string | null;
  choiceId: string;
  prompt: string;
  expectedRunId: string | null;
  questions: ChoiceQuestion[];
  allowMultiple: boolean;
  allowFreeText: boolean;
  source: "workspace" | "coding";
  attemptAddress?: CodingAttemptAddress;
  status: "open" | "submitting" | "resolving" | "delivered" | "expired";
}

/**
 * P1 WIGA Task 9（REQ-WIGA-07）：durable plan 确认 info 的只读投影——不进
 * countedInbox/批量/危险操作，仅供「进度信息」分区显示与一次提醒。
 */
export interface PlanConfirmedInfoProjection {
  key: string;
  planId: string;
  sessionId: string;
  occurredAt: string;
  title: string;
}

/**
 * P2 WIGA Task 9（REQ-WIGA-07/R5）：durable coding FinalConfirm 等待/已确认
 * 信息的只读投影——不进 countedInbox/批量/危险操作，仅供「进度信息」分区
 * 显示、一次提醒与 Coding Workspace 下钻。
 */
export interface CodingFinalConfirmInfoProjection {
  projectId: string;
  issueId: string;
  planId: string;
  attemptId: string;
  key: string;
  occurredAt: string;
  finalConfirmed: boolean;
}

export interface CockpitInboxItem {
  id: string;
  kind: CockpitInboxKind;
  severity: 1 | 2 | 3;
  title: string;
  summary: string;
  triage: boolean;
  source:
    | "gate"
    | "session_status"
    | "protocol_error"
    | "engine_error"
    | "advance"
    | "choice"
    | "plan_confirmed_info"
    | "coding_final_confirm_info"
    | "c1_waiting"
    | "sc_failed"
    | "logical_codebase_bootstrap"
    | "lc_resume_recovery"
    | "coding_attention",
  createdAt: string | null;
  gate: GateProjection | null;
  inlineError: { code: string; message: string } | null;
  /** REQ-WIGA-05 Task 11：kind="choice" 时的作答投影；其余 kind 恒 null。 */
  choice: ChoiceInboxProjection | null;
  /** protocol_error 来源条目的机器码（如 STALE_DRIVER_LEASE）；其余来源缺省。 */
  protocolErrorCode?: string | null;
  /** REQ-WIGA-07 Task 9：kind="info" 时的 plan 确认投影；其余 kind 缺省。 */
  planInfo?: PlanConfirmedInfoProjection | null;
  /** REQ-WIGA-07/R5 Task 9：kind="info" 时的 coding FinalConfirm 投影；其余 kind 缺省。 */
  codingInfo?: CodingFinalConfirmInfoProjection | null;
  /** P3（REQ-WIGA-07）：近期完成事实的稳定身份（JSON [scope,kind,key]）；
   * 仅 recent 目录投影填写，提示去重以此为准（不取易碰撞的 item.id）。 */
  completionIdentity?: string;
  /** P2 GAP-E/G（Task 0.1）：kind="sc_failed" 时的人工显式重驱投影。 */
  scFailure?: { failedNodeId: string; phase: "failed" } | null;
  /** C1（enrollment-recovery-surface Task 9）：kind="c1_recovery" 时的
   * durable 恢复等待项投影（reason/身份/可用动作/下一阶段）。 */
  c1Info?: C1WaitingProjection | null;
  /** C4（LC 冷启动加固 Task 9）：kind="lc_bootstrap" 时的 durable 冷启动
   * 等待/失败通知投影（notice key/step/object/原因/副作用/动作/下一步）；
   * 不进入错误计数，动作统一走 bootstrap action API。 */
  bootstrapInfo?: LogicalCodebaseBootstrapInfoProjection | null;
  /** C1（Task 6.1）：kind="lc_resume_recovery" 时的 durable 恢复卡投影；
   * 仅 waiting/fresh_reserved/fresh_running 投卡，resolved 证据留 store
   * 不投卡（动作资格只镜像后端 allowed_actions，前端不推导）。 */
  recovery?: LcResumeRecoveryInboxProjection | null;
  /** P4-d（REQ-UI37-19）：kind="coding_action" 时的 durable 待处理事项
   * 投影（7 类 kind + 未知 kind 全量；目标/定位供「去处理」导航，动作不
   * 在收件箱/通知上执行）。 */
  attentionInfo?: CodingAttentionInboxProjection | null;
  /** trust 缺失错误条的可登记投影（按稳定码识别 + provider wire 标签
   * 同源解析；owner 2026-10-12 直批「登记并重试」按钮）；其余条目缺省。 */
  trustMissing?: TrustMissingProjection | null;
  /** trustMissing 条目对应 provider 已显式登记成功（typed 回执对账）。 */
  trustRegistered?: boolean;
  /** 批 2 P5（REQ-WIGA-08 C1 前端半）：kind="stopped" 时的托管管理态——
   * durable automation 归属 owner=server 且 enabled（与 P4 服务端 takeover
   * 拒绝同源判据）；true 时收件箱不渲染接管按钮（隐藏仅同步呈现，服务端
   * 拒绝仍是权威）。其余 kind 缺省。 */
  automationManaged?: boolean;
}

/** trust 缺失指引卡投影（owner 2026-10-12 直批简化）：结构化指导
 *（providers/canonicalRoot/commandMarkdown）优先——wire
 * trust_missing_guidance 帧镜像，前端不解析 error 文本；旧服务端无
 * 结构化帧时回落稳定码文本识别（仅 provider 字段）。 */
export interface TrustMissingProjection {
  /** 首个 provider 的 wire 标签（「登记并重试」按钮出站参数）。 */
  provider: string;
  /** 缺 trust 的 provider wire 标签全集（卡面按实际单复数）；文本回落
   * 时缺省（视为仅 provider 单元素）。 */
  providers?: readonly string[];
  /** 冻结聚合根（第一行「该目录（…）」）；文本回落/未知时缺省。 */
  canonicalRoot?: string | null;
  /** 可复制登记命令（Kimi 单行 shell／Codex TOML 片段）；冲突等分型
   * 或文本回落时缺省（卡面不渲染命令窗）。 */
  commandMarkdown?: string | null;
  /** 场景化重试动作文案；文本回落时缺省。 */
  retryAction?: string | null;
}

/** C4 Task 9：LC 冷启动通知的收件箱投影（服务端
 * LogicalCodebaseBootstrapNotice 的只读镜像；字段一一对应，前端不另造
 * 事实，按钮生成稳定 command id 后经统一 action API 出站）。 */
export interface LogicalCodebaseBootstrapInfoProjection {
  projectId: string;
  logicalCodebaseId: string;
  noticeKey: string;
  step: string;
  objectId: string;
  reasonCode: string;
  summary: string;
  externalSideEffect: string;
  allowedActions: readonly string[];
  nextStep: string | null;
  membershipRevision: number | null;
}

/** C1（Task 6.1）：LC 恢复收件箱卡投影（wire LcResumeRecoveryDto 的只读
 * camelCase 镜像，供 D 层卡面渲染；动作资格只镜像 allowed_actions，
 * last_operation 为 typed 事实，不从错误文本解析）。 */
export interface LcResumeRecoveryInboxProjection {
  recoveryId: string;
  state: LcResumeRecoveryDto["state"];
  reasonCode: string;
  provider: string;
  failedNodeId: string;
  runKind: string;
  detectedAt: string;
  updatedAt: string;
  allowedActions: readonly string[];
  lastOperation: LcFreshOperationDto | null;
}

/** C2 Task 12：等待项操作上下文的只读镜像（服务端 WaitingItemAction）。 */
export interface WaitingItemActionProjection {
  action: string;
  commandId: string;
  expectedVersion: number;
}

/** C1 Task 9：durable 恢复等待项的收件箱投影（服务端 C1WaitingItem 的
 * 只读镜像；动作按 actions 名由 facade 触发对应 REST，前端不判定成功）。
 * C2 Task 12 additive：expectedVersion／actionContext（C2 kind 同通道）。
 * C5 Task 6 additive：project 级条目无 issue（issueId 为 null），身份用
 * 后端 operation_id（id 即后端稳定 id，不从展示字符串反解析）。 */
export interface C1WaitingProjection {
  projectId: string;
  issueId: string | null;
  itemId: string;
  kind: string;
  reason: string;
  completedSteps: readonly string[];
  targetLabel: string;
  planId: string | null;
  sessionId: string | null;
  attemptId: string | null;
  gateId: string | null;
  possibleSideEffect: string | null;
  actions: readonly string[];
  nextPhase: string | null;
  expectedVersion: number | null;
  actionContext: readonly WaitingItemActionProjection[];
  // C5 Task 6 additive：repository 初始化失败等待项的操作身份与诊断。
  operationId: string | null;
  diagnostics: RepositoryInitFailureDiagnosticsProjection | null;
}

/** C5 Task 6：repository 初始化失败诊断的只读镜像（结构化原因/步骤/路径）。 */
export interface RepositoryInitFailureDiagnosticsProjection {
  failedStep: string;
  reasonCode: string;
  provider: string | null;
  stderrSummary: string | null;
  changedPaths: readonly string[];
  retryable: boolean;
}

/** P4-d（REQ-CRO-12/契约 A1 · REQ-UI37-19）：durable 待处理事项的收件箱
 * 投影——服务端 `CodingAttentionItemDto` 的 camelCase 只读镜像。identity
 * 为完整 scope `[project_id, issue_id, key]`（JSON 数组序列化，跨
 * project/issue 的重名 key 不合并）；target/focus 只定位不写操作
 *（「去处理」导航由 Shell 消费）。 */
export interface CodingAttentionInboxProjection {
  identity: string;
  attentionKind: string;
  projectId: string;
  issueId: string;
  planId: string | null;
  attemptId: string | null;
  logicalWorkItemId: string | null;
  repositoryName: string | null;
  reasonCode: string | null;
  occurredAt: string;
  target: CodingAttentionItemDto["target"];
  focus: CodingAttentionItemDto["focus"] | null;
}
