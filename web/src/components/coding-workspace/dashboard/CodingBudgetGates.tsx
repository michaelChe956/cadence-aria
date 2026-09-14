// web/src/components/coding-workspace/dashboard/CodingBudgetGates.tsx
import { useEffect, useState } from "react";
import { Hourglass } from "lucide-react";
import type { CodingBudgetGate } from "../../../state/coding-dashboard-projection";
import { formatFlowElapsed } from "../../../state/workspace-cockpit-projection";
import { useCockpitSettings } from "../../cockpit/CockpitShell";

export function CodingBudgetGates({ gates, nowMs }: { gates: readonly CodingBudgetGate[]; nowMs: number }) {
  const settings = useCockpitSettings();
  const nearGates = gates.filter((gate) => gate.nearExhaustion && gate.anchorAtMs !== null && !gate.frozen);
  const [announceSeq, setAnnounceSeq] = useState(0);

  useEffect(() => {
    if (nearGates.length === 0) return;
    const timer = window.setInterval(() => setAnnounceSeq((seq) => seq + 1), settings.escalationRepeatMs);
    return () => window.clearInterval(timer);
  }, [nearGates.length, settings.escalationRepeatMs]);

  if (gates.length === 0) {
    return (
      <section data-testid="coding-budget-gates" aria-label="预算门" className="rounded-lg border border-[var(--aria-line)] bg-white p-3">
        <div data-testid="coding-budget-empty" className="text-xs text-[var(--aria-ink-muted)]">
          尚未进入 Coding 阶段，预算门未开始计时
        </div>
      </section>
    );
  }

  return (
    <section data-testid="coding-budget-gates" aria-label="预算门" className="grid gap-2 rounded-lg border border-[var(--aria-line)] bg-white p-3">
      <div className="flex items-center gap-1.5 text-xs font-semibold text-[var(--aria-ink)]">
        <Hourglass className="h-4 w-4" aria-hidden="true" />
        预算门
      </div>
      {gates.map((gate) => (
        <div key={gate.kind} className="grid gap-1">
          <div className="flex items-baseline justify-between gap-2 text-xs">
            <span className="font-semibold text-[var(--aria-ink)]">{gate.label}</span>
            <span
              data-testid={`coding-budget-remaining-${gate.kind}`}
              className="aria-mono aria-num text-[11px] text-[var(--aria-ink-muted)]"
            >
              {gate.remainingMs <= 0 ? "已耗尽" : `剩 ${formatFlowElapsed(gate.remainingMs)}`}
            </span>
          </div>
          <div
            role="progressbar"
            aria-label={gate.label}
            aria-valuenow={Math.round(gate.ratio * 100)}
            aria-valuemin={0}
            aria-valuemax={100}
            className="h-2 overflow-hidden rounded-full bg-[var(--aria-panel-subtle)]"
          >
            <div
              data-testid={`coding-budget-bar-${gate.kind}`}
              className={gate.nearExhaustion ? "aria-pulse h-2 rounded-full transition-[width] duration-200" : "h-2 rounded-full transition-[width] duration-200"}
              style={{
                width: `${Math.round(gate.ratio * 100)}%`,
                background: gate.nearExhaustion ? "var(--aria-warning)" : "var(--aria-primary)",
              }}
            />
          </div>
        </div>
      ))}
      <p data-testid="coding-budget-disclaimer" className="text-[11px] leading-4 text-[var(--aria-ink-muted)]">
        预算口径为 driver 约定（Work Item 60 分钟 / Coding 90 分钟），由前端自计时呈现，非引擎事件数据。
      </p>
      {nearGates.length > 0 ? (
        <div
          data-testid="coding-budget-alert"
          role="alert"
          className="flex min-h-11 items-center gap-2 rounded-md border border-[var(--aria-gate-open-border)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs font-semibold text-[var(--aria-warning)]"
        >
          预算临近耗尽：{nearGates.map((gate) => gate.label).join("、")} 剩余不足 10 分钟（第 {announceSeq + 1} 次提醒，前端自计时）
        </div>
      ) : null}
    </section>
  );
}
