// 逻辑代码库目录树示意（REQ-LAYOUT-02 的**结构**部分；块3 自有的唯一新增内容）。
//
// 纯结构渲染：这里**只有目录形态与索引范围**，不含任何失败文案 / 动作文案
// （问题 / 原因 / 怎么做的中文成品一律由后端 `copy` 下发，块3 不自建第二份，
// 见 REQ-LAYOUT-04 / REQ-LAYOUT-05）。
export function LogicalCodebaseLayoutTree({
  className = "",
}: {
  className?: string;
}) {
  return (
    <div
      data-testid="lc-layout-tree"
      className={`rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1.5 ${className}`}
    >
      <p className="text-xs font-semibold text-[var(--aria-ink)]">目录结构</p>
      <div
        data-testid="lc-layout-tree-diagram"
        className="mt-1 font-mono text-[11px] leading-snug text-[var(--aria-ink)]"
      >
        <div data-testid="lc-layout-tree-root">
          my-lc-root/{" "}
          <span className="text-[var(--aria-ink-muted)]">
            ─ 逻辑代码库根（普通空目录，不是 git 仓）
          </span>
        </div>
        <div data-testid="lc-layout-tree-member-backend">
          ├─ backend/{" "}
          <span className="text-[var(--aria-ink-muted)]">
            ─ 成员仓（直接子目录）
          </span>
        </div>
        <div data-testid="lc-layout-tree-member-frontend">
          └─ frontend/{" "}
          <span className="text-[var(--aria-ink-muted)]">
            ─ 成员仓（直接子目录）
          </span>
        </div>
      </div>
      <p
        data-testid="lc-layout-tree-note"
        className="mt-1 text-xs text-[var(--aria-ink-muted)]"
      >
        成员仓代码要放在仓库根或普通子目录；只在 .worktrees/ 下的代码不可被索引。
      </p>
    </div>
  );
}
