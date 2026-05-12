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
        <Dialog.Overlay className="fixed inset-0 bg-black/70" />
        <Dialog.Content className="fixed left-1/2 top-1/2 w-[34rem] -translate-x-1/2 -translate-y-1/2 rounded-md border border-cyan-300/15 bg-[#0b1220] p-5 text-slate-100 shadow-xl shadow-cyan-950/60">
          <Dialog.Title className="text-lg font-semibold">回退到 checkpoint</Dialog.Title>
          <Dialog.Description className="mt-1 text-sm text-slate-500">
            确认本次回退会丢弃的运行记录、产物和文件变更。
          </Dialog.Description>
          {preview ? (
            <div className="mt-3 space-y-2 text-sm">
              <div>Checkpoint: {preview.checkpoint_id}</div>
              <div>Git head: {preview.git_head ?? "none"}</div>
              <div>Turns: {preview.turns_to_drop}</div>
              <div>Node runs: {preview.node_runs_to_drop}</div>
              <div>Provider runs: {preview.provider_runs_to_drop}</div>
              <div>Artifacts: {preview.artifacts_to_drop}</div>
              <div>Files: {preview.files_may_change.join(", ")}</div>
              {preview.dirty ? (
                <label className="flex items-center gap-2 rounded-md border border-danger/30 bg-red-50 p-2">
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
            <button type="button" className="rounded-md border border-white/10 px-3 py-2" onClick={() => onOpenChange(false)}>
              取消
            </button>
            <button
              type="button"
              className="rounded-md bg-cyan-300 px-3 py-2 text-slate-950 disabled:opacity-50"
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
