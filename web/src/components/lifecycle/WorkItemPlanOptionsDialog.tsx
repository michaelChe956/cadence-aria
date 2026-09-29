import { useEffect, useRef, useState, type FormEvent } from "react";
import { ApiRequestError, getAutomationTarget } from "../../api/client";
import type {
  AutomationTarget,
  EnrollmentTarget,
} from "../../api/types";
import type { WorkspaceProviderName } from "../../api/types";
import {
  getProviderOptions,
  providerOptionsForValue,
  workspaceProviderName,
  type ProviderOption,
} from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";

export type AutomationMode = "manual" | "automatic";

export type WorkItemPlanOptionsFormValue = {
  include_integration_tests: boolean;
  include_e2e_tests: boolean;
  force_frontend_backend_split: boolean;
  require_execution_plan_confirm: boolean;
  /** REQ-PPS-01：创建请求携带的 author provider 快照；缺省表示沿用服务端兼容默认。 */
  author_provider?: WorkspaceProviderName;
  /** REQ-PPS-01：创建请求携带的 reviewer provider 快照；缺省表示沿用服务端兼容默认。 */
  reviewer_provider?: WorkspaceProviderName;
  /** P1 WIGA：缺省「不自动化」；仅在已确认 Design 上可选 automatic。 */
  automation_mode: AutomationMode;
};

type WorkItemPlanBooleanOptionKey =
  | "include_integration_tests"
  | "include_e2e_tests"
  | "force_frontend_backend_split"
  | "require_execution_plan_confirm";

export function WorkItemPlanOptionsDialog({
  defaultOptions,
  automationAvailable,
  automationTargetScope = null,
  onConfirm,
  onClose,
}: {
  defaultOptions: WorkItemPlanOptionsFormValue;
  /** P1 WIGA：仅已确认 Design 显示自动化模式选择（缺省手动，人工链路不变）。 */
  automationAvailable: boolean;
  /** C5 Task 7：automatic 模式的 automation-target 投影身份（project+issue）；
   * manual 模式不请求 target，缺省 null 时 automatic 提交 fail-closed。 */
  automationTargetScope?: { projectId: string; issueId: string } | null;
  onConfirm: (options: WorkItemPlanOptionsFormValue) => Promise<void> | void;
  onClose: () => void;
}) {
  const [options, setOptions] =
    useState<WorkItemPlanOptionsFormValue>(defaultOptions);
  const [submitError, setSubmitError] = useState<SubmitErrorState | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const [targetProjection, setTargetProjection] =
    useState<AutomationTargetProjectionState>({ status: "idle" });
  const [targetRetryTick, setTargetRetryTick] = useState(0);
  const providerSnapshot = useProviderAvailabilityStore(
    (state) => state.snapshot,
  );
  const providerOptions = getProviderOptions(providerSnapshot);

  const automatic = options.automation_mode === "automatic";
  const targetScope = automationAvailable && automatic ? automationTargetScope : null;
  // C5 Task 7：automatic 提交必须有就绪的 target 投影——loading/GET 失败/
  // 身份（scope）缺失都禁止 stale automatic submit；manual 分支不受影响。
  const automaticSubmitBlocked =
    automatic &&
    automationAvailable &&
    (targetScope === null || targetProjection.status !== "ready");

  // C5 Task 7：打开自动模式先取 automation-target 投影（无 query：身份
  // 投影与 provider options 无关，展示 target.kind 与双级身份）；scope
  // 身份变化（project/issue 漂移）即重取，旧投影不落地。manual 模式不请求。
  useEffect(() => {
    if (targetScope === null) {
      setTargetProjection({ status: "idle" });
      return;
    }
    let alive = true;
    setTargetProjection({ status: "loading" });
    getAutomationTarget(targetScope.projectId, targetScope.issueId, {})
      .then((target: AutomationTarget) => {
        if (alive) {
          setTargetProjection({ status: "ready", target });
        }
      })
      .catch((reason: unknown) => {
        if (alive) {
          setTargetProjection({
            status: "error",
            message:
              reason instanceof Error ? reason.message : "自动化目标读取失败",
          });
        }
      });
    return () => {
      alive = false;
    };
  }, [
    targetScope?.projectId,
    targetScope?.issueId,
    targetRetryTick,
    targetScope === null,
  ]);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submittingRef.current || automaticSubmitBlocked) {
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    setSubmitError(null);
    try {
      await onConfirm(options);
    } catch (reason) {
      // C5 Task 3：role-chain 预检错误保留 ApiRequestError.code/details，
      // 逐角色渲染 role/provider/reason；不包装为普通 Error 语义。
      setSubmitError({
        message:
          reason instanceof Error ? reason.message : "创建 Work Item Plan 失败",
        roleChainViolations: roleChainViolationsOf(reason),
      });
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  function updateOption(key: WorkItemPlanBooleanOptionKey) {
    setOptions((current) => ({
      ...current,
      [key]: !current[key],
    }));
    setSubmitError(null);
  }

  // 空字符串是「沿用服务端默认」占位项：收窄失败即落 undefined，请求体不落键。
  function updateProvider(
    key: "author_provider" | "reviewer_provider",
    value: string,
  ) {
    const provider = workspaceProviderName(value) ?? undefined;
    setOptions((current) => ({ ...current, [key]: provider }));
    setSubmitError(null);
  }

  // P1 WIGA：模式切换不落 enrollment，只改本地表单；提交才走 PUT。
  function updateAutomationMode(mode: AutomationMode) {
    setOptions((current) => ({ ...current, automation_mode: mode }));
    setSubmitError(null);
  }

  return (
    <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/35 p-4">
      <form
        role="dialog"
        aria-label="Work Item Plan 配置"
        aria-modal="true"
        onSubmit={handleSubmit}
        className="w-full max-w-lg rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">
            Work Item Plan 配置
          </h2>
          <button
            type="button"
            disabled={submitting}
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)] disabled:opacity-60"
          >
            关闭
          </button>
        </div>

        <div className="space-y-3">
          <OptionCheckbox
            label="包含贯通/集成测试 Work Item"
            checked={options.include_integration_tests}
            disabled={submitting}
            onChange={() => updateOption("include_integration_tests")}
          />
          <OptionCheckbox
            label="包含 E2E 测试 Work Item"
            checked={options.include_e2e_tests}
            disabled={submitting}
            onChange={() => updateOption("include_e2e_tests")}
          />
          <OptionCheckbox
            label="强制前后端拆分"
            checked={options.force_frontend_backend_split}
            disabled={submitting}
            onChange={() => updateOption("force_frontend_backend_split")}
          />
          <OptionCheckbox
            label="子 Work Item 执行前需要确认 Plan"
            checked={options.require_execution_plan_confirm}
            disabled={submitting}
            onChange={() => updateOption("require_execution_plan_confirm")}
          />
          {/* REQ-PPS-01：创建即快照 provider，不再依赖创建后页面补发选择；
              不可用项置灰禁选，缺省项沿用服务端兼容默认。 */}
          <ProviderSelect
            label="Author Provider"
            value={options.author_provider}
            options={providerOptions}
            disabled={submitting}
            onChange={(value) => updateProvider("author_provider", value)}
          />
          <ProviderSelect
            label="Reviewer Provider"
            value={options.reviewer_provider}
            options={providerOptions}
            disabled={submitting}
            onChange={(value) => updateProvider("reviewer_provider", value)}
          />
          {/* P1 WIGA：仅已确认 Design 提供显式自动化选择；缺省手动，人工链路原样。 */}
          {automationAvailable ? (
            <fieldset className="rounded-md border border-[var(--aria-line)] px-3 py-2">
              <legend className="px-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
                生成模式
              </legend>
              <div className="flex gap-4">
                <label className="flex items-center gap-1.5 text-sm text-[var(--aria-ink)]">
                  <input
                    type="radio"
                    name="automation-mode"
                    checked={!automatic}
                    disabled={submitting}
                    onChange={() => updateAutomationMode("manual")}
                  />
                  手动
                </label>
                <label className="flex items-center gap-1.5 text-sm text-[var(--aria-ink)]">
                  <input
                    type="radio"
                    name="automation-mode"
                    checked={automatic}
                    disabled={submitting}
                    onChange={() => updateAutomationMode("automatic")}
                  />
                  自动化
                </label>
              </div>
            </fieldset>
          ) : null}
        {automatic ? (
          <div className="mt-1">
            {targetProjection.status === "loading" ? (
              <p role="status" className="text-xs text-[var(--aria-ink-muted)]">
                自动化目标加载中…（提交暂不可用）
              </p>
            ) : null}
            {targetProjection.status === "error" ? (
              <div>
                <p
                  role="alert"
                  className="text-xs text-[var(--aria-danger)]"
                >
                  自动化目标加载失败：{targetProjection.message}
                </p>
                <button
                  type="button"
                  data-testid="automation-target-retry"
                  disabled={submitting}
                  onClick={() => setTargetRetryTick((tick) => tick + 1)}
                  className="mt-1 rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
                >
                  重试
                </button>
              </div>
            ) : null}
            {targetProjection.status === "ready" ? (
              <p
                data-testid="automation-target-projection"
                className="text-xs text-[var(--aria-ink-muted)]"
              >
                自动化目标：
                {enrollmentTargetLabel(targetProjection.target.enrollment_target)}
              </p>
            ) : null}
            {automationTargetScope === null ? (
              <p role="alert" className="text-xs text-[var(--aria-danger)]">
                自动化目标身份未知，缺少 project/issue；请刷新后重试
              </p>
            ) : null}
          </div>
        ) : null}
        </div>

        {submitError ? (
          <div
            role="alert"
            className="mt-3 text-sm font-semibold text-[var(--aria-danger)]"
          >
            <p>{submitError.message}</p>
            {submitError.roleChainViolations ? (
              <>
                <ul className="mt-1 list-disc pl-5 text-xs font-normal">
                  {submitError.roleChainViolations.map((violation) => (
                    <li
                      key={`${violation.role}:${violation.provider}:${violation.reasonCode}`}
                      data-testid="role-chain-violation"
                    >
                      {violation.role} · {violation.provider} ·{" "}
                      {violation.reasonCode}
                    </li>
                  ))}
                </ul>
                <p className="mt-1 text-xs font-normal">
                  请更换 provider 配置或更换目标后重试。
                </p>
              </>
            ) : null}
          </div>
        ) : null}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            disabled={submitting}
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink-muted)] disabled:opacity-60"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={submitting || automaticSubmitBlocked}
            className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
          >
            {automatic ? "启用自动化" : "创建并打开 Workspace"}
          </button>
        </div>
      </form>
    </div>
  );
}

type AutomationTargetProjectionState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; target: AutomationTarget };

type SubmitErrorState = {
  message: string;
  /** C5 Task 3：automation_role_chain_unsupported 的逐角色违规
   *（ApiRequestError.details 原样派生，非该错误时为 null）。 */
  roleChainViolations: RoleChainViolation[] | null;
};

type RoleChainViolation = {
  role: string;
  provider: string;
  reasonCode: string;
};

/** C5 Task 3：role ∈ plan_author/coder/plan_reviewer/code_reviewer/
 * internal_reviewer；details.violations 逐条提取，缺失/非字符串字段剔除。 */
function roleChainViolationsOf(reason: unknown): RoleChainViolation[] | null {
  if (
    !(reason instanceof ApiRequestError) ||
    reason.code !== "automation_role_chain_unsupported"
  ) {
    return null;
  }
  const violations = reason.details.violations;
  if (!Array.isArray(violations)) {
    return null;
  }
  const parsed: RoleChainViolation[] = [];
  for (const entry of violations) {
    if (typeof entry !== "object" || entry === null) {
      continue;
    }
    const candidate = entry as Record<string, unknown>;
    if (
      typeof candidate.role === "string" &&
      typeof candidate.provider === "string" &&
      typeof candidate.reason_code === "string"
    ) {
      parsed.push({
        role: candidate.role,
        provider: candidate.provider,
        reasonCode: candidate.reason_code,
      });
    }
  }
  return parsed.length > 0 ? parsed : null;
}

/** C5 Task 1：双载体 enrollment target 的展示身份（单仓=物理仓，
 * LC=双级身份）。 */
function enrollmentTargetLabel(target: EnrollmentTarget): string {
  return target.kind === "single_repository"
    ? `single_repository ${target.repository_id}`
    : `logical_codebase ${target.logical_codebase_id}/${target.logical_repository_id}`;
}

function ProviderSelect({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: WorkspaceProviderName | undefined;
  options: ProviderOption[];
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex items-center gap-3 rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
      <span className="w-32 shrink-0 text-[var(--aria-ink-muted)]">{label}</span>
      <select
        aria-label={label}
        value={value ?? ""}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        className="min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm text-[var(--aria-ink)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
      >
        <option value="">（沿用服务端默认）</option>
        {providerOptionsForValue(options, value).map((option) => (
          <option
            key={option.value}
            value={option.value}
            disabled={option.disabled}
          >
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}

function OptionCheckbox({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: () => void;
}) {
  return (
    <label className="flex items-start gap-3 rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={onChange}
        className="mt-0.5 h-4 w-4 rounded border-[var(--aria-line)]"
      />
      <span>{label}</span>
    </label>
  );
}
