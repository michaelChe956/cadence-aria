import { CheckCircle2, Circle, CircleDot } from "lucide-react";

const PHASES = [
  { id: "clarification", label: "澄清" },
  { id: "development", label: "开发" },
  { id: "acceptance", label: "验收" },
] as const;

export function IssuePhaseRail({ status }: { status: string | null }) {
  const activeIndex = status === "completed" ? 2 : status === "draft" ? 0 : 1;

  return (
    <nav
      aria-label="Issue 阶段"
      className="rounded-lg border-2 border-slate-200 bg-white p-4 shadow-[0_8px_0_rgba(15,23,42,0.06),0_18px_34px_rgba(15,23,42,0.08)]"
    >
      <div className="mb-3">
        <h2 className="text-lg font-black text-[#241B2F]">阶段</h2>
        <p className="mt-1 text-sm font-semibold text-[#5E516B]">Issue lifecycle</p>
      </div>
      <ol className="grid gap-2" aria-label="Issue 阶段列表">
        {PHASES.map((phase, index) => {
          const completed = index < activeIndex;
          const active = index === activeIndex;
          return (
            <li
              key={phase.id}
              className={
                active
                  ? "flex items-center gap-2 rounded-lg border-2 border-cyan-300 bg-cyan-50 px-3 py-2 text-sm font-black text-cyan-950"
                  : "flex items-center gap-2 rounded-lg border-2 border-slate-200 bg-slate-50 px-3 py-2 text-sm font-bold text-slate-600"
              }
            >
              {completed ? (
                <CheckCircle2 className="h-4 w-4 text-emerald-600" />
              ) : active ? (
                <CircleDot className="h-4 w-4 text-cyan-700" />
              ) : (
                <Circle className="h-4 w-4 text-slate-400" />
              )}
              <span>{phase.label}</span>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
