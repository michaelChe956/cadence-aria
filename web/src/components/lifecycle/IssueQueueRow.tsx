// Task 3（展示组件）：workbench Issue 队列的单行组件与阶段 mini-graph。
// 纯展示：消费 Task 1 的 IssueQueueRowData/StagePip，不做任何派生或请求。
// 视觉规范：单行 h-11、标题 truncate、pip h-2 w-2 rounded-full + w-3 连接线；
// focused 用常驻 3px 左条变色（不 focused 时透明）避免布局位移；hover 只改颜色。
import { Fragment } from "react";
import type { JSX } from "react";
import { Sparkles, Trash2 } from "lucide-react";
import type {
  IssueQueueRowData,
  StagePip,
  StagePipState,
} from "./issue-queue-derivation";

// pip 状态色映射（与 Plan 逐字一致）：done=阶段色实心（emerald-500）、
// active=主色、blocked=amber-500、pending=line 令牌。
const PIP_CLASS_BY_STATE: Record<StagePipState, string> = {
  done: "bg-emerald-500",
  active: "bg-[var(--aria-primary)]",
  blocked: "bg-amber-500",
  pending: "bg-[var(--aria-line)]",
};

const STAGE_LABEL: Record<StagePip["stage"], string> = {
  story: "Story",
  design: "Design",
  work_item: "Work Item",
  coding: "编码",
};

const PIP_STATE_LABEL: Record<StagePipState, string> = {
  done: "已完成",
  active: "进行中",
  blocked: "阻塞",
  pending: "待开始",
};

export function StageMiniGraph({ pips }: { pips: StagePip[] }): JSX.Element {
  return (
    <span
      data-testid="stage-mini-graph"
      aria-hidden="true"
      className="flex shrink-0 items-center"
    >
      {pips.map((pip, index) => (
        <Fragment key={pip.stage}>
          {index > 0 ? (
            <span className="h-0.5 w-3 shrink-0 rounded-full bg-[var(--aria-line)]" />
          ) : null}
          <span
            data-testid={`stage-pip-${pip.stage}`}
            data-state={pip.state}
            title={`${STAGE_LABEL[pip.stage]}：${PIP_STATE_LABEL[pip.state]}`}
            className={[
              "h-2 w-2 shrink-0 rounded-full transition-colors duration-200",
              PIP_CLASS_BY_STATE[pip.state],
            ].join(" ")}
          />
        </Fragment>
      ))}
    </span>
  );
}

export function IssueQueueRow(props: {
  row: IssueQueueRowData;
  focused: boolean;
  onSelect: () => void;
  onGenerateStorySpec?: () => void;
  onDelete?: () => void;
  deleting?: boolean;
}): JSX.Element {
  const { row, focused, onSelect, onGenerateStorySpec, onDelete } = props;
  const deleting = props.deleting ?? false;

  return (
    <div
      data-testid="issue-queue-row"
      data-issue-id={row.issueId}
      aria-current={focused ? "true" : undefined}
      aria-busy={deleting}
      className={[
        "group flex h-11 items-center gap-2 border-l-[3px] pr-1 pl-2 transition-colors duration-200",
        focused
          ? "border-l-[var(--aria-primary)] bg-[var(--aria-panel)]"
          : "border-l-transparent hover:bg-[var(--aria-panel-muted)]",
      ].join(" ")}
    >
      <button
        type="button"
        aria-label={`选择 Issue ${row.title}`}
        disabled={deleting}
        onClick={onSelect}
        className="min-w-0 flex-1 cursor-pointer truncate text-left text-sm font-medium text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        <span data-testid="issue-queue-row-title" className="block truncate">
          {row.title}
        </span>
      </button>
      <StageMiniGraph pips={row.stagePips} />
      {onGenerateStorySpec ? (
        <button
          type="button"
          aria-label="生成 Story Spec"
          disabled={deleting}
          onClick={onGenerateStorySpec}
          className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] opacity-0 transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] group-hover:opacity-100 group-focus-within:opacity-100"
        >
          <Sparkles className="h-3.5 w-3.5" />
        </button>
      ) : null}
      {onDelete ? (
        <button
          type="button"
          aria-label={`删除 Issue ${row.title}`}
          disabled={deleting}
          onClick={onDelete}
          className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] opacity-0 transition-colors duration-200 hover:border-[var(--aria-danger)] hover:text-[var(--aria-danger)] focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] group-hover:opacity-100 group-focus-within:opacity-100"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      ) : null}
    </div>
  );
}
