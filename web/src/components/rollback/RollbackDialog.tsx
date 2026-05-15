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
        <Dialog.Overlay className="fixed inset-0 bg-slate-950/35 backdrop-blur-sm" />
        <Dialog.Content className="fixed left-1/2 top-1/2 w-[calc(100vw-2rem)] max-w-[34rem] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-5 text-[var(--aria-ink)] shadow-lg">
          <Dialog.Title className="text-lg font-semibold">回退到 checkpoint</Dialog.Title>
          <Dialog.Description className="mt-1 text-sm font-medium text-[var(--aria-ink-muted)]">
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
                <label className="flex items-center gap-2 rounded-md border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] p-2 font-semibold text-[var(--aria-danger)]">
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
              className="inline-flex h-9 items-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              onClick={() => onOpenChange(false)}
            >
              取消
            </button>
            <button
              type="button"
              className="inline-flex h-9 items-center rounded-md border border-red-600 bg-red-600 px-3 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
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
    <div className="grid grid-cols-[8rem_minmax(0,1fr)] gap-3 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2">
      <span className="font-semibold text-[var(--aria-ink-muted)]">{label}</span>
      <span className="break-words font-mono font-medium text-[var(--aria-ink)]">{value || "none"}</span>
    </div>
  );
}
