// Task 5（展示组件）：阶段步进器 -- tablist 语义的阶段切换条（Story / Design / Work Item）。
// 纯展示：stages 数据与 activeStage 由父层派生，本组件不做任何派生或请求。
// 视觉规范：每段 = 胶囊计数 chip + pip 状态色点 + 段间连接线；
// hover 仅改颜色/边框（不位移）；过渡 200ms + motion-reduce 降级；
// cursor-pointer；focus-visible 主色 ring。供 Task 6 阶段标签工作区消费。
import { Fragment } from "react";
import type { JSX } from "react";
import type { StagePipState } from "./issue-queue-derivation";

export type WorkbenchStageKey = "story" | "design" | "work_item";

export interface StageStepperStage {
  key: WorkbenchStageKey;
  label: string;
  count: number;
  state: StagePipState;
}

// pip 状态色映射（与 IssueQueueRow 的 StageMiniGraph 逐字一致）：
// done=阶段色实心（emerald-500）、active=主色、blocked=amber-500、pending=line 令牌。
const PIP_CLASS_BY_STATE: Record<StagePipState, string> = {
  done: "bg-emerald-500",
  active: "bg-[var(--aria-primary)]",
  blocked: "bg-amber-500",
  pending: "bg-[var(--aria-line)]",
};

const PIP_STATE_LABEL: Record<StagePipState, string> = {
  done: "已完成",
  active: "进行中",
  blocked: "阻塞",
  pending: "待开始",
};

export function StageStepper(props: {
  stages: StageStepperStage[];
  activeStage: WorkbenchStageKey;
  onSelect: (stage: WorkbenchStageKey) => void;
}): JSX.Element {
  const { stages, activeStage, onSelect } = props;

  return (
    <div
      data-testid="stage-stepper"
      role="tablist"
      aria-label="生命周期阶段"
      className="flex min-w-0 items-center gap-2"
    >
      {stages.map((stage, index) => {
        const selected = stage.key === activeStage;
        return (
          <Fragment key={stage.key}>
            {index > 0 ? (
              <span
                data-testid="stage-stepper-connector"
                aria-hidden="true"
                className="h-0.5 w-4 shrink-0 rounded-full bg-[var(--aria-line)]"
              />
            ) : null}
            <button
              type="button"
              role="tab"
              data-testid={`stage-tab-${stage.key}`}
              aria-selected={selected}
              aria-controls={`stage-panel-${stage.key}`}
              onClick={() => onSelect(stage.key)}
              className={[
                "inline-flex h-8 shrink-0 cursor-pointer items-center gap-1.5 rounded-full border px-3 text-sm font-medium",
                "transition-colors duration-200 motion-reduce:transition-none",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
                selected
                  ? "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                  : "border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)]",
              ].join(" ")}
            >
              <span
                data-testid={`stage-tab-pip-${stage.key}`}
                data-state={stage.state}
                aria-hidden="true"
                title={`${stage.label}：${PIP_STATE_LABEL[stage.state]}`}
                className={[
                  "h-2 w-2 shrink-0 rounded-full transition-colors duration-200",
                  PIP_CLASS_BY_STATE[stage.state],
                ].join(" ")}
              />
              <span className="truncate">{stage.label}</span>
              <span
                data-testid={`stage-tab-count-${stage.key}`}
                className={[
                  "inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full px-1 text-xs font-semibold leading-none",
                  selected
                    ? "bg-[var(--aria-primary)] text-[var(--aria-panel)]"
                    : "bg-[var(--aria-panel-subtle)] text-[var(--aria-ink-muted)]",
                ].join(" ")}
              >
                {stage.count}
              </span>
            </button>
          </Fragment>
        );
      })}
    </div>
  );
}
