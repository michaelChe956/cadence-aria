import { ChevronDown } from "lucide-react";

/**
 * 工作台 issue 详情页折叠抽屉共用的展开/收起控件（owner 直批 2026-10-14：要求
 * 「部分交付」区与「Work Item」区默认收起，且收起后仍要有清晰展开提示）。
 *
 * 单一定义避免两处文案/aria 约定分叉：图标 + 文字按钮（比纯图标更易发现），
 * aria-label 展开态带内容计数，供读屏与测试稳定定位。
 */
export function SectionCollapseToggle({
  expanded,
  expandLabel,
  collapseLabel,
  onToggle,
}: {
  expanded: boolean;
  expandLabel: string;
  collapseLabel: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-expanded={expanded}
      aria-label={expanded ? collapseLabel : expandLabel}
      onClick={onToggle}
      className="inline-flex h-7 shrink-0 cursor-pointer items-center gap-1 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 text-[11px] font-semibold text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] motion-reduce:transition-none"
    >
      <ChevronDown
        className={`h-3.5 w-3.5 shrink-0 transition-transform duration-200 motion-reduce:transition-none${
          expanded ? " rotate-180" : ""
        }`}
      />
      {expanded ? "收起" : "展开"}
    </button>
  );
}
