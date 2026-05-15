import { CheckCircle2, OctagonAlert, PencilLine } from "lucide-react";

export type GateActionBarProps = {
  gate: {
    gate_id: string;
    node_id: string;
    status: string;
  };
  onConfirm: () => void;
  onRequestChange: () => void;
  onTerminate: () => void;
};

export function GateActionBar({
  gate,
  onConfirm,
  onRequestChange,
  onTerminate,
}: GateActionBarProps) {
  return (
    <section
      role="region"
      aria-label="Gate action bar"
      className="rounded-lg border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] p-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[11px] font-semibold uppercase text-[var(--aria-warning)]">
            gate checkpoint
          </p>
          <div className="mt-1 flex flex-wrap gap-2 font-mono text-xs font-medium text-[var(--aria-ink)]">
            <span>{gate.node_id}</span>
            <span>{gate.status}</span>
            <span>{gate.gate_id}</span>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={onConfirm}
            className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white transition-colors hover:bg-cyan-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <CheckCircle2 className="mr-1 h-4 w-4" />
            确认继续
          </button>
          <button
            type="button"
            onClick={onRequestChange}
            className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-warning)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-warning)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-warning)]"
          >
            <PencilLine className="mr-1 h-4 w-4" />
            要求修改
          </button>
          <button
            type="button"
            onClick={onTerminate}
            className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-danger)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-danger)] transition-colors hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)]"
          >
            <OctagonAlert className="mr-1 h-4 w-4" />
            终止
          </button>
        </div>
      </div>
    </section>
  );
}
