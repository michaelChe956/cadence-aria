import { useRef, useState, type FormEvent } from "react";
import { ApiRequestError } from "../../api/client";
import type {
  CreateRepositoryRequest,
  CreateRepositoryResponse,
  RepositoryRegistrationErrorDetails,
  WorkspaceProviderName,
} from "../../api/types";
import { getProviderOptions } from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";

export function CreateRepositoryDialog({
  onCreate,
  onClose,
}: {
  onCreate: (
    payload: CreateRepositoryRequest,
  ) => Promise<CreateRepositoryResponse> | CreateRepositoryResponse;
  onClose: () => void;
}) {
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
  const [success, setSuccess] = useState<CreateRepositoryResponse | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const availabilitySnapshot = useProviderAvailabilityStore(
    (state) => state.snapshot,
  );
  const providerOptions = getProviderOptions(availabilitySnapshot);
  const visibleProviderOptions = providerOptions.filter(
    (option) => option.visible || option.value === providerMode,
  );
  const unavailableProviderOptions = visibleProviderOptions.filter(
    (option) => option.disabled,
  );
  const claudeCode = providerOptions.find(
    (option) => option.value === "claude_code",
  )!;

  function clearErrors() {
    setValidationError(null);
    setRegistrationError(null);
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
    try {
      const response = await onCreate({
        name: trimmedName,
        path: trimmedPath,
        default_policy_preset: policyPreset,
        default_provider_mode: providerMode,
      });
      setSuccess(response);
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

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <form
        role="dialog"
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
            disabled={submitting}
            className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
          >
            关闭
          </button>
        </div>
        {success ? (
          <RepositoryInitializationSuccess response={success} />
        ) : (
          <>
            <div className="space-y-3">
              <label className="block text-sm font-semibold text-[var(--aria-ink)]">
                代码库名称
                <input
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
                    setProviderMode(
                      event.target.value as WorkspaceProviderName,
                    );
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
                disabled={submitting}
                className="rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink-muted)]"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={submitting || !claudeCode.available}
                className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
              >
                添加代码库
              </button>
            </div>
          </>
        )}
        {success ? (
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white"
            >
              完成
            </button>
          </div>
        ) : null}
      </form>
    </div>
  );
}

function RepositoryInitializationSuccess({
  response,
}: {
  response: CreateRepositoryResponse;
}) {
  const { initialization } = response;
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
