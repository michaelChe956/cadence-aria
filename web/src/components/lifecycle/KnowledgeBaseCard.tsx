// Task 9：LC 仓管理抽屉内的「知识库」卡片。
// 与 AggregateInitializationCard 同构：卡片头 + 按钮区 + 步骤列表 + 失败文案；
// provider 选择摘要组件（含 SKIPPED_REASON_LABELS）从聚合卡提升至此共用，
// AggregateInitializationCard 保留 re-export 以维持既有导入路径稳定。
import { useState } from "react";
import {
  CircleCheck,
  CircleMinus,
  CircleX,
  LoaderCircle,
  TriangleAlert,
} from "lucide-react";
import type { AggregateProviderSelectionDto } from "../../api/types";
import type {
  KnowledgeBaseChangePackageViolationDto,
  KnowledgeBaseOperationSnapshotDto,
  KnowledgeBaseOperationStepStatus,
  KnowledgeBaseStatusDto,
} from "../../api/types/knowledge-base";
import { KnowledgeBaseUpdateDialog } from "./KnowledgeBaseUpdateDialog";

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
  knowledge_base_requires_aggregate_initialization:
    "知识库预检未通过：需先完成聚合初始化。",
  knowledge_base_skills_unavailable: "知识库预检未通过：Cadence skills 当前不可用。",
  knowledge_base_inputs_incomplete: "知识库预检未通过：输入模板不完整。",
  knowledge_base_manifest_missing: "知识库预检未通过：manifest 缺失。",
  knowledge_base_manifest_version_unsupported:
    "知识库预检未通过：manifest 版本不受支持。",
  knowledge_base_not_initialized: "知识库尚未初始化。",
  knowledge_base_change_package_invalid: "变更包未通过门禁检查。",
  knowledge_base_change_package_path_invalid: "变更包路径不合法。",
  knowledge_base_change_package_missing_document: "变更包缺少文档。",
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
  const statusLabel = operation
    ? OPERATION_STATUS_LABELS[operation.status]
    : status?.initialization_status === "complete"
      ? "初始化已完成"
      : "尚未初始化";

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
            disabled={busy}
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
            disabled={busy || !updateReady}
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
                disabled={busy}
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
