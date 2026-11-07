import {
  CircleCheck,
  CircleMinus,
  CircleX,
  LoaderCircle,
  TriangleAlert,
} from "lucide-react";
import type {
  AggregateInitializationOperationSnapshot,
  AggregateInitializationOperationStatus,
  AggregateInitializationStepStatus,
  AggregateProviderSelectionDto,
} from "../../api/types";

const STEP_LABELS: Record<string, string> = {
  machine_skills: "Machine Skills",
  aggregate_preflight: "聚合预检",
  pre_check: "预检",
  rule_and_mcp_config: "规则与 MCP 配置",
  openspec_and_examples: "OpenSpec 与示例",
};

/**
 * 冷启动推进的停止原因（advance / 手动「准备」被拒时落盘，GET 投影透出）。
 *
 * 与冷启动卡片（`LogicalCodebaseBootstrapCard`）同一份后端事实、同一套文案
 * 来源——聚合初始化卡片只把它挂在按钮旁，让用户看到「卡在哪一步、为什么」，
 * 不新造措辞（机器码与原始诊断由 bootstrap 通知卡承担，同一信息只呈现一次）。
 */
export type AggregateInitializationAdvanceStop = {
  /** 停住的步骤（依赖顺序里的那一步）。 */
  step: string;
  /** 停止枚举（waiting_for_human / failed / no_actionable_step …），不是中文文案。 */
  reason: string;
  /** 稳定机器码——两道闸门的区分键。 */
  reason_code: string;
  /** 已脱敏的原始拒绝上下文（拒绝 message + details），可空。 */
  detail?: string | null;
  /** 停止时间。 */
  stopped_at: string;
};

/** 冷启动步骤名 → 中文标签（与冷启动卡片同一套）。 */
const BOOTSTRAP_STEP_LABELS: Record<string, string> = {
  identity: "身份",
  manifest_checkout: "清单/检出",
  rules_policy: "规则/政策",
  member_index: "成员索引",
  aggregate_index_active: "聚合索引激活",
};

/**
 * advance 停止原因的中文口径（与冷启动卡片同一份标签表）。
 * in_progress = 长任务（provider turn / 索引构建）正在跑——不是错误。
 */
const ADVANCE_STOP_LABELS: Record<string, string> = {
  planning_ready: "已就绪（PlanningReady）",
  in_progress: "进行中，稍后刷新",
  failed: "已失败",
  waiting_for_human: "等待人工处理",
  no_actionable_step: "没有可推进的步骤",
  iteration_limit: "已达单次推进上限",
};

/**
 * 停止原因的两道闸门区分键（后端稳定码）——与冷启动卡片同一套短标签：
 * 让用户一眼看出该去核验 provider 还是去完成信任确认。
 */
const ADVANCE_STOP_GATE_LABELS: Record<string, string> = {
  aggregate_initialization_provider_needs_verification: "需先核验 provider",
  aggregate_initialization_trust_waiting: "需先完成信任确认",
};

/**
 * 「已跳过」步骤的说明文案：服务端判定该步骤本次不执行（对应命令未运行）。
 * 具体判定依据只落在服务端 operation 记录里，不随步骤状态投影到前端，
 * 故此处只陈述「被判定跳过 / 未执行」这一可观测事实。
 */
const SKIPPED_STEP_HINT =
  "该步骤本次被判定跳过：服务端确认这一步无需执行，对应命令未运行。";

const STATUS_LABELS: Record<AggregateInitializationOperationStatus, string> = {
  created: "等待启动",
  running: "初始化中",
  completed: "初始化完成",
  failed: "初始化失败",
  cancelled: "已取消",
};

const STATUS_BADGE_CLASSES: Record<
  AggregateInitializationOperationStatus,
  string
> = {
  created:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
  running:
    "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]",
  completed:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  failed:
    "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-danger)]",
  cancelled:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
};

function StepIndicator({ status }: { status: AggregateInitializationStepStatus }) {
  return (
    <span
      data-testid="aggregate-initialization-step-indicator"
      data-status={status}
      className="inline-flex h-4 w-4 shrink-0"
    >
      {stepIndicatorIcon(status)}
    </span>
  );
}

function stepIndicatorIcon(status: AggregateInitializationStepStatus) {
  if (status === "running") {
    return (
      <LoaderCircle
        className="h-4 w-4 shrink-0 animate-spin text-[var(--aria-primary)]"
        aria-hidden="true"
      />
    );
  }
  if (status === "completed") {
    return (
      <CircleCheck
        className="h-4 w-4 shrink-0 text-[var(--aria-success)]"
        aria-hidden="true"
      />
    );
  }
  if (status === "failed") {
    return (
      <CircleX
        className="h-4 w-4 shrink-0 text-[var(--aria-danger)]"
        aria-hidden="true"
      />
    );
  }
  if (status === "skipped") {
    // 「已跳过」既不是进行中也不是成功：中性色减号圈，与 pending 的空心圈
    // 区分开，避免用户把跳过误读成「未开始」。
    return (
      <CircleMinus
        className="h-4 w-4 shrink-0 text-[var(--aria-ink-muted)]"
        aria-hidden="true"
      />
    );
  }
  return (
    <span className="h-4 w-4 shrink-0 rounded-full border border-[var(--aria-line)]" />
  );
}

/**
 * 「重新初始化」的二次确认文案：必须写明三件不可逆/易误解的事实——
 * ①强制重跑完整五步（含四条 Cadence-skills 命令，不因已初始化而跳过）；
 * ②适用场景是 Cadence-skills 更新后重装；③会重新合成并发布新的聚合政策版本。
 */
const REINITIALIZE_CONFIRM_TEXT =
  "重新初始化会强制重跑完整五步：/pre-check、/rule-config、/mcp-configuration、/project-rules-examples 四条 Cadence-skills 命令一律执行（不因聚合根已初始化而跳过），适用于 Cadence-skills 更新后重装；执行后会重新合成并发布新的聚合政策版本（policy/<项目>/<逻辑代码库>/<版本+1>）。确认重新初始化？";
/** 跳过原因码 → 摘要行中文标签（与后端 skipped_reason 词表一一对应）。 */
const SKIPPED_REASON_LABELS: Record<string, string> = {
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


export type AggregateInitializationCardProps = {
  operation: AggregateInitializationOperationSnapshot | null;
  busy: boolean;
  onStart: () => void;
  onCancel: () => void;
  onReinitialize: () => void;
  /**
   * 冷启动推进的停止原因（可选）：聚合初始化卡片本身没有 operation 时
   * （点「启动聚合初始化」失败、或推进在准备阶段被拒），用户就只看到一个
   * 无解释的按钮——把后端落盘的停止原因挂在按钮旁，让用户看到「卡在哪一步、
   * 为什么、属于哪道闸门」。
   *
   * 未推进过 / 未提供时为 undefined ⇒ 卡片维持现状（不渲染该块）。
   */
  advanceStop?: AggregateInitializationAdvanceStop | null;
};

export function AggregateInitializationCard({
  operation,
  busy,
  onStart,
  onCancel,
  onReinitialize,
  advanceStop,
}: AggregateInitializationCardProps) {
  const active =
    operation !== null &&
    (operation.status === "created" || operation.status === "running");
  // 四态互斥：null→启动；created|running→取消；completed|failed|cancelled→重新初始化。
  const canStart = operation === null;
  const canReinitialize =
    operation !== null &&
    (operation.status === "completed" ||
      operation.status === "failed" ||
      operation.status === "cancelled");
  const statusLabel = operation ? STATUS_LABELS[operation.status] : "尚未初始化";
  // 停止原因属于哪道闸门（provider 选择门 / trust 门）——未知原因码不硬套
  // 标签，如实只给中文口径。
  const advanceStopGateLabel = advanceStop
    ? (ADVANCE_STOP_GATE_LABELS[advanceStop.reason_code] ?? null)
    : null;

  function handleReinitialize() {
    if (!window.confirm(REINITIALIZE_CONFIRM_TEXT)) {
      return;
    }
    onReinitialize();
  }

  return (
    <section
      data-testid="aggregate-initialization-card"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--aria-ink)]">
            聚合初始化
          </span>
          {operation ? (
            <span
              data-testid="aggregate-initialization-status"
              data-status={operation.status}
              className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${STATUS_BADGE_CLASSES[operation.status]}`}
            >
              {operation.status}
            </span>
          ) : null}
          <span className="text-xs text-[var(--aria-ink-muted)]">
            {statusLabel}
          </span>
        </div>
        {canStart ? (
          <button
            type="button"
            disabled={busy}
            onClick={onStart}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {busy ? (
              <LoaderCircle
                data-testid="aggregate-initialization-spinner"
                className="mr-1 h-4 w-4 animate-spin"
                aria-hidden="true"
              />
            ) : null}
            启动聚合初始化
          </button>
        ) : null}
        {active ? (
          <button
            type="button"
            disabled={busy}
            onClick={onCancel}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {busy ? (
              <LoaderCircle
                data-testid="aggregate-initialization-spinner"
                className="mr-1 h-4 w-4 animate-spin"
                aria-hidden="true"
              />
            ) : null}
            取消初始化
          </button>
        ) : null}
        {canReinitialize ? (
          <button
            type="button"
            disabled={busy}
            onClick={handleReinitialize}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {busy ? (
              <LoaderCircle
                data-testid="aggregate-initialization-spinner"
                className="mr-1 h-4 w-4 animate-spin"
                aria-hidden="true"
              />
            ) : null}
            重新初始化
          </button>
        ) : null}
      </div>

      <p
        data-testid="aggregate-initialization-aggregate-order-notice"
        className="mb-2 rounded border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-2 py-1 text-[11px] text-[var(--aria-danger)]"
      >
        聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi &gt; Claude Code &gt; Codex。
      </p>
      {operation ? (
        <ol className="mt-3 grid gap-1.5">
          {operation.steps.map((step) => (
            <li
              key={step.step_id}
              data-testid={`aggregate-initialization-step-${step.step_id}`}
              data-status={step.status}
              className="flex items-center gap-2 text-xs text-[var(--aria-ink-muted)]"
            >
              <StepIndicator status={step.status} />
              <span>{STEP_LABELS[step.step_id] ?? step.step_id}</span>
              {step.status === "running" ? (
                <span className="text-[var(--aria-primary)]">进行中</span>
              ) : null}
              {step.status === "failed" ? (
                <span className="text-[var(--aria-danger)]">失败</span>
              ) : null}
              {step.status === "skipped" ? (
                <span
                  data-testid="aggregate-initialization-step-skipped-label"
                  title={SKIPPED_STEP_HINT}
                  className="text-[var(--aria-ink-muted)]"
                >
                  已跳过
                </span>
              ) : null}
            </li>
          ))}
        </ol>
      ) : null}

      {advanceStop ? (
        // 聚合初始化为什么没开始：**只给通知卡没有的东西**——停在哪一步、
        // 属于哪道闸门。机器码与原始诊断由 bootstrap 通知卡
        // （`bootstrap:advance-stop:*`，通用渲染）承担：它的 summary 已含
        // 「机器码：…」与「诊断：…」，另有自己的机器码次要行——同一份信息
        // 只呈现一次（与冷启动卡片同一口径）。
        <div
          data-testid="aggregate-initialization-advance-stop"
          className="mt-2 rounded border border-amber-200 bg-amber-50 px-2 py-1.5 text-xs leading-snug"
        >
          <p
            className="text-slate-700"
            data-testid="aggregate-initialization-advance-stop-headline"
          >
            {`聚合初始化停在「${BOOTSTRAP_STEP_LABELS[advanceStop.step] ?? advanceStop.step}」：${ADVANCE_STOP_LABELS[advanceStop.reason] ?? advanceStop.reason}`}
            {advanceStopGateLabel ? (
              <span
                className="ml-1 rounded bg-amber-200 px-1.5 py-0.5 text-[11px] font-semibold text-amber-900"
                data-testid="aggregate-initialization-advance-stop-gate"
              >
                {advanceStopGateLabel}
              </span>
            ) : null}
          </p>
        </div>
      ) : null}

      {operation?.provider_selection ? (
        <p
          data-testid="aggregate-initialization-provider-selection"
          className="mt-2 text-xs text-[var(--aria-ink-muted)]"
        >
          {providerSelectionSummary(operation.provider_selection)}
        </p>
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

      {operation?.status === "cancelled" && operation.cancellation ? (
        <div className="mt-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs text-[var(--aria-warning)]">
          取消原因：{operation.cancellation.reason_code}
          {operation.cancellation.detail
            ? ` · ${operation.cancellation.detail}`
            : ""}
        </div>
      ) : null}
    </section>
  );
}
