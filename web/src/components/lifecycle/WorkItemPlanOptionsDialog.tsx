import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  ApiRequestError,
  getAutomationEnrollment,
  getAutomationTarget,
} from "../../api/client";
import type {
  AutomationTarget,
  EnrollmentTarget,
  IssueAutomationEnrollment,
} from "../../api/types";
import type { WorkspaceProviderName } from "../../api/types";
import {
  getProviderOptions,
  providerOptionsForValue,
  workspaceProviderName,
  type ProviderOption,
} from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import {
  enrollmentTargetMemberCountText,
  enrollmentTargetPlaceholder,
} from "../../state/enrollment-target-labels";
import {
  carrierTargetFromEnrollmentTarget,
  LEGACY_CARRIER_BLOCKED_MESSAGE,
  sameProviderChoice,
} from "./automation-reenable";
import { DialogShell } from "../shared/DialogShell";

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
  const [enrollmentProjection, setEnrollmentProjection] =
    useState<AutomationEnrollmentProjectionState>({ status: "idle" });
  const [targetRetryTick, setTargetRetryTick] = useState(0);
  const providerSnapshot = useProviderAvailabilityStore(
    (state) => state.snapshot,
  );
  const providerOptions = getProviderOptions(providerSnapshot);

  const automatic = options.automation_mode === "automatic";
  const targetScope = automationAvailable && automatic ? automationTargetScope : null;
  // 批 2 T4（REQ-WIGA-09 修订）：已终止的 enrollment 上，本次提交是「重新
  // 开启」——payload 原样回传终止记录（不顺便改写授权），因此不依赖新的
  // target 投影：入口不被终止态阻断，表单里的参数改动也不生效（置灰）。
  const terminatedEnrollment =
    enrollmentProjection.status === "ready" &&
    enrollmentProjection.enrollment?.terminated_at
      ? enrollmentProjection.enrollment
      : null;
  // REQ-WIGA-VOID-01：作废态优先于终止态呈现（连组一起没了，更贴近 owner
  // 心智）；两态可区分——作废态显示作废文案、不冒充终止态，终止态文案
  // 逐字保持原样。作废记录的重新开启同样不被阻断（后端放行全部漂移）。
  const voidedEnrollment =
    enrollmentProjection.status === "ready" &&
    enrollmentProjection.enrollment?.voided_at
      ? enrollmentProjection.enrollment
      : null;
  const reenableRecord = voidedEnrollment ?? terminatedEnrollment;
  const reenablePending = automatic && reenableRecord !== null;
  // 重新开启时「实际会用的 provider」：当前生效＝enrollment 的生效 options
  // （不是表单里未生效的值）；本次将应用＝表单本次选择（未选＝沿用原授权）。
  // 两者不一致＝本次会换 provider（一次提交完成换代，会先删除现有工作项组）。
  // REQ-WIGA-VOID-01：作废态不展示该对比——旧授权已随工作项组删除作废、
  // 无「当前生效」可言（作废文案已说明重新开启可自由选择）。
  const effectiveProviders =
    terminatedEnrollment && voidedEnrollment === null
      ? {
          author_provider: terminatedEnrollment.options?.author_provider,
          reviewer_provider: terminatedEnrollment.options?.reviewer_provider,
        }
      : null;
  const appliedProviders = effectiveProviders
    ? {
        author_provider:
          options.author_provider ?? effectiveProviders.author_provider,
        reviewer_provider:
          options.reviewer_provider ?? effectiveProviders.reviewer_provider,
      }
    : null;
  const providerChangePending =
    effectiveProviders !== null &&
    appliedProviders !== null &&
    !sameProviderChoice(effectiveProviders, appliedProviders);
  // C5 Task 7：automatic 提交必须有就绪的 target 投影——loading/GET 失败/
  // 身份（scope）缺失都禁止 stale automatic submit；manual 分支不受影响。
  // 重新开启（terminated）例外：payload 取终止记录，投影只用于展示。
  //
  // 发起侧载体收窄（方案 A，REQ-ROUTE-C5-TARGET 前端侧）：命令侧只接受
  // 逻辑代码库两变体。**legacy 单仓载体只读展示，但不得再发起命令**——
  // 禁用提交并给出提示（不静默降级、不猜一个 LC）。目标取「本次提交实际会
  // 用的那个 target」：重新开启＝终止记录的 target（缺失才回退新投影），
  // 常规启用＝新投影的 target——与提交链路同一口径。
  const enrollmentTargetForSubmit: EnrollmentTarget | null = reenablePending
    ? (reenableRecord?.target ??
      (targetProjection.status === "ready"
        ? targetProjection.target.enrollment_target
        : null))
    : targetProjection.status === "ready"
      ? targetProjection.target.enrollment_target
      : null;
  const submitCarrierTarget =
    enrollmentTargetForSubmit === null
      ? null
      : carrierTargetFromEnrollmentTarget(enrollmentTargetForSubmit);
  const legacyCarrierBlocked =
    enrollmentTargetForSubmit !== null && submitCarrierTarget === null;
  const automaticSubmitBlocked =
    automatic &&
    automationAvailable &&
    ((!reenablePending &&
      (targetScope === null || targetProjection.status !== "ready")) ||
      legacyCarrierBlocked);

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

  // 批 2 T4（REQ-WIGA-09 修订）：自动模式补读当前 enrollment 投影——只读
  // 展示，用于告知用户该 issue 的自动化已被终止（人工点「启用自动化」即
  // 显式重新开启）。读取失败不阻塞提交：提交链路自带权威 GET 与错误呈现，
  // 此处投影缺失只是少一句说明，不猜状态、不阻断入口。manual 模式不请求。
  useEffect(() => {
    if (targetScope === null) {
      setEnrollmentProjection({ status: "idle" });
      return;
    }
    let alive = true;
    setEnrollmentProjection({ status: "loading" });
    getAutomationEnrollment(targetScope.projectId, targetScope.issueId)
      .then((enrollment: IssueAutomationEnrollment | null) => {
        if (alive) {
          setEnrollmentProjection({ status: "ready", enrollment });
        }
      })
      .catch(() => {
        if (alive) {
          setEnrollmentProjection({ status: "error" });
        }
      });
    return () => {
      alive = false;
    };
  }, [targetScope?.projectId, targetScope?.issueId, targetScope === null]);

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
        code: reason instanceof ApiRequestError ? reason.code : null,
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
    <DialogShell
      label="Work Item Plan 配置"
      onClose={onClose}
      closeDisabled={submitting}
      layerClass="z-[80]"
    >
      <form onSubmit={handleSubmit}>
        <div className="space-y-3">
          <OptionCheckbox
            label="包含贯通/集成测试 Work Item"
            checked={options.include_integration_tests}
            disabled={submitting || reenablePending}
            onChange={() => updateOption("include_integration_tests")}
          />
          <OptionCheckbox
            label="包含 E2E 测试 Work Item"
            checked={options.include_e2e_tests}
            disabled={submitting || reenablePending}
            onChange={() => updateOption("include_e2e_tests")}
          />
          <OptionCheckbox
            label="强制前后端拆分"
            checked={options.force_frontend_backend_split}
            disabled={submitting || reenablePending}
            onChange={() => updateOption("force_frontend_backend_split")}
          />
          <OptionCheckbox
            label="子 Work Item 执行前需要确认 Plan"
            checked={!automatic && options.require_execution_plan_confirm}
            disabled={submitting || automatic}
            onChange={() => updateOption("require_execution_plan_confirm")}
          />
          {/* B2-P1 F2（REQ-WIGA-02）：automatic 下后端冻结一律压 false，
              勾选无效——置灰＋固定关闭显示并说明；不篡改表单值，切回
              手动即恢复用户原勾选（manual 行为不变）。 */}
          {automatic ? (
            <p
              data-testid="plan-approval-auto-note"
              className="text-xs font-normal text-[var(--aria-ink-muted)]"
            >
              自动化模式下无需执行前确认，plan 自动批准
            </p>
          ) : null}
          {/* REQ-PPS-01：创建即快照 provider，不再依赖创建后页面补发选择；
              不可用项置灰禁选，缺省项沿用服务端兼容默认。
              重新开启（terminated）时 provider 选择器保持可用——换 provider 是
              契约允许的（一次提交完成换代：先删除现有工作项组再按新 provider
              重建），本次选择会真正生效（下方如实展示当前生效与本次将应用的
              取值）。 */}
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
            {/* 重新开启（terminated）：目标取终止记录（新投影用不上，也不
                呈现其 loading/失败——重新开启不依赖新目标投影）。 */}
            {reenablePending ? (
              <p
                data-testid="automation-target-projection"
                className="text-xs text-[var(--aria-ink-muted)]"
              >
                自动化目标：
                {reenableRecord?.target
                  ? enrollmentTargetLabel(reenableRecord.target)
                  : "—"}
              </p>
            ) : (
              <>
                {targetProjection.status === "loading" ? (
                  <p
                    role="status"
                    className="text-xs text-[var(--aria-ink-muted)]"
                  >
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
                    {enrollmentTargetLabel(
                      targetProjection.target.enrollment_target,
                    )}
                  </p>
                ) : null}
              </>
            )}
            {/* 批 2 T4（REQ-WIGA-09 修订）：terminated 态入口仍可用——人工
                点「启用自动化」即显式重新开启（生成新的自动化授权）；此处
                只说明事实，不阻断提交，也不代替服务端的最终判定。
                REQ-WIGA-VOID-01：作废态优先呈现（两态文案可区分）。 */}
            {reenableRecord ? (
              voidedEnrollment ? (
                <p
                  role="status"
                  data-testid="automation-voided-notice"
                  className="text-xs text-[var(--aria-ink-muted)]"
                >
                  该 issue 的自动化登记已随旧工作项组删除作废；确认后将重新开启
                  （可自由选择 provider，原自动化不恢复）。
                </p>
              ) : (
                <p
                  role="status"
                  data-testid="automation-terminated-notice"
                  className="text-xs text-[var(--aria-ink-muted)]"
                >
                  该 issue 的自动化已于 {terminatedEnrollment.terminated_at} 终止；
                  确认「启用自动化」将重新开启（生成新的自动化授权，原自动化不恢复）。
                </p>
              )
            ) : null}
            {/* 发起侧载体收窄（方案 A）：legacy 单仓载体仍只读展示，但不得
                再发起 Enable/Rebind——禁用提交并说明下一步。 */}
            {legacyCarrierBlocked ? (
              <p
                role="alert"
                data-testid="automation-legacy-carrier-notice"
                className="text-xs text-[var(--aria-danger)]"
              >
                {LEGACY_CARRIER_BLOCKED_MESSAGE}
              </p>
            ) : null}
            {/* 「实际会用的 provider」如实取自 enrollment 的生效 options——不取
                表单里未生效的值，杜绝静默陷阱；本次改了 provider 时同时展示
                本次将应用的取值（换 provider 会先删除现有工作项组，其余参数
                按原授权）。 */}
            {effectiveProviders && appliedProviders ? (
              <p
                role="status"
                data-testid="automation-effective-providers"
                className="text-xs text-[var(--aria-ink-muted)]"
              >
                当前生效的 provider：
                {`Author ${effectiveProviders.author_provider ?? "—"}、Reviewer ${effectiveProviders.reviewer_provider ?? "—"}`}
                {providerChangePending
                  ? `；本次将应用：Author ${appliedProviders.author_provider ?? "—"}、Reviewer ${appliedProviders.reviewer_provider ?? "—"}（换 provider 会先删除现有工作项组，其余参数按原授权）`
                  : "；本次按原授权重新开启（其余参数不变）。"}
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
            <p data-testid="submit-error-message">{submitError.message}</p>
            {/* REQ-WIGA-SWAP-02：机器码只作次要信息（弱化小字），MUST NOT
                充当主文案。 */}
            {submitError.code ? (
              <p
                data-testid="submit-error-code"
                className="mt-1 text-xs font-normal text-[var(--aria-ink-muted)]"
              >
                机器码：{submitError.code}
              </p>
            ) : null}
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
    </DialogShell>
  );
}

type AutomationTargetProjectionState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; target: AutomationTarget };

/** 批 2 T4：当前 enrollment 只读投影（terminated 事实展示用）。读取失败
 * 不阻断提交——提交链路自带权威 GET 与错误呈现。 */
type AutomationEnrollmentProjectionState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "error" }
  | { status: "ready"; enrollment: IssueAutomationEnrollment | null };

type SubmitErrorState = {
  message: string;
  /** REQ-WIGA-SWAP-02：机器码（ApiRequestError.code）只作次要信息展示；
   * 非 ApiRequestError（已包装成中文主文案）时为 null。 */
  code: string | null;
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
 * LC=双级身份）；多成员 LC 展示成员数量。任何分支都不得输出
 * undefined/null/[object Object]，缺字段用「—」占位。
 * ⚠ 三变体全覆盖：`single_repository` 分支**仅 legacy 只读**（老单仓记录
 * 仍要显示，不退化为「target 未知」）；新投影不再产出它，命令侧也不接受
 * （见 `carrierTargetFromEnrollmentTarget`）。 */
function enrollmentTargetLabel(target: EnrollmentTarget): string {
  switch (target.kind) {
    // legacy 只读：仅供老单仓 enrollment 记录展示。
    case "single_repository":
      return `single_repository ${enrollmentTargetPlaceholder(target.repository_id)}`;
    case "logical_codebase":
      return `logical_codebase ${enrollmentTargetPlaceholder(target.logical_codebase_id)}/${enrollmentTargetPlaceholder(target.logical_repository_id)}`;
    case "logical_codebase_members":
      return `logical_codebase ${enrollmentTargetPlaceholder(target.logical_codebase_id)}[${enrollmentTargetMemberCountText(target.logical_repository_ids)}]`;
    default:
      return "—";
  }
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
