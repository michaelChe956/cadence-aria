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
      className="rounded-lg border-2 border-amber-200 bg-amber-50 p-3 shadow-[0_6px_0_rgba(217,119,6,0.14)]"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="text-xs font-black uppercase text-amber-900">
            gate checkpoint
          </p>
          <div className="mt-1 flex flex-wrap gap-2 font-mono text-xs font-bold text-slate-700">
            <span>{gate.node_id}</span>
            <span>{gate.status}</span>
            <span>{gate.gate_id}</span>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={onConfirm}
            className="inline-flex items-center justify-center rounded-lg border-2 border-emerald-700 bg-emerald-600 px-3 py-2 text-sm font-black text-white shadow-[0_4px_0_rgba(4,120,87,0.35)] transition-colors hover:bg-emerald-500 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-emerald-200"
          >
            <CheckCircle2 className="mr-1 h-4 w-4" />
            确认继续
          </button>
          <button
            type="button"
            onClick={onRequestChange}
            className="inline-flex items-center justify-center rounded-lg border-2 border-sky-700 bg-sky-600 px-3 py-2 text-sm font-black text-white shadow-[0_4px_0_rgba(3,105,161,0.30)] transition-colors hover:bg-sky-500 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-sky-200"
          >
            <PencilLine className="mr-1 h-4 w-4" />
            要求修改
          </button>
          <button
            type="button"
            onClick={onTerminate}
            className="inline-flex items-center justify-center rounded-lg border-2 border-rose-700 bg-rose-600 px-3 py-2 text-sm font-black text-white shadow-[0_4px_0_rgba(190,18,60,0.30)] transition-colors hover:bg-rose-500 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-rose-200"
          >
            <OctagonAlert className="mr-1 h-4 w-4" />
            终止
          </button>
        </div>
      </div>
    </section>
  );
}
