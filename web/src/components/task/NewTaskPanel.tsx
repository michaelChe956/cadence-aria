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
      className="border-b border-line bg-white p-3"
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
      <div className="grid grid-cols-[minmax(16rem,1fr)_13rem_10rem_10rem_8rem_auto] gap-2">
        <label className="text-xs font-semibold text-slate-500">
          任务请求
          <input
            aria-label="任务请求"
            className="mt-1 w-full rounded-md border border-line px-2 py-2 text-sm"
            value={requestText}
            onChange={(event) => setRequestText(event.target.value)}
          />
        </label>
        <label className="text-xs font-semibold text-slate-500">
          change id
          <input
            aria-label="change id"
            className="mt-1 w-full rounded-md border border-line px-2 py-2 text-sm"
            value={changeId}
            onChange={(event) => setChangeId(event.target.value)}
          />
        </label>
        <label className="text-xs font-semibold text-slate-500">
          policy preset
          <select
            aria-label="policy preset"
            className="mt-1 w-full rounded-md border border-line px-2 py-2 text-sm"
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
        <label className="text-xs font-semibold text-slate-500">
          provider mode
          <select
            aria-label="provider mode"
            className="mt-1 w-full rounded-md border border-line px-2 py-2 text-sm"
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
        <label className="text-xs font-semibold text-slate-500">
          timeout seconds
          <input
            aria-label="timeout seconds"
            type="number"
            min={1}
            className="mt-1 w-full rounded-md border border-line px-2 py-2 text-sm"
            value={timeoutSecs}
            onChange={(event) => setTimeoutSecs(Number(event.target.value))}
          />
        </label>
        <button
          type="submit"
          disabled={busy || requestText.trim() === "" || changeId.trim() === ""}
          className="mt-5 inline-flex items-center justify-center rounded-md bg-ink px-3 py-2 text-sm font-semibold text-white disabled:opacity-50"
        >
          <Plus className="mr-1 h-4 w-4" />
          新建任务
        </button>
      </div>
    </form>
  );
}
