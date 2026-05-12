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
      className="rounded-xl border border-cyan-300/15 bg-white/[0.03] p-4 shadow-[0_0_35px_rgba(34,211,238,0.08)]"
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
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-lg font-semibold text-slate-100">Workspace</h2>
        <span className="rounded-full border border-cyan-300/20 bg-cyan-300/10 px-3 py-1 text-xs text-cyan-100">
          task input
        </span>
      </div>
      <div className="grid gap-3">
        <label className="text-xs font-semibold text-slate-400">
          任务请求
          <input
            aria-label="任务请求"
            className="mt-1 w-full rounded-md border border-white/10 bg-black/35 px-2 py-2 text-sm text-slate-100 outline-none focus:border-cyan-300"
            value={requestText}
            onChange={(event) => setRequestText(event.target.value)}
          />
        </label>
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 2xl:grid-cols-[minmax(12rem,1fr)_10rem_10rem_8rem_auto]">
          <label className="text-xs font-semibold text-slate-400">
            change id
            <input
              aria-label="change id"
              className="mt-1 w-full rounded-md border border-white/10 bg-black/35 px-2 py-2 text-sm text-slate-100 outline-none focus:border-cyan-300"
              value={changeId}
              onChange={(event) => setChangeId(event.target.value)}
            />
          </label>
          <label className="text-xs font-semibold text-slate-400">
            policy preset
            <select
              aria-label="policy preset"
              className="mt-1 w-full rounded-md border border-white/10 bg-black/35 px-2 py-2 text-sm text-slate-100 outline-none focus:border-cyan-300"
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
          <label className="text-xs font-semibold text-slate-400">
            provider mode
            <select
              aria-label="provider mode"
              className="mt-1 w-full rounded-md border border-white/10 bg-black/35 px-2 py-2 text-sm text-slate-100 outline-none focus:border-cyan-300"
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
          <label className="text-xs font-semibold text-slate-400">
            timeout seconds
            <input
              aria-label="timeout seconds"
              type="number"
              min={1}
              className="mt-1 w-full rounded-md border border-white/10 bg-black/35 px-2 py-2 text-sm text-slate-100 outline-none focus:border-cyan-300"
              value={timeoutSecs}
              onChange={(event) => setTimeoutSecs(Number(event.target.value))}
            />
          </label>
          <button
            type="submit"
            disabled={busy || requestText.trim() === "" || changeId.trim() === ""}
            className="self-end inline-flex items-center justify-center rounded-md bg-cyan-300 px-3 py-2 text-sm font-semibold text-slate-950 shadow-lg shadow-cyan-500/20 disabled:opacity-50"
          >
            <Plus className="mr-1 h-4 w-4" />
            新建任务
          </button>
        </div>
      </div>
    </form>
  );
}
