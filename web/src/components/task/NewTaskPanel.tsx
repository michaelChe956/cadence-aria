import { Plus } from "lucide-react";
import { useState } from "react";
import type { CreateTaskRequest } from "../../api/types";

const policyPresets = ["manual-write", "manual-all", "auto-review", "non-interactive"];
const providerModes = ["fake", "real"];

export function NewTaskPanel({
  onCreateTask,
  busy,
}: {
  onCreateTask: (payload: CreateTaskRequest) => void | Promise<void>;
  busy: boolean;
}) {
  const [requestText, setRequestText] = useState("");
  const [changeId, setChangeId] = useState("");
  const [policyPreset, setPolicyPreset] = useState("manual-write");
  const [providerMode, setProviderMode] = useState("fake");
  const [timeoutSecs, setTimeoutSecs] = useState(2400);

  return (
    <form
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-sm"
      onSubmit={(event) => {
        event.preventDefault();
        void onCreateTask({
          request_text: requestText,
          change_id: changeId,
          policy_preset: policyPreset,
          provider_mode: providerMode,
          timeout_secs: timeoutSecs,
        });
      }}
    >
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold text-[var(--aria-ink)]">Workspace</h2>
          <p className="mt-1 text-sm font-medium text-[var(--aria-ink-muted)]">
            Describe the task, and Aria will split execution into inspectable nodes.
          </p>
        </div>
        <span className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink)]">
          guided prompt
        </span>
      </div>
      <div className="grid gap-4">
        <label className="text-sm font-semibold text-[var(--aria-ink)]">
          任务请求
          <textarea
            aria-label="任务请求"
            rows={4}
            placeholder="例如：实现 Fibonacci square sum，并解释每一步验证结果"
            className="mt-2 w-full resize-y rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 py-2 text-sm font-medium leading-6 text-[var(--aria-ink)] outline-none transition-colors placeholder:text-[var(--aria-ink-muted)] focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            value={requestText}
            onChange={(event) => setRequestText(event.target.value)}
          />
        </label>
        <fieldset
          aria-label="Execution settings"
          className="rounded-lg border border-dashed border-[var(--aria-line-strong)] bg-[var(--aria-panel-muted)] p-3"
        >
          <legend className="px-2 text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
            Execution settings
          </legend>
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 2xl:grid-cols-[minmax(12rem,1fr)_10rem_10rem_8rem_auto]">
            <label className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              change id
              <input
                aria-label="change id"
                className="mt-1 h-9 w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-sm font-medium text-[var(--aria-ink)] outline-none transition-colors placeholder:text-[var(--aria-ink-muted)] focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                value={changeId}
                onChange={(event) => setChangeId(event.target.value)}
              />
            </label>
            <label className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              policy preset
              <select
                aria-label="policy preset"
                className="mt-1 h-9 w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-sm font-medium text-[var(--aria-ink)] outline-none transition-colors focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                value={policyPreset}
                onChange={(event) => setPolicyPreset(event.target.value)}
              >
                {policyPresets.map((preset) => (
                  <option key={preset} value={preset}>
                    {preset}
                  </option>
                ))}
              </select>
            </label>
            <label className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              provider mode
              <select
                aria-label="provider mode"
                className="mt-1 h-9 w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-sm font-medium text-[var(--aria-ink)] outline-none transition-colors focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                value={providerMode}
                onChange={(event) => setProviderMode(event.target.value)}
              >
                {providerModes.map((mode) => (
                  <option key={mode} value={mode}>
                    {mode}
                  </option>
                ))}
              </select>
            </label>
            <label className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              timeout seconds
              <input
                aria-label="timeout seconds"
                type="number"
                min={1}
                className="mt-1 h-9 w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-sm font-medium text-[var(--aria-ink)] outline-none transition-colors focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                value={timeoutSecs}
                onChange={(event) => setTimeoutSecs(Number(event.target.value))}
              />
            </label>
            <button
              type="submit"
              disabled={busy || requestText.trim() === "" || changeId.trim() === ""}
              className="inline-flex h-9 items-center justify-center self-end rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              <Plus className="mr-1 h-4 w-4" />
              新建任务
            </button>
          </div>
        </fieldset>
      </div>
    </form>
  );
}
