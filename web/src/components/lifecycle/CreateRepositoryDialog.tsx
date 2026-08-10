import { Check, Circle, RefreshCw, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { ApiRequestError } from "../../api/client";
import type {
  CreateRepositoryRequest,
  CreateRepositoryResponse,
  RepositoryInitializationOperationSnapshot,
  RepositoryInitializationStep,
  RepositoryInitializationStepId,
  RepositoryRegistrationErrorDetails,
  WorkspaceProviderName,
} from "../../api/types";
import { getProviderOptions } from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";

const POLL_INTERVAL_MS = 1_000;

const STEP_LABELS: Record<RepositoryInitializationStepId, string> = {
  cadence_skills: "准备 Cadence Skills",
  pre_check: "执行预检查",
  rule_config: "配置规则",
  mcp_configuration: "配置 MCP",
  project_rules_examples: "生成项目规则示例",
  git_finalize: "提交并推送",
};

type CreateRepositoryDialogProps = {
  onCreate: (
    payload: CreateRepositoryRequest,
  ) =>
    | Promise<RepositoryInitializationOperationSnapshot>
    | RepositoryInitializationOperationSnapshot;
  onFetchOperation: (
    operationId: string,
  ) =>
    | Promise<RepositoryInitializationOperationSnapshot>
    | RepositoryInitializationOperationSnapshot;
  onInitializationCompleted: (
    result: CreateRepositoryResponse,
  ) => void | Promise<void>;
  onClose: () => void;
};

export function CreateRepositoryDialog({
  onCreate,
  onFetchOperation,
  onInitializationCompleted,
  onClose,
}: CreateRepositoryDialogProps) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [policyPreset, setPolicyPreset] = useState("manual-write");
  const [providerMode, setProviderMode] =
    useState<WorkspaceProviderName>("codex");
  const [validationError, setValidationError] = useState<string | null>(null);
  const [registrationError, setRegistrationError] = useState<{
    message: string;
    details: RepositoryRegistrationErrorDetails;
  } | null>(null);
  const [operation, setOperation] =
    useState<RepositoryInitializationOperationSnapshot | null>(null);
  const [pollingError, setPollingError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const fetchOperationRef = useRef(onFetchOperation);
  const initializationCompletedRef = useRef(onInitializationCompleted);
  const dialogRef = useRef<HTMLFormElement>(null);
  const nameInputRef = useRef<HTMLInputElement>(null);
  const shouldFocusNameInputRef = useRef(false);
  const completedOperationIdsRef = useRef(new Set<string>());
  const availabilitySnapshot = useProviderAvailabilityStore(
    (state) => state.snapshot,
  );
  const providerOptions = getProviderOptions(availabilitySnapshot);
  const visibleProviderOptions = providerOptions.filter(
    (option) =>
      // capability policy: 仅 Claude Code 可初始化仓库，Kimi 同 Pi 不参与
      option.value !== "pi" &&
      option.value !== "kimi_code" &&
      (option.visible || option.value === providerMode),
  );
  const unavailableProviderOptions = visibleProviderOptions.filter(
    (option) => option.disabled,
  );
  const claudeCode = providerOptions.find(
    (option) => option.value === "claude_code",
  )!;
  const isOperationRunning =
    operation?.status === "created" || operation?.status === "running";
  const hasCompletedResult =
    operation?.status === "completed" && operation.result !== null;
  const operationId = operation?.operation_id;
  const completedResult = hasCompletedResult ? operation.result : null;
  const completedWithoutResultError =
    operation?.status === "completed" && operation.result === null
      ? {
          message: "代码库初始化已完成，但服务未返回初始化结果",
          details: {
            reason: "初始化操作缺少结果",
            action: "请重新填写并提交代码库信息",
          },
        }
      : null;
  const failedOperationError =
    operation?.status === "failed"
      ? operation.error
        ? { message: operation.error.message, details: operation.error.details }
        : { message: "代码库初始化失败", details: {} }
      : null;

  fetchOperationRef.current = onFetchOperation;
  initializationCompletedRef.current = onInitializationCompleted;

  useEffect(() => {
    if (!operationId || !isOperationRunning) {
      return;
    }

    let disposed = false;
    let inFlight = false;
    const refresh = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        const snapshot = await fetchOperationRef.current(operationId);
        if (!disposed) {
          setOperation(snapshot);
          setPollingError(null);
        }
      } catch (error) {
        if (!disposed) {
          setPollingError(
            error instanceof Error ? error.message : "正在重试获取初始化状态",
          );
        }
      } finally {
        inFlight = false;
      }
    };

    void refresh();
    const interval = window.setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [operationId, isOperationRunning]);

  useEffect(() => {
    if (!operationId || !hasCompletedResult || !completedResult) {
      return;
    }
    if (completedOperationIdsRef.current.has(operationId)) {
      return;
    }

    completedOperationIdsRef.current.add(operationId);
    void initializationCompletedRef.current(completedResult);
  }, [completedResult, hasCompletedResult, operationId]);

  useEffect(() => {
    if (isOperationRunning) {
      dialogRef.current?.focus();
    }
  }, [isOperationRunning]);

  useEffect(() => {
    if (shouldFocusNameInputRef.current && operation === null) {
      shouldFocusNameInputRef.current = false;
      nameInputRef.current?.focus();
    }
  }, [operation]);

  function clearErrors() {
    setValidationError(null);
    setRegistrationError(null);
  }

  function handleRefill() {
    shouldFocusNameInputRef.current = true;
    clearErrors();
    setPollingError(null);
    setOperation(null);
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submittingRef.current) {
      return;
    }

    const trimmedName = name.trim();
    const trimmedPath = path.trim();
    if (!trimmedName) {
      setValidationError("请输入代码库名称");
      return;
    }
    if (!trimmedPath) {
      setValidationError("请输入本地路径");
      return;
    }
    if (!claudeCode.available) {
      setValidationError(
        "代码库初始化固定要求 Claude Code，请先安装或启用 Claude Code",
      );
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    clearErrors();
    setPollingError(null);
    try {
      const snapshot = await onCreate({
        name: trimmedName,
        path: trimmedPath,
        default_policy_preset: policyPreset,
        default_provider_mode: providerMode,
      });
      setOperation(snapshot);
    } catch (reason) {
      if (reason instanceof ApiRequestError) {
        setRegistrationError({
          message: reason.message,
          details: reason.details,
        });
      } else {
        setRegistrationError({
          message: reason instanceof Error ? reason.message : "添加代码库失败",
          details: {},
        });
      }
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  const closeDisabled = submitting || isOperationRunning;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <form
        ref={dialogRef}
        role="dialog"
        tabIndex={-1}
        aria-label="添加代码库"
        aria-modal="true"
        onSubmit={handleSubmit}
        className="w-full max-w-md rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">
            添加代码库
          </h2>
          <button
            type="button"
            onClick={onClose}
            disabled={closeDisabled}
            className="cursor-pointer rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)] disabled:cursor-not-allowed"
          >
            关闭
          </button>
        </div>
        {operation ? (
          completedResult ? (
            <>
              <RepositoryInitializationProgress operation={operation} />
              <RepositoryInitializationSuccess
                response={completedResult}
                operation={operation}
              />
              <div className="mt-4 flex justify-end">
                <button
                  type="button"
                  onClick={onClose}
                  className="cursor-pointer rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white"
                >
                  完成
                </button>
              </div>
            </>
          ) : (
            <>
              <RepositoryInitializationProgress operation={operation} />
              {isOperationRunning ? (
                <p className="mt-3 text-sm text-[var(--aria-ink-muted)]">
                  正在初始化，请保持此窗口打开
                </p>
              ) : null}
              {pollingError ? (
                <p
                  role="status"
                  aria-live="polite"
                  className="mt-3 text-sm text-[var(--aria-danger)]"
                >
                  {pollingError}
                </p>
              ) : null}
              {failedOperationError ? (
                <div className="mt-3">
                  <RepositoryRegistrationError error={failedOperationError} />
                </div>
              ) : null}
              {completedWithoutResultError ? (
                <div className="mt-3">
                  <RepositoryRegistrationError error={completedWithoutResultError} />
                </div>
              ) : null}
              <div className="mt-4 flex justify-end gap-2">
                {isOperationRunning ? (
                  <button
                    type="button"
                    onClick={onClose}
                    disabled
                    className="cursor-pointer rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink-muted)] disabled:cursor-not-allowed"
                  >
                    取消
                  </button>
                ) : operation.status === "failed" || completedWithoutResultError ? (
                  <button
                    type="button"
                    onClick={handleRefill}
                    className="cursor-pointer rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white"
                  >
                    重新填写
                  </button>
                ) : null}
              </div>
            </>
          )
        ) : (
          <>
            <div className="space-y-3">
              <label className="block text-sm font-semibold text-[var(--aria-ink)]">
                代码库名称
                <input
                  ref={nameInputRef}
                  value={name}
                  onChange={(event) => {
                    setName(event.target.value);
                    clearErrors();
                  }}
                  disabled={submitting}
                  className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
                />
              </label>
              <label className="block text-sm font-semibold text-[var(--aria-ink)]">
                本地路径
                <input
                  value={path}
                  onChange={(event) => {
                    setPath(event.target.value);
                    clearErrors();
                  }}
                  disabled={submitting}
                  className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 font-mono text-sm font-normal text-[var(--aria-ink)]"
                />
              </label>
              <label className="block text-sm font-semibold text-[var(--aria-ink)]">
                Policy
                <select
                  value={policyPreset}
                  onChange={(event) => {
                    setPolicyPreset(event.target.value);
                    clearErrors();
                  }}
                  disabled={submitting}
                  className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
                >
                  <option value="manual-write">manual-write</option>
                  <option value="manual-all">manual-all</option>
                  <option value="auto-review">auto-review</option>
                  <option value="non-interactive">non-interactive</option>
                </select>
              </label>
              <label className="block text-sm font-semibold text-[var(--aria-ink)]">
                Provider
                <select
                  value={providerMode}
                  onChange={(event) => {
                    setProviderMode(event.target.value as WorkspaceProviderName);
                    clearErrors();
                  }}
                  disabled={submitting}
                  className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
                >
                  {visibleProviderOptions.map((provider) => (
                    <option
                      key={provider.value}
                      value={provider.value}
                      disabled={provider.disabled}
                    >
                      {provider.label}
                    </option>
                  ))}
                </select>
              </label>
              {!claudeCode.available ? (
                <p className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-800">
                  {
                    "代码库初始化固定要求 Claude Code，请先安装或启用 Claude Code；不会降级使用 Codex。"
                  }
                </p>
              ) : null}
              {unavailableProviderOptions.length > 0 ? (
                <div className="space-y-1 text-xs text-amber-700">
                  {unavailableProviderOptions.map((provider) => (
                    <p key={provider.value}>
                      <span>{provider.reason}</span>
                      {provider.installHint ? (
                        <span className="ml-1">{provider.installHint}</span>
                      ) : null}
                    </p>
                  ))}
                </div>
              ) : null}
              {validationError ? (
                <p
                  role="alert"
                  className="text-sm font-semibold text-[var(--aria-danger)]"
                >
                  {validationError}
                </p>
              ) : null}
              {registrationError ? (
                <RepositoryRegistrationError error={registrationError} />
              ) : null}
            </div>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={onClose}
                disabled={closeDisabled}
                className="cursor-pointer rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink-muted)] disabled:cursor-not-allowed"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={submitting || !claudeCode.available}
                className="cursor-pointer rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:cursor-not-allowed disabled:opacity-60"
              >
                添加代码库
              </button>
            </div>
          </>
        )}
      </form>
    </div>
  );
}

function RepositoryInitializationProgress({
  operation,
}: {
  operation: RepositoryInitializationOperationSnapshot;
}) {
  const completedCount = operation.steps.filter(
    (step) => step.status === "completed",
  ).length;
  const runningStep = operation.steps.find((step) => step.status === "running");
  const currentStep = operation.current_step ?? runningStep?.step_id ?? null;
  const failedStep = operation.failed_step;
  const statusSummary =
    operation.status === "completed" && operation.result === null
      ? `初始化结果缺失。已完成 ${completedCount} / ${operation.steps.length}。`
      : operation.status === "completed"
      ? `初始化完成。已完成 ${completedCount} / ${operation.steps.length}。`
      : operation.status === "failed"
        ? `初始化失败：${failedStep ? STEP_LABELS[failedStep] : "未知步骤"}。已完成 ${completedCount} / ${operation.steps.length}。`
        : currentStep
          ? `正在执行：${STEP_LABELS[currentStep]}。已完成 ${completedCount} / ${operation.steps.length}。`
          : `等待执行。已完成 ${completedCount} / ${operation.steps.length}。`;

  return (
    <section className="space-y-3" aria-label="代码库初始化进度">
      <p
        role="status"
        aria-live="polite"
        aria-atomic="true"
        className="text-sm text-[var(--aria-ink-muted)]"
      >
        {statusSummary}
      </p>
      <ol aria-label="初始化步骤" className="space-y-2">
        {operation.steps.map((step) => (
          <li
            key={step.step_id}
            aria-current={step.status === "running" ? "step" : undefined}
            className="flex items-center gap-2 text-sm text-[var(--aria-ink)]"
          >
            <RepositoryInitializationStepIcon step={step} />
            <span className="min-w-0 flex-1">{STEP_LABELS[step.step_id]}</span>
            <span className="text-xs text-[var(--aria-ink-muted)]">
              {stepStatusLabel(step.status)}
            </span>
          </li>
        ))}
      </ol>
    </section>
  );
}

function RepositoryInitializationStepIcon({
  step,
}: {
  step: RepositoryInitializationStep;
}) {
  if (step.status === "completed") {
    return (
      <Check
        aria-hidden="true"
        className="h-4 w-4 shrink-0 text-[var(--aria-success)]"
      />
    );
  }
  if (step.status === "running") {
    return (
      <RefreshCw
        aria-hidden="true"
        className="h-4 w-4 shrink-0 animate-spin motion-reduce:animate-none"
      />
    );
  }
  if (step.status === "failed") {
    return (
      <TriangleAlert
        aria-hidden="true"
        className="h-4 w-4 shrink-0 text-[var(--aria-danger)]"
      />
    );
  }
  return (
    <Circle
      aria-hidden="true"
      className="h-4 w-4 shrink-0 text-[var(--aria-ink-muted)]"
    />
  );
}

function stepStatusLabel(status: RepositoryInitializationStep["status"]) {
  switch (status) {
    case "completed":
      return "已完成";
    case "running":
      return "正在执行";
    case "failed":
      return "失败";
    case "pending":
      return "等待执行";
  }
}

function RepositoryInitializationSuccess({
  response,
  operation,
}: {
  response: CreateRepositoryResponse;
  operation: RepositoryInitializationOperationSnapshot;
}) {
  const { initialization } = response;
  const gitFinalizeFailed = operation.steps.some(
    (step) => step.step_id === "git_finalize" && step.status === "failed",
  );
  return (
    <section className="space-y-3 text-sm" aria-label="代码库初始化结果">
      <h3 className="font-semibold text-[var(--aria-ink)]">代码库初始化完成</h3>
      <dl className="grid grid-cols-[7rem_minmax(0,1fr)] gap-x-2 gap-y-1">
        <dt className="text-[var(--aria-ink-muted)]">source</dt>
        <dd>{initialization.source}</dd>
        <dt className="text-[var(--aria-ink-muted)]">completed_at</dt>
        <dd>{initialization.completed_at}</dd>
      </dl>
      {initialization.commands.length > 0 ? (
        <div>
          <div className="font-semibold">completed commands</div>
          <ul className="list-disc pl-5 font-mono text-xs">
            {initialization.commands.map((command) => (
              <li key={command.index}>{command.command}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {initialization.changed_paths.length > 0 ? (
        <div>
          <div className="font-semibold">changed_paths</div>
          <ul className="list-disc pl-5 font-mono text-xs">
            {initialization.changed_paths.map((path) => (
              <li key={path}>{path}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {initialization.warnings.length > 0 ? (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-amber-800">
          <div className="font-semibold">warnings</div>
          <ul className="list-disc pl-5">
            {initialization.warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {gitFinalizeFailed ? (
        <div
          role="alert"
          aria-live="assertive"
          className="rounded-md border border-[var(--aria-danger)]/30 bg-red-50 px-3 py-2 text-[var(--aria-danger)]"
        >
          <p>自动提交推送未完成，请在目标仓库手动执行 git commit / git push</p>
          {initialization.git_finalize_warning ? (
            <p className="mt-1">{initialization.git_finalize_warning}</p>
          ) : null}
        </div>
      ) : initialization.git_finalize_warning ? (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-amber-800">
          {initialization.git_finalize_warning}
        </div>
      ) : null}
    </section>
  );
}

function RepositoryRegistrationError({
  error,
}: {
  error: { message: string; details: RepositoryRegistrationErrorDetails };
}) {
  const reason = detailString(error.details, "reason");
  const stage = detailString(error.details, "stage");
  const provider = detailString(error.details, "provider");
  const command = detailString(error.details, "command");
  const reasonCode = detailString(error.details, "reason_code");
  const stderrSummary = detailString(error.details, "stderr_summary");
  const action = detailString(error.details, "action");
  const changedPaths = Array.isArray(error.details.changed_paths)
    ? error.details.changed_paths.filter(
        (path): path is string => typeof path === "string",
      )
    : [];

  return (
    <div
      role="alert"
      className="space-y-2 rounded-md border border-[var(--aria-danger)]/30 bg-red-50 px-3 py-2 text-sm text-[var(--aria-danger)]"
    >
      <p className="font-semibold">{error.message}</p>
      {reason ? <p>{reason}</p> : null}
      <dl className="grid grid-cols-[7rem_minmax(0,1fr)] gap-x-2 gap-y-1 text-xs">
        {stage ? <DetailRow label="stage" value={stage} /> : null}
        {provider ? <DetailRow label="provider" value={provider} /> : null}
        {command ? <DetailRow label="command" value={command} /> : null}
        {reasonCode ? <DetailRow label="reason_code" value={reasonCode} /> : null}
        {stderrSummary ? (
          <DetailRow label="stderr_summary" value={stderrSummary} />
        ) : null}
        {typeof error.details.retryable === "boolean" ? (
          <DetailRow
            label="retryable"
            value={error.details.retryable ? "true" : "false"}
          />
        ) : null}
        {action ? <DetailRow label="action" value={action} /> : null}
      </dl>
      {changedPaths.length > 0 ? (
        <div className="space-y-1 text-xs">
          <div className="font-semibold">changed_paths</div>
          <ul className="list-disc pl-5 font-mono">
            {changedPaths.map((path) => (
              <li key={path}>{path}</li>
            ))}
          </ul>
          <p>目标代码库中的上述修改可能已保留，系统未执行破坏性回滚。</p>
        </div>
      ) : null}
      {error.details.retryable === true ? <p>修复问题后可以重新提交。</p> : null}
    </div>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt className="font-semibold">{label}</dt>
      <dd className="break-words">{value}</dd>
    </>
  );
}

function detailString(
  details: RepositoryRegistrationErrorDetails,
  key: string,
) {
  const value = details[key];
  return typeof value === "string" && value.length > 0 ? value : null;
}
