import { useRef, useState, type FormEvent } from "react";

export type WorkItemPlanOptionsFormValue = {
  include_integration_tests: boolean;
  include_e2e_tests: boolean;
  force_frontend_backend_split: boolean;
  require_execution_plan_confirm: boolean;
};

export function WorkItemPlanOptionsDialog({
  defaultOptions,
  onConfirm,
  onClose,
}: {
  defaultOptions: WorkItemPlanOptionsFormValue;
  onConfirm: (options: WorkItemPlanOptionsFormValue) => Promise<void> | void;
  onClose: () => void;
}) {
  const [options, setOptions] =
    useState<WorkItemPlanOptionsFormValue>(defaultOptions);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submittingRef.current) {
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    setSubmitError(null);
    try {
      await onConfirm(options);
    } catch (reason) {
      setSubmitError(
        reason instanceof Error ? reason.message : "创建 Work Item Plan 失败",
      );
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  function updateOption(key: keyof WorkItemPlanOptionsFormValue) {
    setOptions((current) => ({
      ...current,
      [key]: !current[key],
    }));
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
        </div>

        {submitError ? (
          <p
            role="alert"
            className="mt-3 text-sm font-semibold text-[var(--aria-danger)]"
          >
            {submitError}
          </p>
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
            disabled={submitting}
            className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
          >
            创建并打开 Workspace
          </button>
        </div>
      </form>
    </div>
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
