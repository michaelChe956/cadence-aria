import { CircleCheck, CircleX, LoaderCircle, TriangleAlert } from "lucide-react";
import type {
  AggregateInitializationOperationSnapshot,
  AggregateInitializationOperationStatus,
  AggregateInitializationStepStatus,
} from "../../api/types";

const STEP_LABELS: Record<string, string> = {
  machine_skills: "Machine Skills",
  aggregate_preflight: "聚合预检",
  pre_check: "预检",
  rule_and_mcp_config: "规则与 MCP 配置",
  openspec_and_examples: "OpenSpec 与示例",
};

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
  return (
    <span className="h-4 w-4 shrink-0 rounded-full border border-[var(--aria-line)]" />
  );
}

export type AggregateInitializationCardProps = {
  operation: AggregateInitializationOperationSnapshot | null;
  busy: boolean;
  onStart: () => void;
  onCancel: () => void;
};

export function AggregateInitializationCard({
  operation,
  busy,
  onStart,
  onCancel,
}: AggregateInitializationCardProps) {
  const active =
    operation !== null &&
    (operation.status === "created" || operation.status === "running");
  const canStart =
    operation === null ||
    operation.status === "failed" ||
    operation.status === "cancelled";
  const statusLabel = operation ? STATUS_LABELS[operation.status] : "尚未初始化";

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
            </li>
          ))}
        </ol>
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
