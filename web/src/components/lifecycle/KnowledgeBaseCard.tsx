// Task 9：LC 仓管理抽屉内的「知识库」卡片。
// 与 AggregateInitializationCard 同构：卡片头 + 按钮区 + 步骤列表 + 失败文案；
// provider 选择摘要组件（含 SKIPPED_REASON_LABELS）从聚合卡提升至此共用，
// AggregateInitializationCard 保留 re-export 以维持既有导入路径稳定。
//
// Task 3（REQ-KB-13）：后端把「聚合初始化未完成」的前置门盖在全部知识库端点
// （含 status 端点）入口，失败 = 422 + 稳定码
// knowledge_base_requires_aggregate_initialization。卡片捕获该码 ⇒ 显示与
// 详情页逐字一致的中文提示，并把写入口（初始化/更新/创建输入模板）禁用。
import { useEffect, useRef, useState } from "react";
import {
  CircleCheck,
  CircleMinus,
  CircleX,
  LoaderCircle,
  TriangleAlert,
} from "lucide-react";
import { ApiRequestError } from "../../api/client";
import { getKnowledgeBaseStatus } from "../../api/knowledge-base";
import type { AggregateProviderSelectionDto } from "../../api/types";
import type {
  KnowledgeBaseChangePackageViolationDto,
  KnowledgeBaseOperationSnapshotDto,
  KnowledgeBaseOperationStepStatus,
  KnowledgeBaseStatusDto,
} from "../../api/types/knowledge-base";
import { KnowledgeBaseUpdateDialog } from "./KnowledgeBaseUpdateDialog";

/**
 * REQ-KB-13：后端「聚合初始化未完成」前置门的稳定失败码。
 * 后端在知识库全部端点入口统一判定（HTTP 层一处前置门），前端按此码
 * 渲染统一提示并禁用写入口——不臆造 status 投影里并不存在的门控字段。
 */
export const AGGREGATE_INITIALIZATION_REQUIRED_CODE =
  "knowledge_base_requires_aggregate_initialization";

/**
 * REQ-KB-13：卡片与详情页共用的同款中文提示（逐字一致，两处必须同一句）。
 * 口径取自技术方案 §3.1「需先完成聚合初始化（规则/政策）后再操作知识库」；
 * 详情页复用本常量导出，不另起一套文案。
 */
export const AGGREGATE_INITIALIZATION_REQUIRED_PROMPT =
  "需先完成聚合初始化（规则/政策）后再操作知识库";

/**
 * 知识库的**适用项目类型**（当前能力边界）。
 * 知识库目前只覆盖 Java Web / 纯 Java / 纯 Web 三类项目；其它技术栈的项目
 * 使用知识库会得到不适用的产物，所以卡片与详情页都必须把这条边界摆在
 * 写入口旁边（两处共用本常量 ⇒ 逐字一致，不各写一套）。
 */
export const KNOWLEDGE_BASE_SUPPORTED_PROJECT_HINT =
  "知识库当前仅支持 Java Web 项目、纯 Java 项目与纯 Web 项目；其他类型项目请勿使用。";

/**
 * 前置门未满足时的复检间隔（毫秒）：聚合初始化在同一抽屉内完成后，
 * 卡片据此恢复写入口（用户不必关开抽屉）。
 */
const GATE_RECHECK_INTERVAL_MS = 5_000;

/**
 * 判定一次失败是否为「聚合初始化未完成」的前置门拒绝。
 * 只认后端给出的稳定码（ApiRequestError.code）；其它失败码一律不误判。
 */
function isAggregateGateFailure(reason: unknown): boolean {
  return (
    reason instanceof ApiRequestError &&
    reason.code === AGGREGATE_INITIALIZATION_REQUIRED_CODE
  );
}

/** 跳过原因码 → 摘要行中文标签（与后端 skipped_reason 词表一一对应）。 */
export const SKIPPED_REASON_LABELS: Record<string, string> = {
  cli_unavailable: "未安装",
  needs_verification: "未核验",
  denied: "实测拒绝",
  evidence_stale: "证据过期",
};

/**
 * 冻结的 provider 选择摘要（REQ-BOOT-07）：「本次使用 X；跳过：Y（原因）、…」。
 * 候选顺序沿用后端 owner 顺序，不按结果重排；未知原因码如实原样展示。
 */
function providerSelectionSummary(selection: AggregateProviderSelectionDto): string {
  const selected =
    selection.candidates.find((c) => c.provider === selection.selected)?.display_name ??
    selection.selected;
  const skipped = selection.candidates
    .filter((c) => c.skipped_reason != null)
    .map((c) => {
      const reason = c.skipped_reason ?? "";
      return `${c.display_name}（${SKIPPED_REASON_LABELS[reason] ?? reason}）`;
    });
  return skipped.length === 0
    ? `本次使用 ${selected}`
    : `本次使用 ${selected}；跳过：${skipped.join("、")}`;
}

/**
 * 公共 provider 选择摘要组件（Task 9 从 AggregateInitializationCard 提升）：
 * 聚合初始化卡与知识库卡共用同一份文案口径；容器与 testid 由调用方决定。
 * 两个数据来源（已建 operation 的 provider_selection / 预检失败透出的选择结果）
 * 装配成同一个 AggregateProviderSelectionDto 后传入。
 */
export function ProviderSelectionSummary({
  selection,
}: {
  selection: AggregateProviderSelectionDto;
}) {
  return <>{providerSelectionSummary(selection)}</>;
}

/**
 * 预检失败是**请求期**产物（POST /initializations 被 4xx 拒，没有 operation
 * 承载），由宿主在 catch 里把 ApiError 装配成本形状挂在卡片上。
 * 注：Task 8 的 api/types/knowledge-base.ts 未提供该 DTO（api/ 目录只读），
 * 故由卡片自带 prop 级形状；字段口径与 Task 7 失败面
 * （details.missing / details.violations）对齐，不改动 Task 8 的 DTO。
 */
export type KnowledgeBasePreflightFailureDetails = {
  /** 预检判定缺失的输入文件相对路径（如 api-scope.md）。 */
  missing?: string[];
  /** 预检判定的逐条问题（与变更包门禁违规同构）。 */
  violations?: KnowledgeBaseChangePackageViolationDto[];
};

export type KnowledgeBasePreflightFailureDto = {
  code: string;
  message: string;
  details?: KnowledgeBasePreflightFailureDetails | null;
};

/** 预检失败稳定码 → 中文口径（Task 7 失败码；未知码回退展示 message）。 */
const PREFLIGHT_FAILURE_LABELS: Record<string, string> = {
  // 前置门未满足 ⇒ 与详情页同一句（REQ-KB-13：两处必须逐字一致）。
  knowledge_base_requires_aggregate_initialization:
    AGGREGATE_INITIALIZATION_REQUIRED_PROMPT,
  knowledge_base_skills_unavailable: "知识库预检未通过：Cadence skills 当前不可用。",
  knowledge_base_inputs_incomplete: "知识库预检未通过：输入模板不完整。",
  knowledge_base_manifest_missing: "知识库预检未通过：manifest 缺失。",
  knowledge_base_manifest_version_unsupported:
    "知识库预检未通过：manifest 版本不受支持。",
  knowledge_base_not_initialized: "知识库尚未初始化。",
  knowledge_base_change_package_invalid: "变更包未通过门禁检查。",
  knowledge_base_change_package_missing_document: "变更包缺少文档。",
  knowledge_base_provider_capability_denied:
    "知识库预检未通过：provider 拒绝了知识库写入分格，需修复该 provider 的拒绝原因或改用其他 provider 后重试。",
};

/**
 * 「知识库初始化」的二次确认文案：必须写明三件不可逆/易误解的事实——
 * ①会真实调用 provider；②会消耗额度；③会把结果写入聚合根目录。
 */
const BOOTSTRAP_CONFIRM_TEXT =
  "知识库初始化会真实调用 provider、消耗额度，并把生成结果写入聚合根目录。确认开始知识库初始化？";

const OPERATION_STATUS_LABELS: Record<
  KnowledgeBaseOperationSnapshotDto["status"],
  string
> = {
  running: "知识库操作进行中",
  completed: "知识库操作已完成",
  failed: "知识库操作失败",
  cancelled: "知识库操作已取消",
};

const OPERATION_STATUS_BADGE_CLASSES: Record<
  KnowledgeBaseOperationSnapshotDto["status"],
  string
> = {
  running:
    "border-[var(--aria-primary)] bg-[var(--aria-panel-muted)] text-[var(--aria-primary)]",
  completed: "border-emerald-300 bg-emerald-50 text-emerald-700",
  failed: "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-danger)]",
  cancelled:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

function stepIndicatorIcon(status: KnowledgeBaseOperationStepStatus) {
  if (status === "completed") {
    return <CircleCheck className="h-4 w-4 text-emerald-600" aria-hidden="true" />;
  }
  if (status === "failed") {
    return <CircleX className="h-4 w-4 text-[var(--aria-danger)]" aria-hidden="true" />;
  }
  if (status === "running") {
    return (
      <LoaderCircle
        className="h-4 w-4 animate-spin text-[var(--aria-primary)]"
        aria-hidden="true"
      />
    );
  }
  return <CircleMinus className="h-4 w-4 text-[var(--aria-ink-muted)]" aria-hidden="true" />;
}

export type KnowledgeBaseCardProps = {
  projectId: string;
  logicalCodebaseId: string;
  status: KnowledgeBaseStatusDto | null;
  operation: KnowledgeBaseOperationSnapshotDto | null;
  busy: boolean;
  /** 启动/更新请求期（预检）失败明细：没有 operation 承载，由宿主装配。 */
  preflightFailure: KnowledgeBasePreflightFailureDto | null;
  onStart: (kind: "bootstrap" | "update", packagePath?: string) => void;
  onCancel: () => void;
  onOpenDetail: () => void;
  onCreateBootstrapInputs: () => void;
  onDismissPreflightFailure: () => void;
};

export function KnowledgeBaseCard({
  projectId,
  logicalCodebaseId,
  status,
  operation,
  busy,
  preflightFailure,
  onStart,
  onCancel,
  onOpenDetail,
  onCreateBootstrapInputs,
  onDismissPreflightFailure,
}: KnowledgeBaseCardProps) {
  const [updateDialogOpen, setUpdateDialogOpen] = useState(false);
  // REQ-KB-13：后端把「聚合初始化未完成」的门盖在全部知识库端点入口
  // （含 status 端点），失败 = 422 + 稳定码 knowledge_base_requires_aggregate_initialization。
  // 宿主 hook 把 status 取数失败折进工作台级错误横幅，卡片拿不到失败码
  // ⇒ 卡片自行探测一次 status，只认后端给出的稳定码（不臆造 status 投影里
  // 并不存在的「前置未满足」布尔位）。
  const [aggregateGateUnmet, setAggregateGateUnmet] = useState(false);
  // 更新按钮闸门（与后端 update 预检口径一致）：initialization_status 必须是
  // complete **且** global_validation 必须是 passed——少任何一条后端都会拒，
  // 前端提前禁用，避免用户点了才被拒。
  const updateReady =
    status?.initialization_status === "complete" &&
    status?.global_validation === "passed";
  const operationRunning = operation?.status === "running";
  const missingInputs = preflightFailure?.details?.missing ?? [];
  const preflightViolations = preflightFailure?.details?.violations ?? [];
  const showCreateBootstrapInputs =
    preflightFailure !== null &&
    (missingInputs.length > 0 ||
      preflightFailure.code === "knowledge_base_inputs_incomplete");
  // 前置门未满足：探测命中，或写请求被前置门拒绝（同一稳定码）。
  const gateRejected =
    aggregateGateUnmet ||
    preflightFailure?.code === AGGREGATE_INITIALIZATION_REQUIRED_CODE;
  // 前置门失败已由预检失败面板承载同一句文案时，不再另起一条提示（避免同一
  // 句话在同一张卡片上出现两次）。
  const gatePromptShownByPreflight =
    preflightFailure?.code === AGGREGATE_INITIALIZATION_REQUIRED_CODE;
  const statusLabel = operation
    ? OPERATION_STATUS_LABELS[operation.status]
    : gateRejected
      ? "前置未满足"
      : status?.initialization_status === "complete"
        ? "初始化已完成"
        : "尚未初始化";
  // 起草完成（kind=user_input 且 completed）⇒ 下一步是「用户补充并确认输入
  // 文件」，卡片必须给出入口（OpenSpec change
  // add-knowledge-base-interactive-init-flow）。
  const userInputDraftCompleted =
    operation?.kind === "user_input" && operation.status === "completed";
  // 自动进入详情页的「已跳过」标记：同一 operation 只跳一次。
  const autoOpenedOperationRef = useRef<string | null>(null);
  // 上一份 operation 快照（id + status）：只在**观察到** running → completed
  // 的跃迁时自动跳转（打开抽屉时看到的是早已完成的起草 ⇒ 不抢焦点，只留
  // 卡片入口）。
  const previousOperationRef = useRef<{ id: string; status: string } | null>(
    null,
  );

  useEffect(() => {
    const current = operation
      ? { id: operation.operation_id, status: operation.status }
      : null;
    const previous = previousOperationRef.current;
    previousOperationRef.current = current;
    if (!operation || !userInputDraftCompleted || !previous) {
      return;
    }
    if (previous.id !== operation.operation_id || previous.status !== "running") {
      return;
    }
    if (autoOpenedOperationRef.current === operation.operation_id) {
      return;
    }
    autoOpenedOperationRef.current = operation.operation_id;
    onOpenDetail();
  }, [operation, userInputDraftCompleted, onOpenDetail]);

  // 前置门探测：命中稳定码即置「前置未满足」；取数成功即门已放行（清零）。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        await getKnowledgeBaseStatus(projectId, logicalCodebaseId);
        if (!cancelled) {
          setAggregateGateUnmet(false);
        }
      } catch (reason) {
        if (cancelled) {
          return;
        }
        setAggregateGateUnmet(isAggregateGateFailure(reason));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [projectId, logicalCodebaseId]);

  // 门未满足时定期复探：用户在同抽屉内完成聚合初始化后，知识库写入口要能
  // 自行恢复（不必关开抽屉）。
  useEffect(() => {
    if (!aggregateGateUnmet) {
      return;
    }
    const timer = window.setInterval(() => {
      void getKnowledgeBaseStatus(projectId, logicalCodebaseId).then(
        () => {
          setAggregateGateUnmet(false);
        },
        () => {
          // 复探失败仍维持「前置未满足」（只有取数成功才放行）。
        },
      );
    }, GATE_RECHECK_INTERVAL_MS);
    return () => {
      window.clearInterval(timer);
    };
  }, [aggregateGateUnmet, projectId, logicalCodebaseId]);

  function handleBootstrap() {
    if (!window.confirm(BOOTSTRAP_CONFIRM_TEXT)) {
      return;
    }
    onStart("bootstrap");
  }

  return (
    <section
      data-testid="knowledge-base-card"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--aria-ink)]">知识库</span>
          {operation ? (
            <span
              data-testid="knowledge-base-operation-status"
              data-status={operation.status}
              className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${OPERATION_STATUS_BADGE_CLASSES[operation.status]}`}
            >
              {operation.status}
            </span>
          ) : null}
          <span className="text-xs text-[var(--aria-ink-muted)]">{statusLabel}</span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            disabled={busy || gateRejected}
            onClick={handleBootstrap}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {busy && !updateReady ? (
              <LoaderCircle
                data-testid="knowledge-base-spinner"
                className="mr-1 h-4 w-4 animate-spin"
                aria-hidden="true"
              />
            ) : null}
            知识库初始化
          </button>
          <button
            type="button"
            disabled={busy || !updateReady || gateRejected}
            onClick={() => {
              setUpdateDialogOpen(true);
            }}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {busy && updateReady ? (
              <LoaderCircle
                data-testid="knowledge-base-spinner"
                className="mr-1 h-4 w-4 animate-spin"
                aria-hidden="true"
              />
            ) : null}
            知识库更新
          </button>
          <button
            type="button"
            onClick={onOpenDetail}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            查看知识库详细内容
          </button>
          {operationRunning ? (
            <button
              type="button"
              disabled={busy}
              onClick={onCancel}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              {busy ? (
                <LoaderCircle
                  data-testid="knowledge-base-spinner"
                  className="mr-1 h-4 w-4 animate-spin"
                  aria-hidden="true"
                />
              ) : null}
              取消知识库操作
            </button>
          ) : null}
        </div>
      </div>

      {/* 起草完成 ⇒ 下一步入口（OpenSpec change
          add-knowledge-base-interactive-init-flow）：用户要补充并确认这 9 份
          输入文件，之后才会建库。 */}
      {userInputDraftCompleted ? (
        <div
          data-testid="knowledge-base-user-input-cta"
          className="mt-2 rounded-md border border-amber-300 bg-amber-50 px-2 py-1.5 text-xs text-amber-900"
        >
          <p>输入草稿已生成（9 份 user-input 文件），请补充并确认后再执行知识库初始化。</p>
          <button
            type="button"
            data-testid="knowledge-base-open-user-input"
            onClick={onOpenDetail}
            className="mt-1 inline-flex h-7 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-2 text-xs font-semibold text-white transition-colors duration-200 hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            打开输入文件并补充
          </button>
        </div>
      ) : null}

      {/* 知识库的适用项目类型（能力边界）：Java Web / 纯 Java / 纯 Web。
          摆在按钮区正下方，避免其它技术栈的项目误点初始化。 */}
      <p
        data-testid="knowledge-base-supported-project-hint"
        className="mt-2 rounded border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-2 py-1 text-xs text-[var(--aria-ink)]"
      >
        {KNOWLEDGE_BASE_SUPPORTED_PROJECT_HINT}
      </p>

      {/* REQ-KB-13：前置门未满足 ⇒ 统一中文提示（与详情页逐字一致）。
          预检失败面板已经承载同一句时不重复展示（同一张卡片不出现同一句两遍）。 */}
      {gateRejected && !gatePromptShownByPreflight ? (
        <p
          data-testid="knowledge-base-aggregate-gate-hint"
          role="alert"
          className="mt-2 text-xs text-[var(--aria-danger)]"
        >
          {AGGREGATE_INITIALIZATION_REQUIRED_PROMPT}
        </p>
      ) : null}

      {!updateReady ? (
        <p
          data-testid="knowledge-base-update-hint"
          className="mt-2 text-xs text-[var(--aria-ink-muted)]"
        >
          需先完成知识库初始化且全局校验通过，才能执行知识库更新。
        </p>
      ) : null}

      {operation ? (
        <ol className="mt-3 grid gap-1.5">
          {operation.steps.map((step) => (
            <li
              key={step.step_id}
              data-testid={`knowledge-base-step-${step.step_id}`}
              data-status={step.status}
              className="flex items-center gap-2 text-xs text-[var(--aria-ink-muted)]"
            >
              {stepIndicatorIcon(step.status)}
              <span>{step.step_id}</span>
              {step.status === "running" ? (
                <span className="text-[var(--aria-primary)]">进行中</span>
              ) : null}
              {step.status === "failed" ? (
                <span className="text-[var(--aria-danger)]">失败</span>
              ) : null}
            </li>
          ))}
        </ol>
      ) : null}

      {operation?.provider_selection ? (
        <p
          data-testid="knowledge-base-provider-selection"
          className="mt-2 text-xs text-[var(--aria-ink-muted)]"
        >
          <ProviderSelectionSummary selection={operation.provider_selection} />
        </p>
      ) : null}

      {preflightFailure ? (
        <div
          data-testid="knowledge-base-preflight-failure"
          role="alert"
          className="mt-2 rounded-md border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-xs text-[var(--aria-danger)]"
        >
          <div className="flex items-center gap-2">
            <TriangleAlert className="h-4 w-4 shrink-0" aria-hidden="true" />
            <p data-testid="knowledge-base-preflight-failure-headline">
              {PREFLIGHT_FAILURE_LABELS[preflightFailure.code] ??
                `知识库预检未通过：${preflightFailure.message}`}
            </p>
          </div>
          {missingInputs.length > 0 ? (
            <div className="mt-1 pl-6">
              <p className="font-semibold">缺失文件：</p>
              <ul data-testid="knowledge-base-preflight-missing" className="mt-0.5 grid gap-0.5">
                {missingInputs.map((path) => (
                  <li key={path} className="font-mono">
                    {path}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
          {preflightViolations.length > 0 ? (
            <ul
              data-testid="knowledge-base-preflight-violations"
              className="mt-1 grid gap-0.5 pl-6"
            >
              {preflightViolations.map((violation, index) => (
                <li key={`${violation.code}-${index}`}>
                  {violation.document !== null ? `${violation.document} · ` : ""}
                  {violation.field !== null ? `${violation.field} · ` : ""}
                  {violation.detail}
                </li>
              ))}
            </ul>
          ) : null}
          <div className="mt-2 flex flex-wrap gap-2 pl-6">
            {showCreateBootstrapInputs ? (
              <button
                type="button"
                disabled={busy || gateRejected}
                onClick={onCreateBootstrapInputs}
                className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 text-[11px] font-semibold text-[var(--aria-ink)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
              >
                {busy ? (
                  <LoaderCircle
                    data-testid="knowledge-base-spinner"
                    className="mr-1 h-4 w-4 animate-spin"
                    aria-hidden="true"
                  />
                ) : null}
                创建输入模板
              </button>
            ) : null}
            <button
              type="button"
              onClick={onDismissPreflightFailure}
              className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 text-[11px] font-semibold text-[var(--aria-ink)]"
            >
              知道了
            </button>
          </div>
        </div>
      ) : null}

      {operation?.status === "failed" && operation.error ? (
        <div
          role="alert"
          className="mt-2 flex items-center gap-2 rounded-md border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-xs text-[var(--aria-danger)]"
        >
          <TriangleAlert className="h-4 w-4 shrink-0" aria-hidden="true" />
          {operation.error.message}
        </div>
      ) : null}

      <KnowledgeBaseUpdateDialog
        projectId={projectId}
        logicalCodebaseId={logicalCodebaseId}
        open={updateDialogOpen}
        onClose={() => {
          setUpdateDialogOpen(false);
        }}
        onStart={(kind, packagePath) => {
          setUpdateDialogOpen(false);
          onStart(kind, packagePath);
        }}
      />
    </section>
  );
}
