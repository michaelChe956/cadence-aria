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
};

export function AggregateInitializationCard({
  operation,
  busy,
  onStart,
  onCancel,
  onReinitialize,
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
