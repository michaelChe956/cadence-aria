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
      className="rounded-lg border-2 border-indigo-200 bg-white p-4 shadow-[0_10px_0_rgba(79,70,229,0.10),0_18px_38px_rgba(79,70,229,0.16)]"
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
          <h2 className="text-lg font-black text-[#241B2F]">Workspace</h2>
          <p className="mt-1 text-sm font-semibold text-[#5E516B]">
            Describe the task, and Aria will split execution into inspectable nodes.
          </p>
        </div>
        <span className="rounded-lg border-2 border-cyan-200 bg-cyan-100 px-3 py-1 text-xs font-black text-cyan-950">
          guided prompt
        </span>
      </div>
      <div className="grid gap-4">
        <label className="text-sm font-black text-[#241B2F]">
          任务请求
          <textarea
            aria-label="任务请求"
            rows={4}
            placeholder="例如：实现 Fibonacci square sum，并解释每一步验证结果"
            className="mt-2 w-full resize-y rounded-lg border-2 border-indigo-200 bg-white px-4 py-3 text-base font-semibold leading-7 text-indigo-950 shadow-inner shadow-indigo-200/70 outline-none transition-colors placeholder:text-indigo-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
            value={requestText}
            onChange={(event) => setRequestText(event.target.value)}
          />
        </label>
        <fieldset
          aria-label="Execution settings"
          className="rounded-lg border-2 border-dashed border-indigo-200 bg-indigo-50/70 p-3"
        >
          <legend className="px-2 text-xs font-black uppercase text-[#8E2D60]">Execution settings</legend>
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 2xl:grid-cols-[minmax(12rem,1fr)_10rem_10rem_8rem_auto]">
          <label className="text-xs font-black text-indigo-800">
            change id
            <input
              aria-label="change id"
              className="mt-1 w-full rounded-lg border-2 border-indigo-100 bg-white px-3 py-2 text-sm font-semibold text-indigo-950 shadow-inner shadow-indigo-200/60 outline-none transition-colors placeholder:text-indigo-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
              value={changeId}
              onChange={(event) => setChangeId(event.target.value)}
            />
          </label>
          <label className="text-xs font-black text-indigo-800">
            policy preset
            <select
              aria-label="policy preset"
              className="mt-1 w-full rounded-lg border-2 border-indigo-100 bg-white px-3 py-2 text-sm font-semibold text-indigo-950 shadow-inner shadow-indigo-200/60 outline-none transition-colors focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
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
          <label className="text-xs font-black text-indigo-800">
            provider mode
            <select
              aria-label="provider mode"
              className="mt-1 w-full rounded-lg border-2 border-indigo-100 bg-white px-3 py-2 text-sm font-semibold text-indigo-950 shadow-inner shadow-indigo-200/60 outline-none transition-colors focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
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
          <label className="text-xs font-black text-indigo-800">
            timeout seconds
            <input
              aria-label="timeout seconds"
              type="number"
              min={1}
              className="mt-1 w-full rounded-lg border-2 border-indigo-100 bg-white px-3 py-2 text-sm font-semibold text-indigo-950 shadow-inner shadow-indigo-200/60 outline-none transition-colors focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
              value={timeoutSecs}
              onChange={(event) => setTimeoutSecs(Number(event.target.value))}
            />
          </label>
          <button
            type="submit"
            disabled={busy || requestText.trim() === "" || changeId.trim() === ""}
            className="inline-flex items-center justify-center self-end rounded-lg border-2 border-orange-500 bg-orange-300 px-4 py-2 text-sm font-black text-orange-950 shadow-[0_5px_0_rgba(251,146,60,0.36)] transition-colors hover:bg-orange-200 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
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
