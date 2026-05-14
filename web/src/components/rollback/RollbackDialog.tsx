import * as Dialog from "@radix-ui/react-dialog";
import { useState } from "react";
import type { RollbackPreviewResponse } from "../../api/types";

export function RollbackDialog({
  open,
  preview,
  onConfirm,
  onOpenChange,
}: {
  open: boolean;
  preview: RollbackPreviewResponse | null;
  onConfirm: (payload: { checkpoint_id: string; force_when_dirty: boolean }) => void;
  onOpenChange: (open: boolean) => void;
}) {
  const [force, setForce] = useState(false);
  const disabled = !preview || (preview.dirty && !force);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-indigo-950/45 backdrop-blur-sm" />
        <Dialog.Content className="fixed left-1/2 top-1/2 w-[calc(100vw-2rem)] max-w-[34rem] -translate-x-1/2 -translate-y-1/2 rounded-lg border-2 border-indigo-200 bg-white p-5 text-indigo-950 shadow-[0_18px_0_rgba(79,70,229,0.18),0_28px_70px_rgba(30,27,75,0.32)]">
          <Dialog.Title className="text-lg font-bold">回退到 checkpoint</Dialog.Title>
          <Dialog.Description className="mt-1 text-sm font-semibold text-indigo-600">
            确认本次回退会丢弃的运行记录、产物和文件变更。
          </Dialog.Description>
          {preview ? (
            <div className="mt-3 space-y-2 text-sm">
              <PreviewLine label="Checkpoint" value={preview.checkpoint_id} />
              <PreviewLine label="Git head" value={preview.git_head ?? "none"} />
              <PreviewLine label="Turns" value={String(preview.turns_to_drop)} />
              <PreviewLine label="Node runs" value={String(preview.node_runs_to_drop)} />
              <PreviewLine label="Provider runs" value={String(preview.provider_runs_to_drop)} />
              <PreviewLine label="Artifacts" value={String(preview.artifacts_to_drop)} />
              <PreviewLine label="Files" value={preview.files_may_change.join(", ")} />
              {preview.dirty ? (
                <label className="flex items-center gap-2 rounded-lg border-2 border-rose-200 bg-rose-100 p-2 font-semibold text-rose-900">
                  <input
                    type="checkbox"
                    checked={force}
                    onChange={(event) => setForce(event.target.checked)}
                  />
                  允许丢弃当前未提交变更
                </label>
              ) : null}
            </div>
          ) : null}
          <div className="mt-4 flex justify-end gap-2">
            <button
              type="button"
              className="rounded-lg border-2 border-indigo-200 bg-white px-3 py-2 font-bold text-indigo-700 shadow-[0_4px_0_rgba(129,140,248,0.16)] transition-colors hover:bg-indigo-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-indigo-200"
              onClick={() => onOpenChange(false)}
            >
              取消
            </button>
            <button
              type="button"
              className="rounded-lg border-2 border-orange-600 bg-orange-500 px-3 py-2 font-bold text-white shadow-[0_5px_0_rgba(154,52,18,0.45)] transition-colors hover:bg-orange-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
              disabled={disabled}
              onClick={() =>
                preview &&
                onConfirm({
                  checkpoint_id: preview.checkpoint_id,
                  force_when_dirty: force,
                })
              }
            >
              执行回退
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function PreviewLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[8rem_minmax(0,1fr)] gap-3 rounded-lg border-2 border-indigo-100 bg-indigo-50/80 px-3 py-2">
      <span className="font-semibold text-indigo-600">{label}</span>
      <span className="break-words font-mono font-semibold text-indigo-950">{value || "none"}</span>
    </div>
  );
}
